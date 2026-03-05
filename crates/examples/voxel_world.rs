//! `voxel_world` — Procedurally generated Minecraft-style voxel world.
//!
//! Uses a simple hash-based heightmap to generate a world of 1 m³ cubes across
//! a 16×16 chunk grid.  Each column gets a grass-top / dirt body / stone base
//! stratification based on depth.  The level is saved to
//! `levels/voxel_world/` via the Stratum level FS and streamed back chunk-by-
//! chunk before the first rendered frame.
//!
//! ## Controls
//!
//! | Key            | Action                              |
//! |----------------|-------------------------------------|
//! | WASD           | Fly forward / left / back / right   |
//! | Space / LShift | Fly up / down                       |
//! | Mouse drag     | Look (click window to grab cursor)  |
//! | Tab            | Toggle Editor ↔ Game mode           |
//! | Escape         | Release cursor / exit               |

use std::collections::HashSet;
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
    GpuMesh, Renderer, RendererConfig,
    features::{
        BillboardsFeature, BloomFeature, FeatureRegistry,
        LightingFeature, RadianceCascadesFeature, ShadowsFeature,
    },
};

use stratum::{
    chunk_to_components, discover_chunk_coords, save_level,
    CameraId, CameraKind, Components, LevelId,
    MaterialHandle, MeshHandle, Projection, RenderTargetHandle,
    SimulationMode, Stratum, StratumCamera, Transform, Viewport,
    Level, StreamEvent, LevelStreamer,
};
use stratum_helio::{AssetRegistry, HelioIntegration, Material, TextureData};

// ── Level directory ───────────────────────────────────────────────────────────

fn level_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("levels")
        .join("voxel_world")
}

// ── World generation parameters ───────────────────────────────────────────────

/// Size of each Stratum spatial chunk in world units (= voxel metres here).
const CHUNK_SIZE: f32 = 16.0;
/// Streaming activation radius — keep 3 chunks around the camera loaded.
const ACTIVATION_RADIUS: f32 = 48.0;
/// How many chunk columns to generate in X and Z.
const WORLD_CHUNKS_X: i32 = 16;
const WORLD_CHUNKS_Z: i32 = 16;
/// Voxels per chunk edge (chunk is VOXELS_PER_CHUNK × VOXELS_PER_CHUNK in XZ).
const VOXELS_PER_CHUNK: i32 = 16;
/// Minimum terrain height (stone floor).
const BASE_HEIGHT: i32 = 2;
/// Maximum additional height from the heightmap.
const HEIGHT_RANGE: i32 = 12;

const CAM_SPEED: f32 = 12.0;
const LOOK_SENS: f32 = 0.002;

// ── Voxel type ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum Block { Grass, Dirt, Stone }

// ── Procedural heightmap (no external noise dep) ──────────────────────────────
//
// Good-enough hash-based noise: mixes x/z with a few prime multiplications,
// then adds octaves by calling itself at higher frequencies.

fn hash(x: i32, z: i32, seed: u64) -> f32 {
    let mut v = (x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (z as u64).wrapping_mul(0x6c62_272e_07bb_0142)
        ^ seed;
    v ^= v >> 30;
    v = v.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    v ^= v >> 27;
    v = v.wrapping_mul(0x94d0_49bb_1331_11eb);
    v ^= v >> 31;
    (v as f32) / (u64::MAX as f32)
}

fn octave_noise(x: f32, z: f32, seed: u64) -> f32 {
    let xi = x.floor() as i32;
    let zi = z.floor() as i32;
    let fx = x - xi as f32;
    let fz = z - zi as f32;
    // Smooth interpolation
    let ux = fx * fx * (3.0 - 2.0 * fx);
    let uz = fz * fz * (3.0 - 2.0 * fz);
    let v00 = hash(xi,     zi,     seed);
    let v10 = hash(xi + 1, zi,     seed);
    let v01 = hash(xi,     zi + 1, seed);
    let v11 = hash(xi + 1, zi + 1, seed);
    let top = v00 + ux * (v10 - v00);
    let bot = v01 + ux * (v11 - v01);
    top + uz * (bot - top)
}

fn terrain_height(wx: i32, wz: i32) -> i32 {
    let x = wx as f32;
    let z = wz as f32;
    // 3 octaves of noise at different scales
    let n = octave_noise(x / 24.0, z / 24.0, 0)
          + octave_noise(x / 12.0, z / 12.0, 1) * 0.5
          + octave_noise(x /  6.0, z /  6.0, 2) * 0.25;
    let n = n / 1.75; // normalise to ~[0,1]
    BASE_HEIGHT + (n * HEIGHT_RANGE as f32) as i32
}

fn block_at(wx: i32, wy: i32, wz: i32) -> Option<Block> {
    let h = terrain_height(wx, wz);
    if wy > h       { return None; }
    if wy == h      { return Some(Block::Grass); }
    if wy >= h - 3  { return Some(Block::Dirt);  }
    Some(Block::Stone)
}

// ── World generation ──────────────────────────────────────────────────────────

fn generate_world(
    device:    &wgpu::Device,
    assets:    &mut AssetRegistry,
    grass_mat: MaterialHandle,
    dirt_mat:  MaterialHandle,
    stone_mat: MaterialHandle,
) -> PathBuf {
    // One shared unit-cube mesh per block type.
    // GpuMesh::cube takes (center_position, half_size) — we use [0,0,0] as
    // placeholder; actual position comes from the entity Transform.
    let h_grass = assets.add(GpuMesh::cube(device, [0.0, 0.0, 0.0], 0.5));
    let h_dirt  = assets.add(GpuMesh::cube(device, [0.0, 0.0, 0.0], 0.5));
    let h_stone = assets.add(GpuMesh::cube(device, [0.0, 0.0, 0.0], 0.5));

    let mut level = Level::new(
        LevelId::new(2),
        "voxel_world",
        CHUNK_SIZE,
        ACTIVATION_RADIUS,
    );

    let total_x = WORLD_CHUNKS_X * VOXELS_PER_CHUNK;
    let total_z = WORLD_CHUNKS_Z * VOXELS_PER_CHUNK;
    let max_h   = BASE_HEIGHT + HEIGHT_RANGE + 1;

    let mut block_count = 0usize;

    for wx in 0..total_x {
        for wz in 0..total_z {
            let h = terrain_height(wx, wz);
            for wy in 0..=h.min(max_h) {
                let Some(block) = block_at(wx, wy, wz) else { continue };

                let (mesh, mat) = match block {
                    Block::Grass => (h_grass, grass_mat),
                    Block::Dirt  => (h_dirt,  dirt_mat),
                    Block::Stone => (h_stone, stone_mat),
                };

                // Centre each 1 m³ cube at (wx+0.5, wy+0.5, wz+0.5).
                let pos = Vec3::new(wx as f32 + 0.5, wy as f32 + 0.5, wz as f32 + 0.5);

                level.spawn_entity(
                    Components::new()
                        .with_transform(Transform::from_position(pos))
                        .with_mesh(mesh)
                        .with_material(mat)
                        .with_bounding_radius(0.5 * f32::sqrt(3.0)),
                );
                block_count += 1;
            }
        }
    }

    log::info!(
        "Generated {} blocks across {}×{} columns",
        block_count, total_x, total_z
    );

    // Save to disk.
    let dir = level_dir();
    let _ = std::fs::remove_dir_all(&dir);
    save_level(&level, &dir).expect("save_level failed");
    log::info!(
        "Saved voxel level — {} entities, {} chunks → {}",
        level.entities().len(),
        level.partition().chunks().count(),
        dir.display()
    );

    dir
}

// ─────────────────────────────────────────────────────────────────────────────
// Load phase
// ─────────────────────────────────────────────────────────────────────────────

enum LoadPhase {
    Streaming { expected: usize, loaded: usize },
    Ready,
}

// ─────────────────────────────────────────────────────────────────────────────
// App
// ─────────────────────────────────────────────────────────────────────────────

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

    streamer:       LevelStreamer,
    load_phase:     LoadPhase,

    last_frame:     std::time::Instant,
    keys:           HashSet<KeyCode>,
    cursor_grabbed: bool,
    mouse_delta:    (f32, f32),
    time:           f32,
}

// ─────────────────────────────────────────────────────────────────────────────
// ApplicationHandler
// ─────────────────────────────────────────────────────────────────────────────

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() { return; }

        let window = Arc::new(
            event_loop.create_window(
                Window::default_attributes()
                    .with_title("Stratum — Voxel World")
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

        // Renderer — no billboards needed for a pure voxel world, but keep
        // lighting + shadows so it looks nice.
        let world_w = (WORLD_CHUNKS_X * VOXELS_PER_CHUNK) as f32;
        let world_z = (WORLD_CHUNKS_Z * VOXELS_PER_CHUNK) as f32;
        let world_h = (BASE_HEIGHT + HEIGHT_RANGE + 4) as f32;

        let (sprite_rgba, sw, sh) = load_sprite();
        let features = FeatureRegistry::builder()
            .with_feature(LightingFeature::new())
            .with_feature(BloomFeature::new().with_intensity(0.2).with_threshold(1.5))
            .with_feature(ShadowsFeature::new().with_atlas_size(4096).with_max_lights(4))
            .with_feature(BillboardsFeature::new().with_sprite(sprite_rgba, sw, sh))
            .with_feature(
                RadianceCascadesFeature::new()
                    .with_world_bounds([0.0, -1.0, 0.0], [world_w, world_h, world_z]),
            )
            .build();

        let renderer = Renderer::new(
            device.clone(), queue.clone(),
            RendererConfig { width: size.width, height: size.height, surface_format: fmt, features },
        ).expect("renderer");

        let mut integration = HelioIntegration::new(renderer, AssetRegistry::new());

        // ── Three block materials ─────────────────────────────────────────────
        let grass_mat = {
            let gpu = integration.create_material(
                &Material::new().with_roughness(0.9).with_metallic(0.0)
                    .with_base_color([0.24, 0.55, 0.16, 1.0]),
            );
            integration.assets_mut().add_material(gpu)
        };
        let dirt_mat = {
            let gpu = integration.create_material(
                &Material::new().with_roughness(1.0).with_metallic(0.0)
                    .with_base_color([0.47, 0.30, 0.14, 1.0]),
            );
            integration.assets_mut().add_material(gpu)
        };
        let stone_mat = {
            let gpu = integration.create_material(
                &Material::new().with_roughness(0.85).with_metallic(0.0)
                    .with_base_color([0.50, 0.50, 0.50, 1.0]),
            );
            integration.assets_mut().add_material(gpu)
        };

        // ── Generate and save world ───────────────────────────────────────────
        log::info!("Generating voxel world ({}×{} chunk columns) …",
            WORLD_CHUNKS_X, WORLD_CHUNKS_Z);
        let dir = generate_world(
            &device, integration.assets_mut(),
            grass_mat, dirt_mat, stone_mat,
        );

        // ── Empty Stratum world — stream entities in from disk ────────────────
        let mut stratum = Stratum::new(SimulationMode::Editor);
        let level_id    = stratum.create_level("voxel_world", CHUNK_SIZE, ACTIVATION_RADIUS);
        stratum.level_mut(level_id).unwrap().activate_all_chunks();

        // ── Camera — start above the centre of the world, looking down ────────
        let cx = (WORLD_CHUNKS_X * VOXELS_PER_CHUNK) as f32 * 0.5;
        let cz = (WORLD_CHUNKS_Z * VOXELS_PER_CHUNK) as f32 * 0.5;
        let main_cam_id = stratum.register_camera(StratumCamera {
            id:            CameraId::PLACEHOLDER,
            kind:          CameraKind::EditorPerspective,
            position:      Vec3::new(cx, world_h + 8.0, cz + 30.0),
            yaw:           0.0,
            pitch:         -0.35,
            projection:    Projection::perspective(std::f32::consts::FRAC_PI_3, 0.1, 1000.0),
            render_target: RenderTargetHandle::PrimarySurface,
            viewport:      Viewport::full(),
            priority:      0,
            active:        true,
        });

        // ── Stream chunks from disk ───────────────────────────────────────────
        let streamer = LevelStreamer::new();
        let coords   = discover_chunk_coords(&dir).expect("discover_chunk_coords");
        let expected = coords.len();
        log::info!("Streaming {} chunks …", expected);
        for coord in coords {
            streamer.request_chunk(dir.clone(), coord);
        }

        self.state = Some(AppState {
            window, surface, device, queue, surface_format: fmt,
            stratum, integration, main_cam_id,
            streamer,
            load_phase: LoadPhase::Streaming { expected, loaded: 0 },
            last_frame:     std::time::Instant::now(),
            keys:           HashSet::new(),
            cursor_grabbed: false,
            mouse_delta:    (0.0, 0.0),
            time:           0.0,
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

// ─────────────────────────────────────────────────────────────────────────────
// Per-frame render
// ─────────────────────────────────────────────────────────────────────────────

impl AppState {
    fn render(&mut self, dt: f32) {
        self.time += dt;

        // ── Drain streamer ────────────────────────────────────────────────────
        for event in self.streamer.poll_loaded() {
            let level = self.stratum.active_level_mut().expect("level");
            match event {
                StreamEvent::ChunkReady { coord, data } => {
                    let n = data.entities.len();
                    for (_id, components) in chunk_to_components(data) {
                        level.spawn_entity(components);
                    }
                    log::debug!("Chunk {:?} — {} blocks", coord, n);
                    if let LoadPhase::Streaming { loaded, .. } = &mut self.load_phase {
                        *loaded += 1;
                    }
                }
                StreamEvent::ChunkError { coord, error } => {
                    log::warn!("Chunk {:?}: {}", coord, error);
                    if let LoadPhase::Streaming { loaded, .. } = &mut self.load_phase {
                        *loaded += 1;
                    }
                }
            }
        }

        if let LoadPhase::Streaming { expected, loaded } = self.load_phase {
            if loaded >= expected {
                self.load_phase = LoadPhase::Ready;
                let n = self.stratum.active_level().map(|l| l.entities().len()).unwrap_or(0);
                log::info!("World ready — {} blocks loaded from {} chunks", n, expected);
            }
        }

        // ── Camera ────────────────────────────────────────────────────────────
        {
            let cam = self.stratum.cameras_mut()
                .get_mut(self.main_cam_id).expect("camera");

            cam.yaw   += self.mouse_delta.0 * LOOK_SENS;
            cam.pitch  = (cam.pitch - self.mouse_delta.1 * LOOK_SENS).clamp(-1.5, 1.5);

            let fwd   = cam.forward();
            let right = cam.right();
            let keys  = &self.keys;
            if keys.contains(&KeyCode::KeyW)      { cam.position += fwd   * CAM_SPEED * dt; }
            if keys.contains(&KeyCode::KeyS)      { cam.position -= fwd   * CAM_SPEED * dt; }
            if keys.contains(&KeyCode::KeyA)      { cam.position -= right * CAM_SPEED * dt; }
            if keys.contains(&KeyCode::KeyD)      { cam.position += right * CAM_SPEED * dt; }
            if keys.contains(&KeyCode::Space)     { cam.position += Vec3::Y * CAM_SPEED * dt; }
            if keys.contains(&KeyCode::ShiftLeft) { cam.position -= Vec3::Y * CAM_SPEED * dt; }
        }
        self.mouse_delta = (0.0, 0.0);

        // ── Tick + views ──────────────────────────────────────────────────────
        self.stratum.tick(dt);
        let size  = self.window.inner_size();
        let views = self.stratum.build_views(size.width, size.height, self.time);
        if views.is_empty() { return; }

        let output = match self.surface.get_current_texture() {
            Ok(t)  => t,
            Err(e) => { log::warn!("Surface error: {e:?}"); return; }
        };
        let surface_view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let level = self.stratum.active_level().expect("level");

        if self.stratum.mode() == SimulationMode::Editor {
            self.integration.debug_draw_world_partition(level.partition());
        }

        if let Err(e) = self.integration.submit_frame(&views, level, &surface_view, dt) {
            log::error!("Render error: {e:?}");
        }

        output.present();
    }
}

// ── Sprite fallback (no billboard in this demo, but BillboardsFeature needs one)

fn load_sprite() -> (Vec<u8>, u32, u32) {
    let bytes: &[u8] = include_bytes!("../../assets/spotlight.png");
    let img = image::load_from_memory(bytes)
        .unwrap_or_else(|_| {
            let mut px = image::RgbaImage::new(1, 1);
            px.put_pixel(0, 0, image::Rgba([255, 255, 255, 255]));
            image::DynamicImage::ImageRgba8(px)
        })
        .into_rgba8();
    let (w, h) = img.dimensions();
    (img.into_raw(), w, h)
}

// ── main ──────────────────────────────────────────────────────────────────────

fn main() {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    log::info!(
        "Voxel world — {}×{} chunks, {}×{} blocks per chunk",
        WORLD_CHUNKS_X, WORLD_CHUNKS_Z,
        VOXELS_PER_CHUNK, VOXELS_PER_CHUNK
    );
    log::info!("Controls: WASD fly | Space/Shift up/down | Mouse look | Tab mode | Esc exit");

    EventLoop::new().expect("event loop")
        .run_app(&mut App::new())
        .expect("event loop error");
}
