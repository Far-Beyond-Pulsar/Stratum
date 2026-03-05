//! `level_fs` — Level file system with live rendering.
//!
//! Full lifecycle:
//!
//! 1. Upload GPU meshes to the integration's `AssetRegistry` — the resulting
//!    `MeshHandle` IDs are stable u64s that get written into the JSON chunk files.
//! 2. Build a world level (5 chunks, entities with those handles + lights) and
//!    **save it to `levels/example_world/`** on disk.
//! 3. Construct a fresh empty `Stratum` world — no entities in memory.
//! 4. **Stream** every chunk back via `LevelStreamer` (background OS thread).
//!    Entities are re-spawned into the level as each chunk arrives.
//! 5. Render continuously with Helio.
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
    BillboardData, CameraId, CameraKind, Components,
    LevelId, LightData, MaterialHandle, MeshHandle,
    Projection, RenderTargetHandle, SimulationMode,
    Stratum, StratumCamera, Transform, Viewport, Level, StreamEvent, LevelStreamer,
};
use stratum_helio::{AssetRegistry, HelioIntegration, Material, TextureData};

// ── Level directory ───────────────────────────────────────────────────────────

fn level_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("levels")
        .join("example_world")
}

// ── World constants ───────────────────────────────────────────────────────────

const CHUNK_SIZE:        f32 = 32.0;
const ACTIVATION_RADIUS: f32 = 128.0;
const CAM_SPEED:         f32 = 10.0;
const LOOK_SENS:         f32 = 0.002;

// ── Sprite loader ─────────────────────────────────────────────────────────────

fn load_sprite(path: &str) -> (Vec<u8>, u32, u32) {
    let bytes: Option<&'static [u8]> = match path {
        "image.png"     => Some(include_bytes!("../../assets/image.png")),
        "spotlight.png" => Some(include_bytes!("../../assets/spotlight.png")),
        _               => None,
    };
    let img = bytes
        .and_then(|b| image::load_from_memory(b).ok())
        .unwrap_or_else(|| {
            let mut px = image::RgbaImage::new(1, 1);
            px.put_pixel(0, 0, image::Rgba([255, 255, 255, 255]));
            image::DynamicImage::ImageRgba8(px)
        })
        .into_rgba8();
    let (w, h) = img.dimensions();
    (img.into_raw(), w, h)
}

// ─────────────────────────────────────────────────────────────────────────────
// Scene build + save
// ─────────────────────────────────────────────────────────────────────────────

/// Upload all GPU meshes into `assets`, build a `Level` using those handles,
/// save the level to disk, and return the cube `MaterialHandle`.
///
/// Because we write directly into the integration's `AssetRegistry`, every
/// handle ID stored in JSON is already valid when we stream the chunks back.
fn build_and_save(
    device:     &wgpu::Device,
    assets:     &mut AssetRegistry,
    cube_mat:   MaterialHandle,
) -> PathBuf {
    // ── Upload meshes (handle IDs are stable u64s assigned by the registry) ──
    let h_ground    = assets.add(GpuMesh::plane(device, [32.0,  0.0,  32.0], 36.0));

    // Chunk (0,0,0)
    let h_pillar_a  = assets.add(GpuMesh::cube(device, [ 8.0,  3.0,   8.0], 3.0));
    let h_pillar_b  = assets.add(GpuMesh::cube(device, [20.0,  2.0,  20.0], 2.0));
    let _h_lantern  = assets.add(GpuMesh::cube(device, [14.0,  5.5,  14.0], 0.3));

    // Chunk (1,0,0)  x ∈ [32, 64)
    let h_tower     = assets.add(GpuMesh::cube(device, [42.0,  5.0,  10.0], 5.0));
    let h_crate     = assets.add(GpuMesh::cube(device, [54.0,  1.5,  22.0], 1.5));
    let _h_blamp    = assets.add(GpuMesh::cube(device, [48.0,  7.0,  16.0], 0.3));

    // Chunk (0,0,1)  z ∈ [32, 64)
    let h_arch      = assets.add(GpuMesh::cube(device, [10.0,  2.0,  44.0], 2.0));
    let h_tree      = assets.add(GpuMesh::cube(device, [22.0,  3.0,  55.0], 3.0));

    // Chunk (-1,0,0) x ∈ [-32, 0)
    let h_rock      = assets.add(GpuMesh::cube(device, [-14.0, 1.0,  14.0], 1.0));
    let h_torch     = assets.add(GpuMesh::cube(device, [ -6.0, 4.0,  20.0], 0.5));

    // Chunk (2,0,2)  x ∈ [64,96), z ∈ [64,96)
    let h_monolith  = assets.add(GpuMesh::cube(device, [74.0,  8.0,  74.0], 8.0));
    let _h_redlamp  = assets.add(GpuMesh::cube(device, [74.0, 18.0,  74.0], 0.3));

    // ── Build Level ───────────────────────────────────────────────────────────
    let mut level = Level::new(LevelId::new(1), "example_world", CHUNK_SIZE, ACTIVATION_RADIUS);

    let r3 = f32::sqrt(3.0_f32);

    // Ground
    level.spawn_entity(
        Components::new()
            .with_transform(Transform::from_position(Vec3::new(32.0, 0.0, 32.0)))
            .with_mesh(h_ground)
            .with_bounding_radius(36.0 * f32::sqrt(2.0))
            .with_tag("ground"),
    );

    // Chunk (0,0,0) — static props
    for (pos, h, half, tag) in [
        (Vec3::new( 8.0, 3.0,  8.0), h_pillar_a, 3.0_f32, "pillar_a"),
        (Vec3::new(20.0, 2.0, 20.0), h_pillar_b, 2.0_f32, "pillar_b"),
    ] {
        level.spawn_entity(
            Components::new()
                .with_transform(Transform::from_position(pos))
                .with_mesh(h).with_material(cube_mat)
                .with_bounding_radius(half * r3).with_tag(tag),
        );
    }
    // Orange lantern (light + billboard)
    level.spawn_entity(
        Components::new()
            .with_transform(Transform::from_position(Vec3::new(14.0, 5.5, 14.0)))
            .with_light(LightData::Point { color: [1.0, 0.75, 0.3], intensity: 6.0, range: 18.0 })
            .with_billboard(BillboardData::new(1.5, 1.5, [1.0, 0.75, 0.3, 0.9]))
            .with_tag("lantern"),
    );

    // Chunk (1,0,0)
    for (pos, h, half, tag) in [
        (Vec3::new(42.0, 5.0, 10.0), h_tower, 5.0_f32, "tower"),
        (Vec3::new(54.0, 1.5, 22.0), h_crate, 1.5_f32, "crate"),
    ] {
        level.spawn_entity(
            Components::new()
                .with_transform(Transform::from_position(pos))
                .with_mesh(h).with_material(cube_mat)
                .with_bounding_radius(half * r3).with_tag(tag),
        );
    }
    // Blue lamp
    level.spawn_entity(
        Components::new()
            .with_transform(Transform::from_position(Vec3::new(48.0, 7.0, 16.0)))
            .with_light(LightData::Point { color: [0.3, 0.6, 1.0], intensity: 5.0, range: 16.0 })
            .with_billboard(BillboardData::new(1.5, 1.5, [0.3, 0.6, 1.0, 0.9]))
            .with_tag("blue_lamp"),
    );

    // Chunk (0,0,1)
    for (pos, h, half, tag) in [
        (Vec3::new(10.0, 2.0, 44.0), h_arch, 2.0_f32, "arch"),
        (Vec3::new(22.0, 3.0, 55.0), h_tree, 3.0_f32, "tree"),
    ] {
        level.spawn_entity(
            Components::new()
                .with_transform(Transform::from_position(pos))
                .with_mesh(h).with_material(cube_mat)
                .with_bounding_radius(half * r3).with_tag(tag),
        );
    }

    // Chunk (-1,0,0)
    for (pos, h, half, tag) in [
        (Vec3::new(-14.0, 1.0, 14.0), h_rock,  1.0_f32, "rock"),
        (Vec3::new( -6.0, 4.0, 20.0), h_torch, 0.5_f32, "torch"),
    ] {
        level.spawn_entity(
            Components::new()
                .with_transform(Transform::from_position(pos))
                .with_mesh(h).with_material(cube_mat)
                .with_bounding_radius(half * r3).with_tag(tag),
        );
    }
    // Torch flame light
    level.spawn_entity(
        Components::new()
            .with_transform(Transform::from_position(Vec3::new(-6.0, 6.0, 20.0)))
            .with_light(LightData::Point { color: [1.0, 0.5, 0.1], intensity: 5.0, range: 12.0 })
            .with_billboard(BillboardData::new(1.2, 1.2, [1.0, 0.5, 0.1, 0.85]))
            .with_tag("torch_light"),
    );

    // Chunk (2,0,2) — boss monolith
    level.spawn_entity(
        Components::new()
            .with_transform(Transform::from_position(Vec3::new(74.0, 8.0, 74.0)))
            .with_mesh(h_monolith).with_material(cube_mat)
            .with_bounding_radius(8.0 * r3).with_tag("monolith"),
    );
    // Red overhead light
    level.spawn_entity(
        Components::new()
            .with_transform(Transform::from_position(Vec3::new(74.0, 18.0, 74.0)))
            .with_light(LightData::Point { color: [1.0, 0.1, 0.1], intensity: 7.0, range: 22.0 })
            .with_billboard(BillboardData::new(1.8, 1.8, [1.0, 0.1, 0.1, 0.9]))
            .with_tag("boss_light"),
    );

    // ── Save ──────────────────────────────────────────────────────────────────
    let dir = level_dir();
    let _ = std::fs::remove_dir_all(&dir);
    save_level(&level, &dir).expect("save_level failed");
    log::info!(
        "Saved level '{}' — {} entities, {} chunks → {}",
        level.name,
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

        // ── Window ────────────────────────────────────────────────────────────
        let window = Arc::new(
            event_loop.create_window(
                Window::default_attributes()
                    .with_title("Stratum — Level FS streaming")
                    .with_inner_size(winit::dpi::LogicalSize::new(1280u32, 720u32)),
            ).expect("window"),
        );

        // ── wgpu ──────────────────────────────────────────────────────────────
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
                label:                 Some("level_fs device"),
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

        let caps   = surface.get_capabilities(&adapter);
        let fmt    = caps.formats.iter().find(|f| f.is_srgb()).copied().unwrap_or(caps.formats[0]);
        let size   = window.inner_size();

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

        // ── Renderer ──────────────────────────────────────────────────────────
        let (sprite_rgba, sw, sh) = load_sprite("spotlight.png");
        let features = FeatureRegistry::builder()
            .with_feature(LightingFeature::new())
            .with_feature(BloomFeature::new().with_intensity(0.3).with_threshold(1.2))
            .with_feature(ShadowsFeature::new().with_atlas_size(2048).with_max_lights(8))
            .with_feature(BillboardsFeature::new().with_sprite(sprite_rgba, sw, sh))
            .with_feature(
                RadianceCascadesFeature::new()
                    .with_world_bounds([-32.0, -2.0, -32.0], [100.0, 24.0, 100.0]),
            )
            .build();

        let renderer = Renderer::new(
            device.clone(), queue.clone(),
            RendererConfig { width: size.width, height: size.height, surface_format: fmt, features },
        ).expect("renderer");

        // ── Integration + cube material ───────────────────────────────────────
        let mut integration = HelioIntegration::new(renderer, AssetRegistry::new());

        let (tex, tw, th) = load_sprite("image.png");
        let gpu_mat = integration.create_material(
            &Material::new()
                .with_roughness(0.5)
                .with_metallic(0.0)
                .with_base_color_texture(TextureData::new(tex, tw, th)),
        );
        let cube_mat = integration.assets_mut().add_material(gpu_mat);

        // ── Build scene and save to disk ──────────────────────────────────────
        // Meshes go directly into integration.assets so handle IDs in JSON match.
        let dir = build_and_save(&device, integration.assets_mut(), cube_mat);

        // ── Stratum world (empty — entities stream in from disk) ──────────────
        let mut stratum = Stratum::new(SimulationMode::Editor);
        let level_id    = stratum.create_level("example_world", CHUNK_SIZE, ACTIVATION_RADIUS);
        stratum.level_mut(level_id).unwrap().activate_all_chunks();

        // ── Camera ────────────────────────────────────────────────────────────
        let main_cam_id = stratum.register_camera(StratumCamera {
            id:            CameraId::PLACEHOLDER,
            kind:          CameraKind::EditorPerspective,
            position:      Vec3::new(20.0, 12.0, 88.0),
            yaw:           0.0,
            pitch:         -0.18,
            projection:    Projection::perspective(std::f32::consts::FRAC_PI_3, 0.1, 500.0),
            render_target: RenderTargetHandle::PrimarySurface,
            viewport:      Viewport::full(),
            priority:      0,
            active:        true,
        });

        // ── Stream all chunks from disk ───────────────────────────────────────
        let streamer = LevelStreamer::new();
        let coords   = discover_chunk_coords(&dir).expect("discover_chunk_coords");
        let expected = coords.len();
        log::info!("Requesting {} chunk(s) from streamer …", expected);
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

    // ── Window events ─────────────────────────────────────────────────────────

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

        // ── Poll streamer; spawn arrived entities ─────────────────────────────
        for event in self.streamer.poll_loaded() {
            let level = self.stratum.active_level_mut().expect("level");
            match event {
                StreamEvent::ChunkReady { coord, data } => {
                    let n = data.entities.len();
                    for (_id, components) in chunk_to_components(data) {
                        level.spawn_entity(components);
                    }
                    log::debug!("Chunk {:?} — {} entities spawned", coord, n);
                    if let LoadPhase::Streaming { loaded, .. } = &mut self.load_phase {
                        *loaded += 1;
                    }
                }
                StreamEvent::ChunkError { coord, error } => {
                    log::warn!("Chunk {:?} error: {}", coord, error);
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
                log::info!("All {} chunk(s) loaded — {} entities resident", expected, n);
            }
        }

        // ── Camera movement ───────────────────────────────────────────────────
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

        // ── Tick + build views ────────────────────────────────────────────────
        self.stratum.tick(dt);
        let size  = self.window.inner_size();
        let views = self.stratum.build_views(size.width, size.height, self.time);
        if views.is_empty() { return; }

        // ── Acquire surface ───────────────────────────────────────────────────
        let output = match self.surface.get_current_texture() {
            Ok(t)  => t,
            Err(e) => { log::warn!("Surface error: {e:?}"); return; }
        };
        let surface_view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let level = self.stratum.active_level().expect("level");

        if self.stratum.mode() == SimulationMode::Editor {
            self.integration.debug_draw_world_partition(level.partition());
            self.integration.debug_draw_lights(level.entities());
        }

        if let Err(e) = self.integration.submit_frame(&views, level, &surface_view, dt) {
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

    log::info!("Stratum level_fs — WASD fly | Space/Shift up/down | Mouse look | Tab mode | Esc exit");

    EventLoop::new().expect("event loop")
        .run_app(&mut App::new())
        .expect("event loop error");
}
