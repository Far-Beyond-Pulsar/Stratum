use crate::types::{Aabb, ChunkId, ChunkState};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

/// Metadata describing a chunk's spatial properties and configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkMetadata {
    /// Unique identifier for this chunk.
    pub id: ChunkId,
    /// Axis-aligned bounding box in world space.
    pub aabb: Aabb,
    /// Center position in world space (cached for quick access).
    pub center: Vec3,
    /// Level of detail (0 = highest detail).
    pub lod: u8,
    /// Grid coordinates in the world partition.
    pub grid_coords: (i32, i32, i32),
}

impl ChunkMetadata {
    /// Create new chunk metadata from grid coordinates and chunk size.
    pub fn new(grid_x: i32, grid_y: i32, grid_z: i32, lod: u8, chunk_size: f32) -> Self {
        let id = ChunkId::from_coords(grid_x, grid_y, grid_z, lod);

        let grid_pos = Vec3::new(grid_x as f32, grid_y as f32, grid_z as f32);
        let half_size = chunk_size * 0.5;
        let center = grid_pos * chunk_size;

        let aabb = Aabb::from_center_extents(center, Vec3::splat(half_size));

        Self {
            id,
            aabb,
            center,
            lod,
            grid_coords: (grid_x, grid_y, grid_z),
        }
    }

    /// Calculate distance from a point to this chunk's center.
    pub fn distance_to(&self, point: Vec3) -> f32 {
        self.center.distance(point)
    }

    /// Calculate screen-space size metric for LOD selection.
    /// Returns a normalized value based on distance and chunk extent.
    pub fn screen_size_metric(&self, view_position: Vec3, view_distance_scale: f32) -> f32 {
        let distance = self.distance_to(view_position).max(1.0);
        let extent = self.aabb.max_extent();
        (extent * view_distance_scale) / distance
    }
}

/// Represents a chunk's runtime state and data.
pub struct Chunk {
    /// Metadata describing the chunk's spatial properties.
    pub metadata: ChunkMetadata,
    /// Current loading/visibility state.
    pub state: ChunkState,
    /// Priority for loading (higher = more important).
    pub priority: f32,
    /// Frame number when this chunk was last evaluated.
    pub last_eval_frame: u64,
    /// Timestamp when loading started (for timeout detection).
    pub load_start_time: Option<Instant>,
    /// Path to the chunk file on disk.
    pub file_path: Option<PathBuf>,
    /// Compressed chunk data (cached in memory after loading).
    pub cached_data: Option<Arc<Vec<u8>>>,
    /// Size of the chunk data in bytes (for memory tracking).
    pub data_size_bytes: usize,
}

impl Chunk {
    /// Create a new chunk in the Unloaded state.
    pub fn new(metadata: ChunkMetadata) -> Self {
        Self {
            metadata,
            state: ChunkState::Unloaded,
            priority: 0.0,
            last_eval_frame: 0,
            load_start_time: None,
            file_path: None,
            cached_data: None,
            data_size_bytes: 0,
        }
    }

    /// Check if the chunk is currently loaded in memory (any loaded state).
    pub fn is_loaded(&self) -> bool {
        matches!(
            self.state,
            ChunkState::Loaded | ChunkState::Visible | ChunkState::Hidden
        )
    }

    /// Check if the chunk is visible and should be rendered.
    pub fn is_visible(&self) -> bool {
        self.state == ChunkState::Visible
    }

    /// Check if the chunk is currently being loaded.
    pub fn is_loading(&self) -> bool {
        self.state == ChunkState::Loading
    }

    /// Mark chunk as loaded without actually loading from disk (for demos/testing).
    pub fn mark_loaded(&mut self) {
        if matches!(self.state, ChunkState::Unloaded | ChunkState::Loading) {
            self.state = ChunkState::Loaded;
            self.load_start_time = None;
        }
    }

    /// Check if the chunk is unloaded or being evicted.
    pub fn is_unloaded(&self) -> bool {
        matches!(self.state, ChunkState::Unloaded | ChunkState::Evicting)
    }

    /// Transition to Loading state.
    pub fn begin_loading(&mut self) {
        self.state = ChunkState::Loading;
        self.load_start_time = Some(Instant::now());
    }

    /// Transition to Loaded state after successful load.
    pub fn finish_loading(&mut self, data: Arc<Vec<u8>>) {
        self.state = ChunkState::Loaded;
        self.data_size_bytes = data.len();
        self.cached_data = Some(data);
        self.load_start_time = None;
    }

    /// Transition to Visible state.
    pub fn set_visible(&mut self, visible: bool) {
        if visible && self.is_loaded() {
            self.state = ChunkState::Visible;
        } else if !visible && self.state == ChunkState::Visible {
            self.state = ChunkState::Loaded;
        }
    }

    /// Begin eviction process.
    pub fn begin_evicting(&mut self) {
        self.state = ChunkState::Evicting;
    }

    /// Complete eviction and return to Unloaded state.
    pub fn finish_evicting(&mut self) {
        self.state = ChunkState::Unloaded;
        self.cached_data = None;
        self.data_size_bytes = 0;
        self.load_start_time = None;
    }

    /// Get loading duration if currently loading.
    pub fn loading_duration(&self) -> Option<std::time::Duration> {
        self.load_start_time.map(|start| start.elapsed())
    }

    /// Update priority based on distance and other factors.
    pub fn update_priority(&mut self, view_position: Vec3, distance_weight: f32) {
        let distance = self.metadata.distance_to(view_position);
        // Priority is inverse of distance with weighting
        self.priority = distance_weight / (distance + 1.0);
    }
}

/// Serializable chunk data format for disk storage.
#[derive(Debug, Serialize, Deserialize)]
pub struct ChunkData {
    /// Chunk metadata.
    pub metadata: ChunkMetadata,
    /// Version number for format compatibility.
    pub version: u32,
    /// Serialized object data (positions, meshes, materials, etc.).
    pub object_data: Vec<u8>,
    /// Optional compressed mesh data.
    pub mesh_data: Option<Vec<u8>>,
    /// Optional compressed texture data.
    pub texture_data: Option<Vec<u8>>,
}

impl ChunkData {
    /// Current version of the chunk data format.
    pub const VERSION: u32 = 1;

    /// Create a new ChunkData with the current version.
    pub fn new(metadata: ChunkMetadata) -> Self {
        Self {
            metadata,
            version: Self::VERSION,
            object_data: Vec::new(),
            mesh_data: None,
            texture_data: None,
        }
    }

    /// Serialize to bytes with optional compression.
    pub fn serialize(&self, compress: bool) -> Result<Vec<u8>, ChunkError> {
        let json_bytes = serde_json::to_vec(self)
            .map_err(|e| ChunkError::SerializationFailed(e.to_string()))?;

        if compress {
            let compressed = lz4::block::compress(&json_bytes, None, false)
                .map_err(|e| ChunkError::CompressionFailed(e.to_string()))?;
            Ok(compressed)
        } else {
            Ok(json_bytes)
        }
    }

    /// Deserialize from bytes with automatic compression detection.
    pub fn deserialize(bytes: &[u8], compressed: bool) -> Result<Self, ChunkError> {
        let json_bytes = if compressed {
            lz4::block::decompress(bytes, None)
                .map_err(|e| ChunkError::DecompressionFailed(e.to_string()))?
        } else {
            bytes.to_vec()
        };

        serde_json::from_slice(&json_bytes)
            .map_err(|e| ChunkError::DeserializationFailed(e.to_string()))
    }
}

/// Errors that can occur during chunk operations.
#[derive(Debug, thiserror::Error)]
pub enum ChunkError {
    #[error("Serialization failed: {0}")]
    SerializationFailed(String),

    #[error("Deserialization failed: {0}")]
    DeserializationFailed(String),

    #[error("Compression failed: {0}")]
    CompressionFailed(String),

    #[error("Decompression failed: {0}")]
    DecompressionFailed(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Invalid chunk state transition: {0}")]
    InvalidStateTransition(String),

    #[error("Chunk not found: {0}")]
    ChunkNotFound(ChunkId),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_metadata_creation() {
        let metadata = ChunkMetadata::new(0, 0, 0, 0, 100.0);
        assert_eq!(metadata.grid_coords, (0, 0, 0));
        assert_eq!(metadata.lod, 0);
        assert_eq!(metadata.center, Vec3::ZERO);
    }

    #[test]
    fn test_chunk_state_transitions() {
        let metadata = ChunkMetadata::new(0, 0, 0, 0, 100.0);
        let mut chunk = Chunk::new(metadata);

        assert!(chunk.is_unloaded());

        chunk.begin_loading();
        assert!(chunk.is_loading());

        let data = Arc::new(vec![1, 2, 3, 4]);
        chunk.finish_loading(data);
        assert!(chunk.is_loaded());
        assert!(!chunk.is_visible());

        chunk.set_visible(true);
        assert!(chunk.is_visible());

        chunk.set_visible(false);
        assert!(chunk.is_loaded());
        assert!(!chunk.is_visible());
    }

    #[test]
    fn test_chunk_data_serialization() {
        let metadata = ChunkMetadata::new(5, 10, 15, 2, 50.0);
        let chunk_data = ChunkData::new(metadata);

        let serialized = chunk_data.serialize(false).unwrap();
        let deserialized = ChunkData::deserialize(&serialized, false).unwrap();

        assert_eq!(deserialized.version, ChunkData::VERSION);
        assert_eq!(deserialized.metadata.grid_coords, (5, 10, 15));
        assert_eq!(deserialized.metadata.lod, 2);
    }

    #[test]
    fn test_chunk_data_compression() {
        let metadata = ChunkMetadata::new(0, 0, 0, 0, 100.0);
        let mut chunk_data = ChunkData::new(metadata);
        chunk_data.object_data = vec![0u8; 10000]; // Large data for compression test

        let compressed = chunk_data.serialize(true).unwrap();
        let decompressed = ChunkData::deserialize(&compressed, true).unwrap();

        assert_eq!(decompressed.object_data.len(), 10000);
        assert!(compressed.len() < 10000); // Should be smaller when compressed
    }
}
