//! Application state, window event handling, and per-frame render loop.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use glam::Vec3;
use winit::{
    application::ApplicationHandler,
    event::*,
    event_loop::ActiveEventLoop,
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowId},
};

use helio_render_v2::{
    Renderer, RendererConfig,
    features::{BloomFeature, FeatureRegistry, LightingFeature, ShadowsFeature},
    passes::AntiAliasingMode,
};

use stratum::{
    CameraId, CameraKind, Components, LevelStreamer, LightData, Projection,
    RenderTargetHandle, SimulationMode, Stratum, StratumCamera, Transform, Viewport,
};
use stratum_helio::{AssetRegistry, HelioIntegration};

use crate::camera::*;
use crate::chunks::VoxelChunkManager;
use crate::materials::MaterialPalette;
use crate::terrain::*;

// ── Level directory ─────────────────────────────────────────────────────────

fn level_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("levels")
        .join("voxel_world")
}

/// Bump this key to wipe stale on-disk chunks after format changes.
const CACHE_KEY: &str = "voxel_world_v7_multibiome";

// ── Cache invalidation ─────────────────────────────────────────────────────

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

// ── App ─────────────────────────────────────────────────────────────────────

pub struct App {
    state: Option<AppState>,
}

impl App {
    pub fn new() -> Self { Self { state: None } }
}

pub struct AppState {
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
    palette:        MaterialPalette,
    last_frame:     std::time::Instant,
    keys:           HashSet<KeyCode>,
    cursor_grabbed: bool,
    mouse_delta:    (f32, f32),
    time:           f32,
    frame_count:    u32,
    fps_acc:        f32,
}

// ── ApplicationHandler ─────────────────────────────────────────────────────

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() { return; }

        ensure_cache_valid(&level_dir());

        let window = Arc::new(
            event_loop.create_window(
                Window::default_attributes()
                    .with_title("Stratum — Multi-Biome Voxel World")
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
            RendererConfig::new(size.width, size.height, fmt, features)
                .with_aa(AntiAliasingMode::Fxaa),
        ).expect("renderer");

        let mut integration = HelioIntegration::new(renderer, AssetRegistry::new());

        let palette = MaterialPalette::new(&mut integration);

        let mut stratum  = Stratum::new(SimulationMode::Editor);
        let level_id     = stratum.create_level("voxel_world", CHUNK_SIZE, ACTIVATION_RADIUS);
        stratum.level_mut(level_id).unwrap().activate_all_chunks();

        // Sun light
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

        // Camera
        let main_cam_id = stratum.register_camera(StratumCamera {
            id:            CameraId::PLACEHOLDER,
            kind:          CameraKind::EditorPerspective,
            position:      Vec3::new(4.0, 50.0, -12.0),
            yaw:           0.0,
            pitch:         -0.35,
            projection:    Projection::perspective(std::f32::consts::FRAC_PI_3, 0.1, 1000.0),
            render_target: RenderTargetHandle::PrimarySurface,
            viewport:      Viewport::full(),
            priority:      0,
            active:        true,
        });

        let streamer = LevelStreamer::new();
        let chunks   = VoxelChunkManager::new(level_dir());

        self.state = Some(AppState {
            window, surface, device, queue, surface_format: fmt,
            stratum, integration, main_cam_id,
            chunks, streamer, palette,
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

// ── Per-frame logic ─────────────────────────────────────────────────────────

impl AppState {
    fn render(&mut self, dt: f32) {
        self.time       += dt;
        self.frame_count += 1;
        self.fps_acc    += dt;

        // Camera movement
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

        // Drain stream events → pending queue → flush uploads
        let new_events: Vec<_> = self.streamer.poll_loaded().into_iter().collect();
        self.chunks.collect_events(new_events);
        {
            let level  = self.stratum.active_level_mut().expect("level");
            let assets = self.integration.assets_mut();
            self.chunks.flush_events(level, &self.device, assets, &self.palette);
            self.chunks.update(cam_pos, level, &self.streamer, assets);
        }

        self.stratum.tick(dt);

        // Re-activate all manager-loaded chunks
        {
            let level = self.stratum.active_level_mut().expect("level");
            for &coord in self.chunks.loaded.keys() {
                level.partition_mut().get_or_create(coord).activate();
            }
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
