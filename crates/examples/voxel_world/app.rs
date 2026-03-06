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
    features::{BloomFeature, FeatureRegistry, LightingFeature, ShadowsFeature, BillboardsFeature, RadianceCascadesFeature, BillboardInstance},
    passes::AntiAliasingMode,
};

use stratum::{
    CameraId, CameraKind, Components, EntityId, LevelStreamer, LightData, Projection,
    RenderTargetHandle, SimulationMode, SkyAtmosphereData, SkylightData,
    Stratum, StratumCamera, Transform, Viewport,
};
use stratum_helio::{AssetRegistry, HelioIntegration};

use crate::camera::*;
use crate::chunks::VoxelChunkManager;
use crate::materials::MaterialPalette;
use crate::terrain::*;
use crate::player::{Player, update_player, PLAYER_WALK_SPEED};

// ── Asset loading ───────────────────────────────────────────────────────────

fn load_sprite(path: &str) -> (Vec<u8>, u32, u32) {
    let asset_bytes: Option<&'static [u8]> = match path {
        "probe.png"     => Some(include_bytes!("../../../assets/probe.png")),
        "spotlight.png" => Some(include_bytes!("../../../assets/spotlight.png")),
        _ => None,
    };

    let img = asset_bytes
        .and_then(|bytes| image::load_from_memory(bytes).ok())
        .unwrap_or_else(|| {
            log::warn!("Could not decode embedded '{}', using 1x1 white fallback", path);
            let mut px = image::RgbaImage::new(1, 1);
            px.put_pixel(0, 0, image::Rgba([255, 255, 255, 255]));
            image::DynamicImage::ImageRgba8(px)
        })
        .into_rgba8();
    let (w, h) = img.dimensions();
    (img.into_raw(), w, h)
}

// ── Level directory ─────────────────────────────────────────────────────────

fn level_dir() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    exe_dir.join("levels").join("voxel_world")
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
        println!("Invalidating level cache in '{}'", dir.display());
        std::fs::create_dir_all(dir).expect("create level dir");
        std::fs::write(&key_path, CACHE_KEY).expect("write cache key");
        log::trace!("Level cache invalidated — regenerating chunks");
    }
}

// ── App ─────────────────────────────────────────────────────────────────────

pub struct App {
    state: Option<AppState>,
    no_fs: bool,
}

impl App {
    pub fn new(no_fs: bool) -> Self { Self { state: None, no_fs } }
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
    sky_entity_id:  EntityId,
    chunks:         VoxelChunkManager,
    streamer:       LevelStreamer,
    palette:        MaterialPalette,
    player:         Player,         // Game mode player state
    last_frame:     std::time::Instant,
    keys:           HashSet<KeyCode>,
    cursor_grabbed: bool,
    mouse_delta:    (f32, f32),
    time:           f32,
    frame_count:    u32,
    fps_acc:        f32,
    probe_vis:      bool,      // Digit3: toggle RC probe visualization
    rc_debug_bound: bool,      // 'r': toggle RC bounds visualization
}

// ── ApplicationHandler ─────────────────────────────────────────────────────

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() { return; }

        if self.no_fs {
            log::trace!("No-FS mode active: skipping level cache directory checks");
        } else {
            ensure_cache_valid(&level_dir());
        }

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

        let (probe_rgba, probe_w, probe_h) = load_sprite("probe.png");

        let features = FeatureRegistry::builder()
            .with_feature(LightingFeature::new())
            .with_feature(ShadowsFeature::new().with_atlas_size(2048).with_max_lights(4))
            .with_feature(BloomFeature::new().with_intensity(0.1).with_threshold(2.0))
            .with_feature(RadianceCascadesFeature::new().with_camera_follow([40.0, 20.0, 40.0]))
            .with_feature(BillboardsFeature::new()
                .with_sprite(probe_rgba, probe_w, probe_h)
                .with_max_instances(8192))
            .build();

        let renderer = Renderer::new(
            device.clone(), queue.clone(),
            RendererConfig::new(size.width, size.height, fmt, features)
                .with_aa(AntiAliasingMode::Taa)
                .with_ssao(),
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
                        intensity: 5.0,
                    }),
            );
        }

        // Skylight — atmospheric sky + sky-driven ambient
        let sky_entity_id = {
            let level = stratum.level_mut(level_id).unwrap();
            level.spawn_entity(
                Components::new()
                    .with_transform(Transform::default())
                    .with_sky_atmosphere(SkyAtmosphereData::new())
                    .with_skylight(SkylightData::new().with_intensity(0.8)),
            )
        };

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
        let chunks   = VoxelChunkManager::new(level_dir(), !self.no_fs);
        
        // Initialize player at spawn location
        let player = Player::spawn_at_surface(4.0, -12.0);

        self.state = Some(AppState {
            window, surface, device, queue, surface_format: fmt,
            stratum, integration, main_cam_id, sky_entity_id,
            chunks, streamer, palette, player,
            last_frame:     std::time::Instant::now(),
            keys:           HashSet::new(),
            cursor_grabbed: false,
            mouse_delta:    (0.0, 0.0),
            time:           0.0,
            frame_count:    0,
            fps_acc:        0.0,
            probe_vis:      false,
            rc_debug_bound: false,
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
                let is_game = matches!(state.stratum.mode(), SimulationMode::Game);
                
                // Switch camera kind based on mode
                if let Some(cam) = state.stratum.cameras_mut().get_mut(state.main_cam_id) {
                    cam.kind = if is_game {
                        CameraKind::GameCamera { tag: "main".into() }
                    } else {
                        CameraKind::EditorPerspective
                    };
                    
                    // When entering game mode, sync player to current camera position
                    if is_game {
                        state.player.position = cam.position - Vec3::new(0.0, 1.6, 0.0);
                        state.player.yaw = cam.yaw;
                        state.player.pitch = cam.pitch;
                        state.player.velocity = Vec3::ZERO;
                    }
                }
                
                log::info!("Mode → {:?}", state.stratum.mode());
            }

            WindowEvent::KeyboardInput { event: KeyEvent {
                state: ElementState::Pressed,
                physical_key: PhysicalKey::Code(KeyCode::Digit3), ..
            }, .. } => {
                state.probe_vis = !state.probe_vis;
                log::trace!("RC Probe Visualization → {}", if state.probe_vis { "ON" } else { "OFF" });
            }

            WindowEvent::KeyboardInput { event: KeyEvent {
                state: ElementState::Pressed,
                physical_key: PhysicalKey::Code(KeyCode::KeyR), ..
            }, .. } => {
                state.rc_debug_bound = !state.rc_debug_bound;
                log::trace!("RC Debug Bounds → {}", if state.rc_debug_bound { "ON" } else { "OFF" });
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

// ── RC probe visualization ──────────────────────────────────────────────────

/// Build billboard instances at every cascade-0 probe centre.
/// Grid dimensions and snap logic match `RadianceCascadesFeature`.
fn get_rc_probe_grid(camera_pos: Vec3) -> Vec<BillboardInstance> {
    const RC_HALF: [f32; 3] = [40.0, 20.0, 40.0]; // must match with_camera_follow() args
    const PROBE_DIM: u32 = 16;                     // cascade-0 probe count per axis

    // Cell size per axis (world-space metres between neighbouring probes).
    let cell_x = (RC_HALF[0] * 2.0) / PROBE_DIM as f32; // 5.0 m
    let cell_y = (RC_HALF[1] * 2.0) / PROBE_DIM as f32; // 2.5 m
    let cell_z = (RC_HALF[2] * 2.0) / PROBE_DIM as f32; // 5.0 m

    // Snap origin to probe grid (same as RC feature) so the visualisation
    // stays locked to the actual probe positions.
    let anchor_x = (camera_pos.x / cell_x).round() * cell_x;
    let anchor_z = (camera_pos.z / cell_z).round() * cell_z;
    let anchor_y = camera_pos.y;

    let min_x = anchor_x - RC_HALF[0];
    let min_y = anchor_y - RC_HALF[1];
    let min_z = anchor_z - RC_HALF[2];

    // Billboard size: ~10% of the smallest cell dimension so probes are
    // clearly visible without overlapping.
    let probe_size = cell_y * 0.2; // 0.5 m
    let size = [probe_size, probe_size];
    let color = [0.0, 1.0, 1.0, 0.8]; // cyan

    let cap = (PROBE_DIM * PROBE_DIM * PROBE_DIM) as usize;
    let mut probes = Vec::with_capacity(cap);

    for px in 0..PROBE_DIM {
        for py in 0..PROBE_DIM {
            for pz in 0..PROBE_DIM {
                let x = min_x + (px as f32 + 0.5) * cell_x;
                let y = min_y + (py as f32 + 0.5) * cell_y;
                let z = min_z + (pz as f32 + 0.5) * cell_z;
                probes.push(
                    BillboardInstance::new([x, y, z], size)
                        .with_color(color)
                );
            }
        }
    }

    probes
}

/// Compute RC bounds using camera-follow logic (matches RadianceCascadesFeature).
fn get_rc_bounds(camera_pos: Vec3) -> (Vec3, Vec3) {
    const RC_HALF_EXTENTS: [f32; 3] = [40.0, 20.0, 40.0];  // Must match with_camera_follow() args
    const PROBE_DIM: f32 = 16.0;  // CASCADE 0 probe grid dimension
    
    let hx = RC_HALF_EXTENTS[0].max(0.01);
    let hy = RC_HALF_EXTENTS[1].max(0.01);
    let hz = RC_HALF_EXTENTS[2].max(0.01);

    // Snap to cascade-0 probe cell size to keep GI stable while moving.
    let cell_x = (hx * 2.0) / PROBE_DIM;
    let cell_z = (hz * 2.0) / PROBE_DIM;
    let anchor_x = (camera_pos.x / cell_x).round() * cell_x;
    let anchor_z = (camera_pos.z / cell_z).round() * cell_z;
    let anchor_y = camera_pos.y;

    let min = Vec3::new(anchor_x - hx, anchor_y - hy, anchor_z - hz);
    let max = Vec3::new(anchor_x + hx, anchor_y + hy, anchor_z + hz);
    
    (min, max)
}

// ── Per-frame logic ─────────────────────────────────────────────────────────

impl AppState {
    fn render(&mut self, dt: f32) {
        self.time       += dt;
        self.frame_count += 1;
        self.fps_acc    += dt;

        // Camera/player movement based on mode
        let is_game_mode = matches!(self.stratum.mode(), SimulationMode::Game);
        
        if is_game_mode {
            // ── Game mode: player physics ──
            
            // Apply mouse look to player
            self.player.yaw += self.mouse_delta.0 * LOOK_SENS;
            self.player.pitch = (self.player.pitch - self.mouse_delta.1 * LOOK_SENS).clamp(-1.5, 1.5);
            
            // Run player physics with collision
            update_player(&mut self.player, &self.keys, &self.chunks, dt, self.time);
            
            // Sync camera to player eye position
            if let Some(cam) = self.stratum.cameras_mut().get_mut(self.main_cam_id) {
                cam.position = self.player.eye_pos();
                cam.yaw = self.player.yaw;
                cam.pitch = self.player.pitch;
            }
        } else {
            // ── Editor mode: free-fly camera ──
            
            let cam = self.stratum.cameras_mut()
                .get_mut(self.main_cam_id).expect("camera");
            cam.yaw += self.mouse_delta.0 * LOOK_SENS;
            cam.pitch = (cam.pitch - self.mouse_delta.1 * LOOK_SENS).clamp(-1.5, 1.5);
            let fwd = cam.forward();
            let right = cam.right();
            if self.keys.contains(&KeyCode::KeyW) { cam.position += fwd * CAM_SPEED * dt; }
            if self.keys.contains(&KeyCode::KeyS) { cam.position -= fwd * CAM_SPEED * dt; }
            if self.keys.contains(&KeyCode::KeyA) { cam.position -= right * CAM_SPEED * dt; }
            if self.keys.contains(&KeyCode::KeyD) { cam.position += right * CAM_SPEED * dt; }
            if self.keys.contains(&KeyCode::Space) { cam.position += Vec3::Y * CAM_SPEED * dt; }
            if self.keys.contains(&KeyCode::ShiftLeft) { cam.position -= Vec3::Y * CAM_SPEED * dt; }
        }
        self.mouse_delta = (0.0, 0.0);

        let cam_pos = self.stratum.cameras_mut()
            .get_mut(self.main_cam_id)
            .map(|c| c.position)
            .unwrap_or(Vec3::ZERO);

        // Update sky entity position to camera (camera-relative, always visible)
        {
            let level = self.stratum.active_level_mut().expect("level");
            if let Some(sky) = level.entities_mut().get_mut(self.sky_entity_id) {
                if let Some(transform) = &mut sky.transform {
                    transform.position = cam_pos;
                }
            }
        }

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

        // Inject RC probe billboards BEFORE frame submission so they are
        // merged into the scene by HelioIntegration.
        if self.probe_vis {
            self.integration.set_extra_billboards(get_rc_probe_grid(cam_pos));
        } else {
            self.integration.clear_extra_billboards();
        }

        // RC bounds debug box (wireframe, via debug draw)
        if self.rc_debug_bound {
            let (min, max) = get_rc_bounds(cam_pos);
            let center = (min + max) * 0.5;
            let half_extents = (max - min) * 0.5;
            self.integration.renderer_mut().debug_box(
                center, half_extents, glam::Quat::IDENTITY,
                [1.0, 1.0, 0.0, 0.8], 0.05,
            );
        }

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
