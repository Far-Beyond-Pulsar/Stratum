use stratum::{ChunkMetadata, WorldPartitionManager};
use stratum_helio::StratumRenderer;
use glam::{Quat, Vec3};
use winit::{
    event::*,
    event_loop::EventLoop,
    keyboard::{KeyCode, PhysicalKey},
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

/// Simple camera controller for flying through the world
struct CameraController {
    position: Vec3,
    yaw: f32,
    pitch: f32,
    speed: f32,
    mouse_sensitivity: f32,
    mouse_captured: bool,
    forward: bool,
    backward: bool,
    left: bool,
    right: bool,
    up: bool,
    down: bool,
}

impl CameraController {
    fn new(position: Vec3) -> Self {
        Self {
            position,
            yaw: 0.0,
            pitch: 0.0,
            speed: 50.0,
            mouse_sensitivity: 0.002,
            mouse_captured: false,
            forward: false,
            backward: false,
            left: false,
            right: false,
            up: false,
            down: false,
        }
    }

    fn process_keyboard(&mut self, key: KeyCode, state: ElementState) {
        let pressed = state == ElementState::Pressed;
        match key {
            KeyCode::KeyW => self.forward = pressed,
            KeyCode::KeyS => self.backward = pressed,
            KeyCode::KeyA => self.right = pressed,  // Swapped: A now moves right
            KeyCode::KeyD => self.left = pressed,   // Swapped: D now moves left
            KeyCode::Space => self.up = pressed,
            KeyCode::ShiftLeft => self.down = pressed,
            _ => {}
        }
    }

    fn process_mouse_motion(&mut self, delta_x: f64, delta_y: f64) {
        if !self.mouse_captured {
            return;
        }

        self.yaw -= (delta_x as f32) * self.mouse_sensitivity;
        self.pitch -= (delta_y as f32) * self.mouse_sensitivity;

        // Clamp pitch to prevent gimbal lock
        self.pitch = self.pitch.clamp(-std::f32::consts::FRAC_PI_2 + 0.01, std::f32::consts::FRAC_PI_2 - 0.01);
    }

    fn update(&mut self, dt: f32) -> (Vec3, Quat) {
        let forward = Vec3::new(
            self.yaw.cos() * self.pitch.cos(),
            self.pitch.sin(),
            self.yaw.sin() * self.pitch.cos(),
        ).normalize();
        let right = Vec3::new(self.yaw.sin(), 0.0, -self.yaw.cos()).normalize();

        let mut velocity = Vec3::ZERO;
        if self.forward {
            velocity += forward;
        }
        if self.backward {
            velocity -= forward;
        }
        if self.right {
            velocity += right;
        }
        if self.left {
            velocity -= right;
        }
        if self.up {
            velocity += Vec3::Y;
        }
        if self.down {
            velocity -= Vec3::Y;
        }

        if velocity.length_squared() > 0.0 {
            velocity = velocity.normalize() * self.speed * dt;
            self.position += velocity;
        }

        let rotation = Quat::from_euler(glam::EulerRot::YXZ, self.yaw, self.pitch, 0.0);
        (self.position, rotation)
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    log::info!("=== Stratum Visual World Partition Example ===");
    log::info!("Controls: WASD to move, Space/Shift for up/down, Left Click to capture mouse, ESC to release cursor/exit");

    let event_loop = EventLoop::new().unwrap();
    let window_attrs = winit::window::Window::default_attributes()
        .with_title("Stratum World Partition - Visual Demo")
        .with_inner_size(winit::dpi::PhysicalSize::new(1280, 720));
    let window = Arc::new(event_loop.create_window(window_attrs).unwrap());

    // Initialize WGPU
    let instance = wgpu::Instance::default();
    let surface = instance.create_surface(window.clone()).unwrap();

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: Some(&surface),
        force_fallback_adapter: false,
    }))
    .unwrap();

    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("Main Device"),
            required_features: StratumRenderer::required_features(adapter.features()),
            required_limits: StratumRenderer::required_limits(adapter.limits()),
            ..Default::default()
        },
    ))
    .unwrap();

    let device = Arc::new(device);
    let queue = Arc::new(queue);

    let size = window.inner_size();
    let mut surface_config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: surface.get_capabilities(&adapter).formats[0],
        width: size.width,
        height: size.height,
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: wgpu::CompositeAlphaMode::Auto,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    surface.configure(&device, &surface_config);

    // Create Stratum renderer (wraps Helio)
    let mut renderer = StratumRenderer::new(
        device.clone(),
        queue.clone(),
        size.width,
        size.height,
        surface_config.format,
    );
    renderer.set_clear_color([0.12, 0.15, 0.20, 1.0]);
    renderer.set_ambient([0.3, 0.3, 0.4], 0.2);

    // Initialize world partition manager
    let mut world = WorldPartitionManager::new(
        50.0,   // 50m chunks
        200.0,  // Visible within 200m
        400.0,  // Preload within 400m
        600.0,  // Unload beyond 600m
    );

    // Create a large grid of chunks
    log::info!("Creating world chunk grid...");
    let grid_size = 20;
    let mut chunk_count = 0;

    for x in -grid_size..=grid_size {
        for y in -2..=2 {
            for z in -grid_size..=grid_size {
                let metadata = ChunkMetadata::new(x, y, z, 0, 50.0);
                let file_path = PathBuf::from(format!(
                    "world_data/chunks/chunk_{}_{}_{}.bin",
                    x, y, z
                ));
                world.upsert_chunk(metadata, file_path);
                chunk_count += 1;
            }
        }
    }

    // Mark all chunks as loaded immediately (demo mode - no actual files)
    for chunk in world.chunks.values_mut() {
        chunk.mark_loaded();
    }

    log::info!("Created {} chunks (demo mode - all marked as loaded)", chunk_count);

    // Create camera
    let mut camera_controller = CameraController::new(Vec3::new(0.0, 50.0, 0.0));
    let camera_id = world.register_camera(camera_controller.position, Quat::IDENTITY);

    let mut last_frame = Instant::now();
    let mut frame_count = 0u64;
    let mut fps_timer = Instant::now();

    let _ = event_loop.run(move |event, elwt| {
        match event {
            Event::WindowEvent { event, window_id } if window_id == window.id() => {
                match event {
                    WindowEvent::CloseRequested => {
                        log::info!("Window close requested");
                        elwt.exit();
                    }
                    WindowEvent::KeyboardInput { event: key_event, .. } => {
                        if let PhysicalKey::Code(keycode) = key_event.physical_key {
                            if keycode == KeyCode::Escape && key_event.state == ElementState::Pressed {
                                // Release cursor on ESC
                                camera_controller.mouse_captured = false;
                                let _ = window.set_cursor_grab(winit::window::CursorGrabMode::None);
                                window.set_cursor_visible(true);
                            }
                            camera_controller.process_keyboard(keycode, key_event.state);
                        }
                    }
                    WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                        // Capture cursor on left click
                        camera_controller.mouse_captured = true;
                        let _ = window.set_cursor_grab(winit::window::CursorGrabMode::Confined)
                            .or_else(|_| window.set_cursor_grab(winit::window::CursorGrabMode::Locked));
                        window.set_cursor_visible(false);
                    }
                    WindowEvent::Resized(new_size) => {
                        if new_size.width > 0 && new_size.height > 0 {
                            surface_config.width = new_size.width;
                            surface_config.height = new_size.height;
                            surface.configure(&device, &surface_config);
                            renderer.set_render_size(new_size.width, new_size.height);
                        }
                    }
                    WindowEvent::RedrawRequested => {
                        let now = Instant::now();
                        let dt = (now - last_frame).as_secs_f32();
                        last_frame = now;

                        // Update camera
                        let (pos, rot) = camera_controller.update(dt);
                        world.update_camera_transform(camera_id, pos, rot);

                        // Update world partition system
                        world.tick(dt);

                        // Render frame - Stratum handles everything!
                        let frame = match surface.get_current_texture() {
                            Ok(f) => f,
                            Err(e) => {
                                log::warn!("Surface error: {:?}", e);
                                return;
                            }
                        };
                        let view = frame.texture.create_view(&Default::default());

                        // Get the camera from the world and render
                        if let Some(camera) = world.registry.get_camera(camera_id) {
                            if let Err(e) = renderer.render_world(&world, camera, &view) {
                                log::error!("Render error: {}", e);
                            }
                        }

                        frame.present();

                        // FPS counter and metrics
                        frame_count += 1;
                        if fps_timer.elapsed().as_secs() >= 1 {
                            let metrics = world.metrics();
                            let visible_chunks = world.visible_chunk_ids().len();

                            log::info!(
                                "FPS: {} | Pos: ({:.1}, {:.1}, {:.1}) | Visible: {} chunks",
                                frame_count,
                                pos.x, pos.y, pos.z,
                                visible_chunks
                            );
                            log::info!("  World: {}", metrics.format_summary());

                            frame_count = 0;
                            fps_timer = Instant::now();
                        }

                        window.request_redraw();
                    }
                    _ => {}
                }
            }
            Event::DeviceEvent { event: DeviceEvent::MouseMotion { delta }, .. } => {
                camera_controller.process_mouse_motion(delta.0, delta.1);
            }
            Event::AboutToWait => {
                window.request_redraw();
            }
            _ => {}
        }
    });
}
