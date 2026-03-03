//! `stratum_basic` — Feature parity with `render_v2_basic`, driven by Stratum.
//!
//! This example reproduces the same visual output as the Helio
//! `render_v2_basic` example (three lit cubes, a ground plane, three point
//! lights) while routing everything through the Stratum world layer:
//!
//! ```text
//! AppState
//!   └── Stratum  ─────────────────── world orchestrator
//!         ├── Level                  entities + spatial partition
//!         └── CameraRegistry         one GameCamera (free-fly)
//!   └── HelioIntegration ─────────── Stratum → Helio bridge
//!         ├── Renderer               Helio render-v2
//!         └── AssetRegistry          MeshHandle → GpuMesh
//! ```
//!
//! ## Controls
//!
//!   WASD        — move forward / left / back / right
//!   Space/Shift — move up / down
//!   Mouse drag  — look (click to grab cursor)
//!   Escape      — release cursor / exit
//!   Tab         — toggle Editor ↔ Game mode (editor cameras hidden in Game)

use std::collections::HashSet;
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
        FeatureRegistry, LightingFeature, BloomFeature,
        ShadowsFeature, BillboardsFeature,
        RadianceCascadesFeature,
    },
};

use stratum::{
    CameraId, CameraKind, Components, LightData,
    Projection, RenderTargetHandle, SimulationMode,
    Stratum, StratumCamera, Transform, Viewport,
};

use stratum_helio::{AssetRegistry, HelioIntegration};

// ─────────────────────────────────────────────────────────────────────────────

fn load_sprite(path: &str) -> (Vec<u8>, u32, u32) {
    let img = image::open(path)
        .unwrap_or_else(|_| {
            log::warn!("Could not load '{}', using 1×1 white opaque fallback", path);
            // new_rgba8 zero-initialises (alpha=0 → billboards would be discarded).
            // Use a fully-opaque white pixel so colour tints render correctly.
            let mut px = image::RgbaImage::new(1, 1);
            px.put_pixel(0, 0, image::Rgba([255, 255, 255, 255]));
            image::DynamicImage::ImageRgba8(px)
        })
        .into_rgba8();
    let (w, h) = img.dimensions();
    (img.into_raw(), w, h)
}

// ─────────────────────────────────────────────────────────────────────────────

fn main() {
    env_logger::init();
    log::info!("Stratum Basic — starting");

    let event_loop = EventLoop::new().expect("Failed to create event loop");
    let mut app    = App::new();
    event_loop.run_app(&mut app).expect("Event loop error");
}

// ── App shell ─────────────────────────────────────────────────────────────────

struct App {
    state: Option<AppState>,
}

impl App {
    fn new() -> Self { Self { state: None } }
}

// ── AppState ──────────────────────────────────────────────────────────────────

struct AppState {
    // ── Window / GPU surface ──────────────────────────────────────────────────
    window:         Arc<Window>,
    surface:        wgpu::Surface<'static>,
    device:         Arc<wgpu::Device>,
    surface_format: wgpu::TextureFormat,

    // ── World orchestration ───────────────────────────────────────────────────
    stratum:         Stratum,
    integration:     HelioIntegration,
    main_camera_id:  CameraId,

    // ── Per-frame timing ──────────────────────────────────────────────────────
    last_frame: std::time::Instant,

    // ── Input state ───────────────────────────────────────────────────────────
    keys:           HashSet<KeyCode>,
    cursor_grabbed: bool,
    mouse_delta:    (f32, f32),
}

// ── ApplicationHandler ────────────────────────────────────────────────────────

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() { return; }

        // ── Window ────────────────────────────────────────────────────────────
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Stratum Basic — Pulsar World Layer Demo")
                        .with_inner_size(winit::dpi::LogicalSize::new(1280u32, 720u32)),
                )
                .expect("Failed to create window"),
        );

        // ── wgpu device / surface ─────────────────────────────────────────────
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        let surface = instance
            .create_surface(window.clone())
            .expect("Failed to create surface");

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference:       wgpu::PowerPreference::HighPerformance,
            compatible_surface:     Some(&surface),
            force_fallback_adapter: false,
        }))
        .expect("Failed to find adapter");

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("Stratum Main Device"),
                required_features: wgpu::Features::EXPERIMENTAL_RAY_QUERY,
                required_limits: wgpu::Limits::default()
                    .using_minimum_supported_acceleration_structure_values(),
                memory_hints: wgpu::MemoryHints::default(),
                // SAFETY: acknowledging experimental ray-tracing extension.
                experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
                trace: wgpu::Trace::Off,
            },
        ))
        .expect("Failed to create wgpu device (ray tracing required)");

        let device = Arc::new(device);
        let queue  = Arc::new(queue);

        let surface_caps   = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let size = window.inner_size();
        surface.configure(&device, &wgpu::SurfaceConfiguration {
            usage:                         wgpu::TextureUsages::RENDER_ATTACHMENT,
            format:                        surface_format,
            width:                         size.width,
            height:                        size.height,
            present_mode:                  wgpu::PresentMode::Fifo,
            alpha_mode:                    surface_caps.alpha_modes[0],
            view_formats:                  vec![],
            desired_maximum_frame_latency: 2,
        });

        // ── Helio renderer ────────────────────────────────────────────────────
        let (sprite_rgba, sprite_w, sprite_h) = load_sprite("spotlight.png");
        let feature_registry = FeatureRegistry::builder()
            .with_feature(LightingFeature::new())
            .with_feature(BloomFeature::new().with_intensity(0.4).with_threshold(1.2))
            .with_feature(ShadowsFeature::new().with_atlas_size(1024).with_max_lights(4))
            .with_feature(BillboardsFeature::new()
                .with_sprite(sprite_rgba, sprite_w, sprite_h))
            .with_feature(
                RadianceCascadesFeature::new()
                    .with_world_bounds([-3.5, -0.3, -3.5], [3.5, 5.0, 3.5]),
            )
            .build();

        let renderer = Renderer::new(
            device.clone(),
            queue.clone(),
            RendererConfig {
                width:          size.width,
                height:         size.height,
                surface_format,
                features:       feature_registry,
            },
        )
        .expect("Failed to create Helio renderer");

        // ── Asset registry: register GpuMeshes ───────────────────────────────
        let mut assets = AssetRegistry::new();

        let h_cube1  = assets.add(GpuMesh::cube (&device, [ 0.0, 0.5,  0.0], 0.5));
        let h_cube2  = assets.add(GpuMesh::cube (&device, [-2.0, 0.4, -1.0], 0.4));
        let h_cube3  = assets.add(GpuMesh::cube (&device, [ 2.0, 0.3,  0.5], 0.3));
        let h_ground = assets.add(GpuMesh::plane(&device, [ 0.0, 0.0,  0.0], 5.0));

        let integration = HelioIntegration::new(renderer, assets);

        // ── Stratum world setup ───────────────────────────────────────────────
        //
        // Start in Game mode so the GameCamera renders immediately.
        // Press Tab at runtime to toggle into Editor mode (camera hidden).
        let mut stratum = Stratum::new(SimulationMode::Game);

        // Create a level with a 16 m chunk size and 48 m activation radius.
        // The demo scene fits in one chunk, so everything is always active.
        let level_id = stratum.create_level("demo_level", 16.0, 48.0);
        let level    = stratum.level_mut(level_id).unwrap();

        // Spawn static mesh entities
        level.spawn_entity(
            Components::new()
                .with_transform(Transform::from_position(Vec3::new( 0.0, 0.5,  0.0)))
                .with_mesh(h_cube1)
                .with_bounding_radius(0.5 * f32::sqrt(3.0)),
        );
        level.spawn_entity(
            Components::new()
                .with_transform(Transform::from_position(Vec3::new(-2.0, 0.4, -1.0)))
                .with_mesh(h_cube2)
                .with_bounding_radius(0.4 * f32::sqrt(3.0)),
        );
        level.spawn_entity(
            Components::new()
                .with_transform(Transform::from_position(Vec3::new( 2.0, 0.3,  0.5)))
                .with_mesh(h_cube3)
                .with_bounding_radius(0.3 * f32::sqrt(3.0)),
        );
        level.spawn_entity(
            Components::new()
                .with_transform(Transform::from_position(Vec3::new(0.0, 0.0, 0.0)))
                .with_mesh(h_ground)
                .with_bounding_radius(5.0 * f32::sqrt(2.0)), // half_extent=5, plane diagonal
        );

        // Spawn point lights (matched to render_v2_basic positions)
        level.spawn_entity(
            Components::new()
                .with_transform(Transform::from_position(Vec3::new( 0.0, 2.2, 0.0)))
                .with_light(LightData::Point {
                    color: [1.0, 0.55, 0.15], intensity: 6.0, range: 5.0,
                }),
        );
        level.spawn_entity(
            Components::new()
                .with_transform(Transform::from_position(Vec3::new(-3.5, 2.0, -1.5)))
                .with_light(LightData::Point {
                    color: [0.25, 0.5, 1.0], intensity: 5.0, range: 6.0,
                }),
        );
        level.spawn_entity(
            Components::new()
                .with_transform(Transform::from_position(Vec3::new(3.5, 1.5, 1.5)))
                .with_light(LightData::Point {
                    color: [1.0, 0.3, 0.5], intensity: 5.0, range: 6.0,
                }),
        );

        // Force-activate all chunks (small demo level — no streaming needed).
        level.activate_all_chunks();

        // ── Register the main game camera ─────────────────────────────────────
        //
        // CameraKind::GameCamera renders in SimulationMode::Game.
        // Switch to SimulationMode::Editor (Tab) to hide it and show that
        // the editor-camera filtering works.
        let main_camera_id = stratum.register_camera(StratumCamera {
            id:            CameraId::PLACEHOLDER, // overwritten by registry
            kind:          CameraKind::GameCamera { tag: "main".into() },
            position:      Vec3::new(0.0, 2.5, 7.0),
            yaw:           0.0,
            pitch:         -0.2,
            projection:    Projection::perspective(
                               std::f32::consts::FRAC_PI_4, 0.1, 200.0,
                           ),
            render_target: RenderTargetHandle::PrimarySurface,
            viewport:      Viewport::full(),
            priority:      0,
            active:        true,
        });

        // ── Register an editor camera (only renders in Editor mode) ───────────
        //
        // Demonstrates multi-camera registry. In Editor mode (press Tab),
        // this camera takes over. It starts at the same position / angle.
        stratum.register_camera(StratumCamera {
            id:            CameraId::PLACEHOLDER,
            kind:          CameraKind::EditorPerspective,
            position:      Vec3::new(0.0, 2.5, 7.0),
            yaw:           0.0,
            pitch:         -0.2,
            projection:    Projection::perspective(
                               std::f32::consts::FRAC_PI_4, 0.1, 200.0,
                           ),
            render_target: RenderTargetHandle::PrimarySurface,
            viewport:      Viewport::full(),
            priority:      0,
            active:        true,
        });

        self.state = Some(AppState {
            window,
            surface,
            device,
            surface_format,
            stratum,
            integration,
            main_camera_id,
            last_frame:     std::time::Instant::now(),
            keys:           HashSet::new(),
            cursor_grabbed: false,
            mouse_delta:    (0.0, 0.0),
        });
    }

    // ── Events ────────────────────────────────────────────────────────────────

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id:        WindowId,
        event:      WindowEvent,
    ) {
        let Some(state) = &mut self.state else { return };

        match event {
            // ── Exit ──────────────────────────────────────────────────────────
            WindowEvent::CloseRequested => {
                log::info!("Close requested — exiting");
                event_loop.exit();
            }

            WindowEvent::KeyboardInput {
                event: KeyEvent {
                    state:        ElementState::Pressed,
                    physical_key: PhysicalKey::Code(KeyCode::Escape),
                    ..
                },
                ..
            } => {
                if state.cursor_grabbed {
                    state.cursor_grabbed = false;
                    let _ = state.window.set_cursor_grab(CursorGrabMode::None);
                    state.window.set_cursor_visible(true);
                } else {
                    event_loop.exit();
                }
            }

            // ── Tab — toggle Editor / Game mode ───────────────────────────────
            WindowEvent::KeyboardInput {
                event: KeyEvent {
                    state:        ElementState::Pressed,
                    physical_key: PhysicalKey::Code(KeyCode::Tab),
                    ..
                },
                ..
            } => {
                state.stratum.toggle_mode();
                log::info!("Mode → {:?}", state.stratum.mode());
            }

            // ── Keyboard held state ───────────────────────────────────────────
            WindowEvent::KeyboardInput {
                event: KeyEvent {
                    state:        ks,
                    physical_key: PhysicalKey::Code(key),
                    ..
                },
                ..
            } => match ks {
                ElementState::Pressed  => { state.keys.insert(key); }
                ElementState::Released => { state.keys.remove(&key); }
            },

            // ── Mouse button — grab cursor on click ───────────────────────────
            WindowEvent::MouseInput {
                state:  ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                if !state.cursor_grabbed {
                    let grabbed = state.window
                        .set_cursor_grab(CursorGrabMode::Confined)
                        .or_else(|_| state.window.set_cursor_grab(CursorGrabMode::Locked))
                        .is_ok();
                    if grabbed {
                        state.window.set_cursor_visible(false);
                        state.cursor_grabbed = true;
                    }
                }
            }

            // ── Resize ────────────────────────────────────────────────────────
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                state.surface.configure(&state.device, &wgpu::SurfaceConfiguration {
                    usage:                         wgpu::TextureUsages::RENDER_ATTACHMENT,
                    format:                        state.surface_format,
                    width:                         size.width,
                    height:                        size.height,
                    present_mode:                  wgpu::PresentMode::Fifo,
                    alpha_mode:                    wgpu::CompositeAlphaMode::Auto,
                    view_formats:                  vec![],
                    desired_maximum_frame_latency: 2,
                });
                state.integration.resize(size.width, size.height);
            }

            // ── Draw ──────────────────────────────────────────────────────────
            WindowEvent::RedrawRequested => {
                let now = std::time::Instant::now();
                let dt  = (now - state.last_frame).as_secs_f32();
                state.last_frame = now;
                state.render(dt);
                state.window.request_redraw();
            }

            _ => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _id:         winit::event::DeviceId,
        event:       DeviceEvent,
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
        if let Some(state) = &self.state {
            state.window.request_redraw();
        }
    }
}

// ── Per-frame render logic ────────────────────────────────────────────────────

impl AppState {
    fn render(&mut self, dt: f32) {
        const SPEED:     f32 = 5.0;
        const LOOK_SENS: f32 = 0.002;

        // ── Update main camera from input ─────────────────────────────────────
        {
            let cam = self.stratum.cameras_mut()
                .get_mut(self.main_camera_id)
                .expect("main camera unregistered");

            // Mouse look
            cam.yaw   += self.mouse_delta.0 * LOOK_SENS;
            cam.pitch  = (cam.pitch - self.mouse_delta.1 * LOOK_SENS).clamp(-1.5, 1.5);

            // WASD / Space / Shift movement
            let forward = cam.forward();
            let right   = cam.right();
            let keys    = &self.keys;
            if keys.contains(&KeyCode::KeyW)     { cam.position += forward * SPEED * dt; }
            if keys.contains(&KeyCode::KeyS)     { cam.position -= forward * SPEED * dt; }
            if keys.contains(&KeyCode::KeyA)     { cam.position -= right   * SPEED * dt; }
            if keys.contains(&KeyCode::KeyD)     { cam.position += right   * SPEED * dt; }
            if keys.contains(&KeyCode::Space)    { cam.position += Vec3::Y * SPEED * dt; }
            if keys.contains(&KeyCode::ShiftLeft){ cam.position -= Vec3::Y * SPEED * dt; }
        }
        self.mouse_delta = (0.0, 0.0);

        // ── Advance Stratum ───────────────────────────────────────────────────
        self.stratum.tick(dt);

        let size = self.window.inner_size();
        let time = self.integration.renderer().frame_count() as f32 * 0.016;

        // ── Produce render views ──────────────────────────────────────────────
        //
        // In Game mode  → only the GameCamera view is returned.
        // In Editor mode → only the EditorPerspective view is returned.
        // Pressing Tab switches live between the two.
        let views = self.stratum.build_views(size.width, size.height, time);

        if views.is_empty() {
            // Mode has no active cameras — nothing to render this frame.
            log::debug!("No active cameras for mode {:?}", self.stratum.mode());
            return;
        }

        // ── Acquire swapchain image ───────────────────────────────────────────
        let output = match self.surface.get_current_texture() {
            Ok(t)  => t,
            Err(e) => { log::warn!("Surface error: {:?}", e); return; }
        };
        let surface_view = output.texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // ── Submit to Helio via the integration bridge ────────────────────────
        //
        // Split borrows: `level` borrows from `self.stratum`;
        //                `self.integration` is a distinct field — borrow-safe.
        let level = self.stratum.active_level()
            .expect("active level must exist when there are render views");

        if let Err(e) = self.integration.submit_frame(
            &views, level, &surface_view, dt,
        ) {
            log::error!("Render error: {:?}", e);
        }

        output.present();
    }
}
