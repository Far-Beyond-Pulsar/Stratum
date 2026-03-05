//! `voxel_world` — Infinite procedurally-generated Minecraft-style voxel world.
//!
//! Chunks are generated lazily as the camera moves.  Each Stratum world-
//! partition chunk maps 1:1 to one on-disk file under `levels/voxel_world/`.
//!
//! ## Mesh strategy
//! A single `GpuMesh` is built per material (grass / dirt / stone) per chunk.
//! Only exposed faces are emitted — interior faces between two solid blocks are
//! culled. This keeps draw-call count at ≤ 3 per loaded chunk and vertex count
//! proportional to surface area rather than volume.
//!
//! ## Controls
//! WASD fly | Space/Shift up/down | Mouse drag look (click to grab) | Tab mode | Esc exit

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use glam::Vec3;
use winit::{
    application::ApplicationHandler,
    event::*,
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowId},
};

use helio_render_v2::{
    GpuMesh, PackedVertex, Renderer, RendererConfig,
    features::{BloomFeature, FeatureRegistry, LightingFeature, ShadowsFeature},
};

use stratum::{
    chunk_on_disk,
    CameraId, CameraKind, ChunkCoord, Components, EntityId, LightData,
    MaterialHandle, Projection, RenderTargetHandle,
    SimulationMode, Stratum, StratumCamera, Transform, Viewport,
    Level, StreamEvent, LevelStreamer, MeshHandle,
    level_fs::format::{ChunkFile, EntityRecord, TransformRecord, FORMAT_VERSION},
};
use stratum_helio::{AssetRegistry, HelioIntegration, Material};

// ── Level directory ───────────────────────────────────────────────────────────

fn level_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("levels")
        .join("voxel_world")
}

/// Bump this key to wipe stale on-disk chunks after format changes.
const CACHE_KEY: &str = "voxel_world_v4_chunk16";

// ── World constants ───────────────────────────────────────────────────────────

/// World-space metres per chunk edge.  1 voxel = 1 m so this equals VOXELS_PER_CHUNK.
const CHUNK_SIZE: f32 = 16.0;
const VOXELS_PER_CHUNK: i32 = 16;
const ACTIVATION_RADIUS: f32 = CHUNK_SIZE * 8.0;
/// Half-side of the square load area in chunk coords.
const LOAD_RADIUS: i32 = 5;
const BASE_HEIGHT: i32 = 2;
const HEIGHT_RANGE: i32 = 10; // max surface = 12, fits in Y=0 chunk (0..VOXELS_PER_CHUNK)

const CAM_SPEED: f32 = 16.0;
const LOOK_SENS: f32 = 0.002;

// ── Block type ────────────────────────────────────────────────────────────────

/// Block discriminant stored in chunk JSON as `material` field index.
///   0 = air (not stored), 1 = grass, 2 = dirt, 3 = stone
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Block { Grass, Dirt, Stone }

impl Block {
    fn mat_index(self) -> u64 { match self { Block::Grass => 1, Block::Dirt => 2, Block::Stone => 3 } }
    fn from_mat_index(n: u64) -> Option<Self> {
        match n { 1 => Some(Block::Grass), 2 => Some(Block::Dirt), 3 => Some(Block::Stone), _ => None }
    }
}

// ── Heightmap (no external dep) ───────────────────────────────────────────────

fn hash(x: i32, z: i32, seed: u64) -> f32 {
    let mut v = (x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
              ^ (z as u64).wrapping_mul(0x6c62_272e_07bb_0142)
              ^ seed;
    v ^= v >> 30; v = v.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    v ^= v >> 27; v = v.wrapping_mul(0x94d0_49bb_1331_11eb);
    v ^= v >> 31;
    (v as f32) / (u64::MAX as f32)
}

fn smooth_noise(x: f32, z: f32, seed: u64) -> f32 {
    let xi = x.floor() as i32; let zi = z.floor() as i32;
    let ux = { let f = x - xi as f32; f * f * (3.0 - 2.0 * f) };
    let uz = { let f = z - zi as f32; f * f * (3.0 - 2.0 * f) };
    let a = hash(xi, zi, seed);     let b = hash(xi+1, zi,   seed);
    let c = hash(xi, zi+1, seed);   let d = hash(xi+1, zi+1, seed);
    (a + ux*(b-a)) + uz * ((c + ux*(d-c)) - (a + ux*(b-a)))
}

fn terrain_height(wx: i32, wz: i32) -> i32 {
    let (x, z) = (wx as f32, wz as f32);
    let n = smooth_noise(x/20.0, z/20.0, 0)
          + smooth_noise(x/10.0, z/10.0, 1) * 0.5
          + smooth_noise(x/5.0,  z/5.0,  2) * 0.25;
    let n = (n / 1.75).clamp(0.0, 1.0);
    BASE_HEIGHT + (n * HEIGHT_RANGE as f32) as i32
}

fn block_type(wx: i32, wy: i32, wz: i32) -> Block {
    let h = terrain_height(wx, wz);
    if wy == h          { Block::Grass }
    else if wy >= h - 2 { Block::Dirt  }
    else                { Block::Stone }
}

// ── Chunk data generation ─────────────────────────────────────────────────────
//
// The ChunkFile stores one EntityRecord per voxel. Each record contains:
//   - transform.position  → world-space voxel centre
//   - material            → Block discriminant (1=grass, 2=dirt, 3=stone)
// Mesh handles are NOT stored — they are built at load time from geometry.

fn build_chunk_file(coord: ChunkCoord) -> ChunkFile {
    if coord.y != 0 {
        return ChunkFile { version: FORMAT_VERSION, coord: [coord.x, coord.y, coord.z], entities: vec![] };
    }

    let ox = coord.x * VOXELS_PER_CHUNK;
    let oz = coord.z * VOXELS_PER_CHUNK;
    let mut entities = Vec::new();
    let mut eid: u64 = 0;

    for lx in 0..VOXELS_PER_CHUNK {
        for lz in 0..VOXELS_PER_CHUNK {
            let wx = ox + lx;
            let wz = oz + lz;
            let h  = terrain_height(wx, wz).min(VOXELS_PER_CHUNK - 1);
            for wy in 0..=h {
                let block = block_type(wx, wy, wz);
                eid += 1;
                entities.push(EntityRecord {
                    id: eid,
                    transform: Some(TransformRecord {
                        position: [wx as f32 + 0.5, wy as f32 + 0.5, wz as f32 + 0.5],
                        rotation: [0.0, 0.0, 0.0, 1.0],
                        scale:    [1.0, 1.0, 1.0],
                    }),
                    mesh:            None,
                    material:        Some(block.mat_index()),
                    light:           None,
                    billboard:       None,
                    bounding_radius: 0.866,
                    tags:            vec![],
                });
            }
        }
    }

    ChunkFile { version: FORMAT_VERSION, coord: [coord.x, coord.y, coord.z], entities }
}

// ── Chunk mesh builder ────────────────────────────────────────────────────────
//
// Given a voxel set (world-space integer positions), build face-culled geometry.
// Only faces with NO solid neighbour in that direction are emitted.
// Returns one Vec<(vertices, indices)> per distinct Block type.

fn build_chunk_mesh(
    device:    &wgpu::Device,
    chunk:     ChunkFile,
    mat_grass: MaterialHandle,
    mat_dirt:  MaterialHandle,
    mat_stone: MaterialHandle,
) -> Vec<(GpuMesh, MaterialHandle)> {
    // Collect solid voxels keyed by their integer min-corner (floor of centre).
    let mut solid: HashMap<(i32,i32,i32), Block> = HashMap::new();
    for rec in &chunk.entities {
        if let (Some(t), Some(m)) = (&rec.transform, rec.material) {
            if let Some(block) = Block::from_mat_index(m) {
                solid.insert((
                    t.position[0].floor() as i32,
                    t.position[1].floor() as i32,
                    t.position[2].floor() as i32,
                ), block);
            }
        }
    }
    if solid.is_empty() { return vec![]; }

    // Face table derived directly from GpuMesh::cube (same winding / UVs).
    // Each entry: (normal, neighbour-delta, [4 corner offsets from voxel min])
    // Winding: CCW when viewed from outside (matches the renderer's convention).
    #[rustfmt::skip]
    const FACES: &[([f32;3], [i32;3], [[f32;3];4])] = &[
        // +Z
        ([0.,0., 1.], [0,0, 1], [[0.,0.,1.],[1.,0.,1.],[1.,1.,1.],[0.,1.,1.]]),
        // -Z
        ([0.,0.,-1.], [0,0,-1], [[1.,0.,0.],[0.,0.,0.],[0.,1.,0.],[1.,1.,0.]]),
        // +X
        ([ 1.,0.,0.], [ 1,0,0], [[1.,0.,1.],[1.,0.,0.],[1.,1.,0.],[1.,1.,1.]]),
        // -X
        ([-1.,0.,0.], [-1,0,0], [[0.,0.,0.],[0.,0.,1.],[0.,1.,1.],[0.,1.,0.]]),
        // +Y
        ([0., 1.,0.], [0, 1,0], [[0.,1.,1.],[1.,1.,1.],[1.,1.,0.],[0.,1.,0.]]),
        // -Y
        ([0.,-1.,0.], [0,-1,0], [[0.,0.,0.],[1.,0.,0.],[1.,0.,1.],[0.,0.,1.]]),
    ];
    const UVS: [[f32;2]; 4] = [[0.,0.],[1.,0.],[1.,1.],[0.,1.]];

    let mut verts: HashMap<Block, Vec<PackedVertex>> = HashMap::new();
    let mut idxs:  HashMap<Block, Vec<u32>>          = HashMap::new();

    for (&(bx,by,bz), &block) in &solid {
        let v  = verts.entry(block).or_default();
        let ix = idxs.entry(block).or_default();

        for (normal, nb, corners) in FACES {
            if solid.contains_key(&(bx+nb[0], by+nb[1], bz+nb[2])) { continue; }

            let base_vert = v.len() as u32;
            // Tangent = direction from corner[0] to corner[1] (matches GpuMesh::cube).
            let c0 = corners[0]; let c1 = corners[1];
            let td = [c1[0]-c0[0], c1[1]-c0[1], c1[2]-c0[2]];
            let tl = (td[0]*td[0] + td[1]*td[1] + td[2]*td[2]).sqrt().max(1e-8);
            let tangent = [td[0]/tl, td[1]/tl, td[2]/tl];

            for (ci, corner) in corners.iter().enumerate() {
                v.push(PackedVertex::new_with_tangent(
                    [bx as f32 + corner[0], by as f32 + corner[1], bz as f32 + corner[2]],
                    *normal, UVS[ci], tangent,
                ));
            }
            ix.extend_from_slice(&[base_vert, base_vert+1, base_vert+2, base_vert, base_vert+2, base_vert+3]);
        }
    }

    let mat_of = |b: Block| match b { Block::Grass => mat_grass, Block::Dirt => mat_dirt, Block::Stone => mat_stone };
    let mut result = Vec::new();
    for (block, v) in verts {
        if v.is_empty() { continue; }
        let ix = idxs.remove(&block).unwrap_or_default();
        result.push((GpuMesh::new(device, &v, &ix), mat_of(block)));
    }
    result
}

// ── VoxelChunkManager ─────────────────────────────────────────────────────────

/// Max new chunk requests sent to the streamer per frame.
const MAX_REQUESTS_PER_FRAME: usize = 6;
/// Max completed chunk events processed (mesh uploads) per frame.
const MAX_UPLOADS_PER_FRAME: usize = 4;

struct VoxelChunkManager {
    dir:        PathBuf,
    /// Chunks resident in the live `Level`: coord → (entity IDs, mesh handles to free).
    pub loaded: HashMap<ChunkCoord, (Vec<EntityId>, Vec<MeshHandle>)>,
    in_flight:  HashSet<ChunkCoord>,
    grass_mat:  MaterialHandle,
    dirt_mat:   MaterialHandle,
    stone_mat:  MaterialHandle,
    pending_ready: Vec<StreamEvent>,
}

impl VoxelChunkManager {
    fn new(
        dir:       PathBuf,
        grass_mat: MaterialHandle,
        dirt_mat:  MaterialHandle,
        stone_mat: MaterialHandle,
    ) -> Self {
        Self { dir, grass_mat, dirt_mat, stone_mat, loaded: HashMap::new(), in_flight: HashSet::new(), pending_ready: Vec::new() }
    }

    fn desired_set(&self, cam: Vec3) -> HashSet<ChunkCoord> {
        let cx = (cam.x / CHUNK_SIZE).floor() as i32;
        let cz = (cam.z / CHUNK_SIZE).floor() as i32;
        let mut set = HashSet::new();
        for dx in -LOAD_RADIUS..=LOAD_RADIUS {
            for dz in -LOAD_RADIUS..=LOAD_RADIUS {
                set.insert(ChunkCoord::new(cx + dx, 0, cz + dz));
            }
        }
        set
    }

    fn update(&mut self, cam: Vec3, level: &mut Level, streamer: &LevelStreamer, assets: &mut AssetRegistry) {
        let desired = self.desired_set(cam);

        let evict: Vec<ChunkCoord> = self.loaded.keys()
            .filter(|c| !desired.contains(c))
            .copied()
            .collect();
        for coord in evict {
            if let Some((ids, mesh_handles)) = self.loaded.remove(&coord) {
                for id in ids { level.despawn_entity(id); }
                for mh in mesh_handles { assets.remove(mh); }
            }
            level.partition_mut().remove_chunk(coord);
        }

        let mut to_request: Vec<ChunkCoord> = desired.into_iter()
            .filter(|c| !self.loaded.contains_key(c) && !self.in_flight.contains(c))
            .collect();
        let cx = cam.x; let cz = cam.z;
        to_request.sort_unstable_by(|a, b| {
            let da = (a.x as f32 * CHUNK_SIZE - cx).powi(2) + (a.z as f32 * CHUNK_SIZE - cz).powi(2);
            let db = (b.x as f32 * CHUNK_SIZE - cx).powi(2) + (b.z as f32 * CHUNK_SIZE - cz).powi(2);
            da.partial_cmp(&db).unwrap()
        });

        for coord in to_request.into_iter().take(MAX_REQUESTS_PER_FRAME) {
            self.in_flight.insert(coord);
            if chunk_on_disk(&self.dir, coord) {
                streamer.request_chunk(self.dir.clone(), coord);
            } else {
                streamer.request_generate_and_load(self.dir.clone(), coord, build_chunk_file(coord));
            }
        }
    }

    /// Accept newly arrived stream events into the pending queue.
    fn collect_events(&mut self, new_events: Vec<StreamEvent>) {
        self.pending_ready.extend(new_events);
    }

    /// Process up to MAX_UPLOADS_PER_FRAME pending events (GPU mesh uploads).
    fn flush_events(
        &mut self,
        level:  &mut Level,
        device: &wgpu::Device,
        assets: &mut AssetRegistry,
    ) {
        let take = self.pending_ready.len().min(MAX_UPLOADS_PER_FRAME);
        for event in self.pending_ready.drain(..take).collect::<Vec<_>>() {
            match event {
                StreamEvent::ChunkReady { coord, data } => {
                    self.in_flight.remove(&coord);
                    let submeshes = build_chunk_mesh(device, data, self.grass_mat, self.dirt_mat, self.stone_mat);
                    let cx = coord.x as f32 * CHUNK_SIZE + CHUNK_SIZE * 0.5;
                    let cz = coord.z as f32 * CHUNK_SIZE + CHUNK_SIZE * 0.5;
                    let chunk_centre = Vec3::new(cx, CHUNK_SIZE * 0.5, cz);

                    let mut ids = Vec::new();
                    let mut mesh_handles = Vec::new();

                    for (gpu_mesh, mat) in submeshes {
                        let mesh_h = assets.add(gpu_mesh);
                        mesh_handles.push(mesh_h);
                        ids.push(level.spawn_entity(
                            Components::new()
                                .with_transform(Transform::from_position(chunk_centre))
                                .with_mesh(mesh_h)
                                .with_material(mat)
                                .with_bounding_radius(CHUNK_SIZE * 1.5),
                        ));
                    }

                    level.partition_mut().get_or_create(coord).activate();
                    self.loaded.insert(coord, (ids, mesh_handles));
                }
                StreamEvent::ChunkError { coord, error } => {
                    self.in_flight.remove(&coord);
                    log::warn!("Chunk {:?}: {}", coord, error);
                }
            }
        }
    }
}

// ── Cache invalidation ────────────────────────────────────────────────────────

fn ensure_cache_valid(dir: &std::path::Path) {
    let key_path = dir.join(".cache_key");
    let current_ok = std::fs::read_to_string(&key_path)
        .map(|s| s.trim() == CACHE_KEY)
        .unwrap_or(false);
    if !current_ok {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).expect("create level dir");
        std::fs::write(&key_path, CACHE_KEY).expect("write cache key");
        log::info!("Level cache invalidated — regenerating chunks");
    }
}

// ── App ───────────────────────────────────────────────────────────────────────

struct App { state: Option<AppState> }
impl App { fn new() -> Self { Self { state: None } } }

struct AppState {
    window:         Arc<Window>,
    surface:        wgpu::Surface<'static>,
    device:         Arc<wgpu::Device>,
    queue:          Arc<wgpu::Queue>,
    surface_format: wgpu::TextureFormat,
    stratum:        Stratum,
    integration:    HelioIntegration,
    main_cam_id:    CameraId,
    chunks:         VoxelChunkManager,
    streamer:       LevelStreamer,
    last_frame:     std::time::Instant,
    keys:           HashSet<KeyCode>,
    cursor_grabbed: bool,
    mouse_delta:    (f32, f32),
    time:           f32,
    frame_count:    u32,
    fps_acc:        f32,
}

// ── ApplicationHandler ────────────────────────────────────────────────────────

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() { return; }

        ensure_cache_valid(&level_dir());

        let window = Arc::new(
            event_loop.create_window(
                Window::default_attributes()
                    .with_title("Stratum — Infinite Voxel World")
                    .with_inner_size(winit::dpi::LogicalSize::new(1280u32, 720u32)),
            ).expect("window"),
        );

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(), ..Default::default()
        });
        let surface = instance.create_surface(window.clone()).expect("surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference:       wgpu::PowerPreference::HighPerformance,
            compatible_surface:     Some(&surface),
            force_fallback_adapter: false,
        })).expect("adapter");

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label:                 Some("voxel device"),
                required_features:     wgpu::Features::EXPERIMENTAL_RAY_QUERY,
                required_limits:       wgpu::Limits::default()
                    .using_minimum_supported_acceleration_structure_values(),
                memory_hints:          wgpu::MemoryHints::default(),
                experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
                trace:                 wgpu::Trace::Off,
            },
        )).expect("device");

        let device = Arc::new(device);
        let queue  = Arc::new(queue);

        let caps = surface.get_capabilities(&adapter);
        let fmt  = caps.formats.iter().find(|f| f.is_srgb()).copied().unwrap_or(caps.formats[0]);
        let size = window.inner_size();

        surface.configure(&device, &wgpu::SurfaceConfiguration {
            usage:                         wgpu::TextureUsages::RENDER_ATTACHMENT,
            format:                        fmt,
            width:                         size.width,
            height:                        size.height,
            present_mode:                  wgpu::PresentMode::Fifo,
            alpha_mode:                    caps.alpha_modes[0],
            view_formats:                  vec![],
            desired_maximum_frame_latency: 2,
        });

        let features = FeatureRegistry::builder()
            .with_feature(LightingFeature::new())
            .with_feature(ShadowsFeature::new().with_atlas_size(2048).with_max_lights(4))
            .with_feature(BloomFeature::new().with_intensity(0.1).with_threshold(2.0))
            .build();

        let renderer = Renderer::new(
            device.clone(), queue.clone(),
            RendererConfig { width: size.width, height: size.height, surface_format: fmt, features },
        ).expect("renderer");

        let mut integration = HelioIntegration::new(renderer, AssetRegistry::new());

        // Three solid-colour PBR materials — one per block type.
        // Chunk meshes are built at load time; only material handles are stored here.
        let grass_mat = {
            let g = integration.create_material(
                &Material::new().with_base_color([0.24, 0.55, 0.16, 1.0]).with_roughness(0.9),
            );
            integration.assets_mut().add_material(g)
        };
        let dirt_mat = {
            let g = integration.create_material(
                &Material::new().with_base_color([0.47, 0.30, 0.14, 1.0]).with_roughness(1.0),
            );
            integration.assets_mut().add_material(g)
        };
        let stone_mat = {
            let g = integration.create_material(
                &Material::new().with_base_color([0.50, 0.50, 0.50, 1.0]).with_roughness(0.85),
            );
            integration.assets_mut().add_material(g)
        };

        let mut stratum  = Stratum::new(SimulationMode::Editor);
        let level_id     = stratum.create_level("voxel_world", CHUNK_SIZE, ACTIVATION_RADIUS);
        stratum.level_mut(level_id).unwrap().activate_all_chunks();

        // Single directional sun light.
        {
            let level = stratum.level_mut(level_id).unwrap();
            level.spawn_entity(
                Components::new()
                    .with_transform(Transform::from_position(Vec3::new(0.0, 100.0, 0.0)))
                    .with_light(LightData::Directional {
                        direction: Vec3::new(-0.4, -1.0, -0.3).normalize().to_array(),
                        color:     [1.0, 0.97, 0.88],
                        intensity: 8.0,
                    }),
            );
        }

        // Camera — start elevated, looking at terrain.
        let main_cam_id = stratum.register_camera(StratumCamera {
            id:            CameraId::PLACEHOLDER,
            kind:          CameraKind::EditorPerspective,
            position:      Vec3::new(4.0, 14.0, -12.0),
            yaw:           0.0,
            pitch:         -0.35,
            projection:    Projection::perspective(std::f32::consts::FRAC_PI_3, 0.1, 1000.0),
            render_target: RenderTargetHandle::PrimarySurface,
            viewport:      Viewport::full(),
            priority:      0,
            active:        true,
        });

        let streamer = LevelStreamer::new();
        let chunks   = VoxelChunkManager::new(
            level_dir(), grass_mat, dirt_mat, stone_mat,
        );

        self.state = Some(AppState {
            window, surface, device, queue, surface_format: fmt,
            stratum, integration, main_cam_id,
            chunks, streamer,
            last_frame:     std::time::Instant::now(),
            keys:           HashSet::new(),
            cursor_grabbed: false,
            mouse_delta:    (0.0, 0.0),
            time:           0.0,
            frame_count:    0,
            fps_acc:        0.0,
        });
    }

    fn window_event(
        &mut self, event_loop: &ActiveEventLoop,
        _id: WindowId, event: WindowEvent,
    ) {
        let Some(state) = &mut self.state else { return };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::KeyboardInput { event: KeyEvent {
                state: ElementState::Pressed,
                physical_key: PhysicalKey::Code(KeyCode::Escape), ..
            }, .. } => {
                if state.cursor_grabbed {
                    state.cursor_grabbed = false;
                    let _ = state.window.set_cursor_grab(CursorGrabMode::None);
                    state.window.set_cursor_visible(true);
                } else {
                    event_loop.exit();
                }
            }

            WindowEvent::KeyboardInput { event: KeyEvent {
                state: ElementState::Pressed,
                physical_key: PhysicalKey::Code(KeyCode::Tab), ..
            }, .. } => {
                state.stratum.toggle_mode();
                log::info!("Mode → {:?}", state.stratum.mode());
            }

            WindowEvent::KeyboardInput { event: KeyEvent {
                state: ks, physical_key: PhysicalKey::Code(key), ..
            }, .. } => {
                match ks {
                    ElementState::Pressed  => { state.keys.insert(key); }
                    ElementState::Released => { state.keys.remove(&key); }
                }
            }

            WindowEvent::MouseInput {
                state: ElementState::Pressed, button: MouseButton::Left, ..
            } => {
                if !state.cursor_grabbed {
                    let ok = state.window
                        .set_cursor_grab(CursorGrabMode::Confined)
                        .or_else(|_| state.window.set_cursor_grab(CursorGrabMode::Locked))
                        .is_ok();
                    if ok {
                        state.window.set_cursor_visible(false);
                        state.cursor_grabbed = true;
                    }
                }
            }

            WindowEvent::Resized(s) if s.width > 0 && s.height > 0 => {
                state.surface.configure(&state.device, &wgpu::SurfaceConfiguration {
                    usage:                         wgpu::TextureUsages::RENDER_ATTACHMENT,
                    format:                        state.surface_format,
                    width:                         s.width,
                    height:                        s.height,
                    present_mode:                  wgpu::PresentMode::Fifo,
                    alpha_mode:                    wgpu::CompositeAlphaMode::Auto,
                    view_formats:                  vec![],
                    desired_maximum_frame_latency: 2,
                });
                state.integration.resize(s.width, s.height);
            }

            WindowEvent::RedrawRequested => {
                let now = std::time::Instant::now();
                let dt  = (now - state.last_frame).as_secs_f32().min(0.1);
                state.last_frame = now;
                state.render(dt);
                state.window.request_redraw();
            }

            _ => {}
        }
    }

    fn device_event(
        &mut self, _: &ActiveEventLoop, _: winit::event::DeviceId, event: DeviceEvent,
    ) {
        let Some(state) = &mut self.state else { return };
        if let DeviceEvent::MouseMotion { delta: (dx, dy) } = event {
            if state.cursor_grabbed {
                state.mouse_delta.0 += dx as f32;
                state.mouse_delta.1 += dy as f32;
            }
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(s) = &self.state { s.window.request_redraw(); }
    }
}

// ── Per-frame logic ───────────────────────────────────────────────────────────

impl AppState {
    fn render(&mut self, dt: f32) {
        self.time       += dt;
        self.frame_count += 1;
        self.fps_acc    += dt;

        // Camera movement.
        {
            let cam = self.stratum.cameras_mut()
                .get_mut(self.main_cam_id).expect("camera");
            cam.yaw   += self.mouse_delta.0 * LOOK_SENS;
            cam.pitch  = (cam.pitch - self.mouse_delta.1 * LOOK_SENS).clamp(-1.5, 1.5);
            let fwd   = cam.forward();
            let right = cam.right();
            if self.keys.contains(&KeyCode::KeyW)      { cam.position += fwd   * CAM_SPEED * dt; }
            if self.keys.contains(&KeyCode::KeyS)      { cam.position -= fwd   * CAM_SPEED * dt; }
            if self.keys.contains(&KeyCode::KeyA)      { cam.position -= right * CAM_SPEED * dt; }
            if self.keys.contains(&KeyCode::KeyD)      { cam.position += right * CAM_SPEED * dt; }
            if self.keys.contains(&KeyCode::Space)     { cam.position += Vec3::Y * CAM_SPEED * dt; }
            if self.keys.contains(&KeyCode::ShiftLeft) { cam.position -= Vec3::Y * CAM_SPEED * dt; }
        }
        self.mouse_delta = (0.0, 0.0);

        let cam_pos = self.stratum.cameras_mut()
            .get_mut(self.main_cam_id)
            .map(|c| c.position)
            .unwrap_or(Vec3::ZERO);

        // Drain stream events into pending queue, then flush up to the per-frame cap.
        let new_events: Vec<_> = self.streamer.poll_loaded().into_iter().collect();
        self.chunks.collect_events(new_events);
        {
            let level  = self.stratum.active_level_mut().expect("level");
            let assets = self.integration.assets_mut();
            self.chunks.flush_events(level, &self.device, assets);
            self.chunks.update(cam_pos, level, &self.streamer, assets);
        }

        self.stratum.tick(dt);

        // Re-activate all manager-loaded chunks (tick's activation update can fight us).
        {
            let level = self.stratum.active_level_mut().expect("level");
            for &coord in self.chunks.loaded.keys() {
                level.partition_mut().get_or_create(coord).activate();
            }
        }

        // ── Stats every 10 frames ────────────────────────────────────────────
        if self.frame_count % 10 == 0 {
            let fps        = if self.fps_acc > 0.0 { 10.0 / self.fps_acc } else { 0.0 };
            let loaded     = self.chunks.loaded.len();
            let in_flight  = self.chunks.in_flight.len();
            let pending    = self.chunks.pending_ready.len();
            let meshes     = self.integration.assets_mut().mesh_count();
            eprintln!(
                "[frame {:5}] fps={:.1}  chunks loaded={} in_flight={} pending={}  meshes={}",
                self.frame_count, fps, loaded, in_flight, pending, meshes
            );
            self.fps_acc = 0.0;
        }

        let size  = self.window.inner_size();
        let views = self.stratum.build_views(size.width, size.height, self.time);
        if views.is_empty() { return; }

        let output = match self.surface.get_current_texture() {
            Ok(t)  => t,
            Err(e) => { log::warn!("Surface error: {e:?}"); return; }
        };
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let level = self.stratum.active_level().expect("level");


        if let Err(e) = self.integration.submit_frame(&views, level, &view, dt) {
            log::error!("Render error: {e:?}");
        }
        output.present();
    }
}

// ── main ──────────────────────────────────────────────────────────────────────

fn main() {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    log::info!(
        "Infinite voxel world — chunk {}m, load radius {} → {} chunks max",
        CHUNK_SIZE as i32, LOAD_RADIUS, (LOAD_RADIUS * 2 + 1).pow(2),
    );
    log::info!("WASD fly | Space/Shift up/down | Mouse look (click) | Tab mode | Esc exit");

    EventLoop::new().expect("event loop")
        .run_app(&mut App::new())
        .expect("run_app failed");
}
