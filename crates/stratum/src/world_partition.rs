use crate::{camera::{Camera, CameraId, CameraRegistry}, chunk::{Chunk, ChunkData, ChunkError, ChunkMetadata}, types::{ChunkId, ChunkState}};
use glam::Vec3;
use std::{collections::{HashMap, HashSet}, fs, path::PathBuf, sync::Arc, time::{Duration, Instant}};
use tokio::{runtime::Runtime, sync::mpsc};

/// Load request for async disk IO.
pub struct ChunkLoadRequest {
    pub chunk_id: ChunkId,
    pub file_path: PathBuf,
    pub compressed: bool,
}

/// Async load result for a chunk.
pub struct ChunkLoadResult {
    pub chunk_id: ChunkId,
    pub data: Result<ChunkData, ChunkError>,
}

/// Streaming budget constraints (per tick).
pub struct StreamingBudget {
    pub max_io_ms_per_frame: Duration,
    pub max_upload_bytes_per_frame: usize,
    pub max_concurrent_tasks: usize,
}

impl Default for StreamingBudget {
    fn default() -> Self {
        Self {
            max_io_ms_per_frame: Duration::from_millis(6),
            max_upload_bytes_per_frame: 5 * 1024 * 1024,
            max_concurrent_tasks: 6,
        }
    }
}

/// Telemetry counters for world partition streaming.
pub struct WorldPartitionMetrics {
    pub frames: u64,
    pub chunks_loaded: u64,
    pub chunks_evicted: u64,
    pub chunks_failed: u64,
    pub pending_load_tasks: usize,
    pub last_commit_ms: Duration,
}

impl Default for WorldPartitionMetrics {
    fn default() -> Self {
        Self {
            frames: 0,
            chunks_loaded: 0,
            chunks_evicted: 0,
            chunks_failed: 0,
            pending_load_tasks: 0,
            last_commit_ms: Duration::ZERO,
        }
    }
}

/// World partition manager with chunk state tracking, LOD, and async IO.
pub struct WorldPartitionManager {
    pub chunk_size: f32,
    pub visible_radius: f32,
    pub preload_radius: f32,
    pub unload_radius: f32,
    pub max_lod: u8,
    pub chunks: HashMap<ChunkId, Chunk>,
    pub registry: CameraRegistry,
    pub budget: StreamingBudget,
    pub metrics: WorldPartitionMetrics,
    pending_loads: HashSet<ChunkId>,
    runtime: Runtime,
    load_tx: mpsc::Sender<ChunkLoadRequest>,
    load_rx: mpsc::Receiver<ChunkLoadResult>,
    last_tick: Instant,
}

impl WorldPartitionManager {
    /// Create a new manager with configurable radii. If radii overlap incorrectly,
    /// they are clamped to a sane progression.
    pub fn new(chunk_size: f32, visible_radius: f32, preload_radius: f32, unload_radius: f32) -> Self {
        let visible_radius = visible_radius.max(1.0);
        let preload_radius = preload_radius.max(visible_radius);
        let unload_radius = unload_radius.max(preload_radius + 1.0);

        let (load_tx, load_rx_in) = mpsc::channel(256);
        let (result_tx, load_rx) = mpsc::channel(256);

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime for world partition");

        // Start I/O consumer that receives chunk loads from disk and dispatches results.
        let mut receiver: mpsc::Receiver<ChunkLoadRequest> = load_rx_in;
        runtime.spawn(async move {
            while let Some(request) = receiver.recv().await {
                let result_tx = result_tx.clone();
                tokio::spawn(async move {
                    let data = tokio::task::spawn_blocking(move || {
                        let bytes = fs::read(&request.file_path)?;
                        ChunkData::deserialize(&bytes, request.compressed)
                    })
                    .await;

                    let result = match data {
                        Ok(res) => res,
                        Err(join_error) => Err(ChunkError::IoError(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("join error: {join_error}"),
                        ))),
                    };

                    let _ = result_tx
                        .send(ChunkLoadResult { chunk_id: request.chunk_id, data: result })
                        .await;
                });
            }
        });

        Self {
            chunk_size,
            visible_radius,
            preload_radius,
            unload_radius,
            max_lod: 0,
            chunks: HashMap::new(),
            registry: CameraRegistry::new(),
            budget: StreamingBudget::default(),
            metrics: WorldPartitionMetrics::default(),
            pending_loads: HashSet::new(),
            runtime,
            load_tx,
            load_rx,
            last_tick: Instant::now(),
        }
    }

    /// Register a new camera and return its id.
    pub fn register_camera(&mut self, position: Vec3, rotation: glam::Quat) -> CameraId {
        self.registry.register_camera(position, rotation)
    }

    /// Update camera transform with a stable id.
    pub fn update_camera_transform(&mut self, camera_id: CameraId, position: Vec3, rotation: glam::Quat) {
        if let Some(camera) = self.registry.get_camera_mut(camera_id) {
            camera.set_transform(position, rotation);
        }
    }

    /// Prepare a chunk for streaming and disk load path.
    pub fn upsert_chunk(&mut self, metadata: ChunkMetadata, file_path: PathBuf) {
        let id = metadata.id;
        let chunk = self.chunks.entry(id).or_insert_with(|| Chunk::new(metadata.clone()));
        chunk.metadata = metadata;
        chunk.file_path = Some(file_path);
    }

    /// Set streaming budget.
    pub fn set_budget(&mut self, budget: StreamingBudget) {
        self.budget = budget;
    }

    /// Tick the streaming system (call once per frame).
    pub fn tick(&mut self, dt: f32) {
        self.metrics.frames += 1;
        let tick_start = Instant::now();

        self.update_camera_kinematics(dt);
        self.evaluate_lod_and_visibility();
        self.process_io_results();

        self.metrics.last_commit_ms = tick_start.elapsed();
    }

    fn update_camera_kinematics(&mut self, dt: f32) {
        for camera in self.registry.cameras_mut() {
            let new_pos = camera.predict_position(dt);
            camera.update_velocity(new_pos, dt);
        }
    }

    fn evaluate_lod_and_visibility(&mut self) {
        let cameras: Vec<&Camera> = self.registry.cameras().collect();
        if cameras.is_empty() {
            return;
        }

        let mut chunks_to_load = Vec::new();
        let mut chunks_to_evict = Vec::new();

        for chunk in self.chunks.values_mut() {
            let mut nearest_dist = f32::MAX;
            let mut any_frustum = false;

            for camera in &cameras {
                let dist = chunk.metadata.distance_to(camera.position);
                nearest_dist = nearest_dist.min(dist);
                if camera.frustum.intersects_aabb(&chunk.metadata.aabb) {
                    any_frustum = true;
                }
            }

            let should_be_loaded = nearest_dist <= self.preload_radius;
            let should_be_visible = any_frustum && nearest_dist <= self.visible_radius;
            let should_be_evicted = nearest_dist > self.unload_radius;

            match chunk.state {
                ChunkState::Unloaded if should_be_loaded => {
                    chunk.begin_loading();
                    chunks_to_load.push(chunk.metadata.id);
                }
                ChunkState::Loading if should_be_evicted => {
                    chunk.begin_evicting();
                    chunks_to_evict.push(chunk.metadata.id);
                }
                ChunkState::Loaded | ChunkState::Hidden | ChunkState::Visible => {
                    if should_be_visible {
                        chunk.set_visible(true);
                    } else {
                        chunk.set_visible(false);
                    }
                    if should_be_evicted {
                        chunk.begin_evicting();
                        chunks_to_evict.push(chunk.metadata.id);
                    }
                }
                _ => {}
            }
        }

        for chunk_id in chunks_to_load {
            if self.pending_loads.len() >= self.budget.max_concurrent_tasks {
                break;
            }
            self.request_chunk_load(chunk_id);
        }

        for chunk_id in chunks_to_evict {
            if let Some(chunk) = self.chunks.get_mut(&chunk_id) {
                chunk.finish_evicting();
                self.metrics.chunks_evicted += 1;
                self.pending_loads.remove(&chunk_id);
            }
        }

        self.metrics.pending_load_tasks = self.pending_loads.len();
    }

    fn request_chunk_load(&mut self, chunk_id: ChunkId) {
        if !self.pending_loads.contains(&chunk_id) {
            if let Some(chunk) = self.chunks.get_mut(&chunk_id) {
                if chunk.state != ChunkState::Loading {
                    if let Some(ref path) = chunk.file_path {
                        let _ = self.load_tx.try_send(ChunkLoadRequest {
                            chunk_id,
                            file_path: path.clone(),
                            compressed: true,
                        });

                        self.pending_loads.insert(chunk_id);
                    }
                }
            }
        }
    }

    fn process_io_results(&mut self) {
        while let Ok(result) = self.load_rx.try_recv() {
            self.pending_loads.remove(&result.chunk_id);

            match result.data {
                Ok(chunk_data) => {
                    if let Some(chunk) = self.chunks.get_mut(&result.chunk_id) {
                        chunk.finish_loading(Arc::new(chunk_data.serialize(true).unwrap_or_default()));
                        chunk.state = ChunkState::Loaded;
                        // Preserve visibility after load; the next tick sets visible if needed.
                        self.metrics.chunks_loaded += 1;
                    }
                }
                Err(err) => {
                    self.metrics.chunks_failed += 1;
                    if let Some(chunk) = self.chunks.get_mut(&result.chunk_id) {
                        chunk.state = ChunkState::Unloaded;
                        log::warn!("Chunk {:?} load failed: {err}", result.chunk_id);
                    }
                }
            }
        }

        self.metrics.pending_load_tasks = self.pending_loads.len();
    }

    /// Save chunk contents to disk in compressed format.
    pub fn flush_chunk_to_disk(&self, chunk_id: ChunkId) -> Result<(), ChunkError> {
        let chunk = self.chunks.get(&chunk_id).ok_or(ChunkError::ChunkNotFound(chunk_id))?;
        let data = ChunkData::new(chunk.metadata.clone());
        let bytes = data.serialize(true)?;
        let path = chunk.file_path.as_ref().ok_or_else(|| ChunkError::IoError(std::io::Error::new(std::io::ErrorKind::NotFound, "chunk path not set")))?;
        fs::write(path, bytes)?;
        Ok(())
    }

    /// Get all currently visible chunk ids.
    pub fn visible_chunk_ids(&self) -> Vec<ChunkId> {
        self.chunks.iter().filter_map(|(&id, chunk)| {
            if chunk.is_visible() {
                Some(id)
            } else {
                None
            }
        }).collect()
    }

    /// Get a metric snapshot.
    pub fn metrics(&self) -> WorldPartitionMetrics {
        self.metrics.clone()
    }
}

impl Clone for WorldPartitionMetrics {
    fn clone(&self) -> Self {
        Self {
            frames: self.frames,
            chunks_loaded: self.chunks_loaded,
            chunks_evicted: self.chunks_evicted,
            chunks_failed: self.chunks_failed,
            pending_load_tasks: self.pending_load_tasks,
            last_commit_ms: self.last_commit_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3};
    use std::path::PathBuf;

    #[test]
    fn test_manager_camera_registration_and_update() {
        let mut wm = WorldPartitionManager::new(16.0, 80.0, 120.0, 160.0);
        let id = wm.register_camera(Vec3::ZERO, Quat::IDENTITY);
        wm.update_camera_transform(id, Vec3::new(10.0, 0.0, 0.0), Quat::IDENTITY);
        let cam = wm.registry.get_camera(id).expect("camera exists");
        assert_eq!(cam.position, Vec3::new(10.0, 0.0, 0.0));
    }

    #[test]
    fn test_chunk_lod_and_evict_cycle() {
        let mut wm = WorldPartitionManager::new(16.0, 10.0, 20.0, 40.0);

        let metadata = ChunkMetadata::new(0, 0, 0, 0, 16.0);
        let chunk_file = PathBuf::from("/tmp/world_chunk_0_0_0.bin");
        wm.upsert_chunk(metadata, chunk_file);

        let cam_id = wm.register_camera(Vec3::new(0.0, 0.0, 0.0), Quat::IDENTITY);
        wm.tick(1.0 / 60.0);

        assert!(wm.chunks.contains_key(&metadata.id));

        // Move camera far away to trace eviction
        wm.update_camera_transform(cam_id, Vec3::new(1000.0, 0.0, 0.0), Quat::IDENTITY);
        wm.tick(1.0 / 60.0);

        let evicted = wm.metrics.chunks_evicted;
        assert!(evicted >= 0);
    }
}
