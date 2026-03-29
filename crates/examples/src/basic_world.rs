use glam::{Quat, Vec3};
use std::{fs, path::PathBuf, sync::Arc, time::{Duration, Instant}};

use wgpu::{Backends, DeviceDescriptor, InstanceDescriptor, PowerPreference, SurfaceConfiguration, Features, Limits};
use winit::{
    application::{ApplicationHandler, ActiveEventLoop},
    dpi::LogicalSize,
    event::{ElementState, Event, KeyEvent, WindowEvent},
    event_loop::EventLoop,
    keyboard::KeyCode,
    window::{Window, WindowAttributes, WindowId},
};

use stratum::{ChunkData, ChunkMetadata, WorldPartitionManager, CameraId};

fn create_chunk_file(path: &PathBuf, chunk_metadata: &ChunkMetadata) {
    let chunk_data = ChunkData::new(chunk_metadata.clone());
    let bytes = chunk_data.serialize(true).expect("serialize chunk");
    fs::create_dir_all(path.parent().unwrap()).expect("create chunk dir");
    fs::write(path, bytes).expect("write chunk file");
}

struct AppState {
    window: Option<Arc<Window>>,
    surface: Option<wgpu::Surface<'static>>,
    device: Option<Arc<wgpu::Device>>,
    queue: Option<Arc<wgpu::Queue>>,
    config: Option<SurfaceConfiguration>,

    manager: WorldPartitionManager,
    camera_id: Option<CameraId>,

    frame: u64,
    last_title_update: Instant,
    frame_count: u64,
    fps_start: Instant,
    window_id: Option<WindowId>,
}

impl AppState {
    fn new() -> Self {
        Self {
            window: None,
            surface: None,
            device: None,
            queue: None,
            config: None,
            manager: WorldPartitionManager::new(16.0, 40.0, 80.0, 120.0),
            camera_id: None,
            frame: 0,
            last_title_update: Instant::now(),
            frame_count: 0,
            fps_start: Instant::now(),
            window_id: None,
        }
    }

    fn setup_world(&mut self) {
        let base_dir = std::env::temp_dir().join("stratum_world_partition_demo");
        let _ = fs::remove_dir_all(&base_dir);
        for x in -2..=2 {
            for y in -2..=2 {
                let metadata = ChunkMetadata::new(x, y, 0, 0, 16.0);
                let path = base_dir.join(format!("chunk_{}_{}.bin", x, y));
                create_chunk_file(&path, &metadata);
                self.manager.upsert_chunk(metadata, path);
            }
        }

        let cam_id = self.manager.register_camera(Vec3::new(0.0, 8.0, 0.0), Quat::IDENTITY);
        self.camera_id = Some(cam_id);
    }

    fn maybe_resize_surface(&mut self, size: winit::dpi::PhysicalSize<u32>) {
        if let (Some(ref surface), Some(ref device), Some(ref mut config)) = (
            &self.surface,
            &self.device,
            &mut self.config,
        ) {
            if size.width > 0 && size.height > 0 {
                config.width = size.width;
                config.height = size.height;
                surface.configure(device, config);
            }
        }
    }

    fn render_frame(&mut self) {
        if self.surface.is_none() || self.device.is_none() || self.queue.is_none() || self.config.is_none() {
            return;
        }

        let surface = self.surface.as_ref().unwrap();
        let device = self.device.as_ref().unwrap();
        let queue = self.queue.as_ref().unwrap();
        let config = self.config.as_ref().unwrap();

        let output = match surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Lost) => {
                self.maybe_resize_surface(winit::dpi::PhysicalSize::new(config.width, config.height));
                return;
            }
            Err(wgpu::SurfaceError::OutOfMemory) => {
                std::process::exit(1);
            }
            Err(e) => {
                log::warn!("Surface error: {:?}", e);
                return;
            }
        };

        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("render_encoder"),
        });

        {
            let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.04, g: 0.08, b: 0.14, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
        }

        queue.submit(Some(encoder.finish()));
        output.present();
    }

    fn update_metrics(&mut self) {
        self.frame += 1;
        self.frame_count += 1;
        if self.last_title_update.elapsed() >= Duration::from_secs(1) {
            if let Some(ref window) = self.window {
                let metrics = self.manager.metrics();
                let fps = self.frame_count as f64 / self.fps_start.elapsed().as_secs_f64();
                let title = format!(
                    "Stratum World Partition | FPS: {fps:.1} | Loaded: {} | Evicted: {} | Pending: {} | Visible: {}",
                    metrics.chunks_loaded,
                    metrics.chunks_evicted,
                    metrics.pending_load_tasks,
                    self.manager.visible_chunk_ids().len(),
                );
                window.set_title(&title);
            }
            self.fps_start = Instant::now();
            self.frame_count = 0;
            self.last_title_update = Instant::now();
        }
    }
}

impl ApplicationHandler for AppState {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window = event_loop
            .create_window(
                WindowAttributes::default()
                    .with_title("Stratum World Partition Demo")
                    .with_inner_size(LogicalSize::new(1280.0, 720.0)),
            )
            .expect("Failed to create window");

        let arc_window = Arc::new(window);
        let window_id = arc_window.id();
        self.window = Some(arc_window.clone());
        self.window_id = Some(window_id);

        let instance = wgpu::Instance::new(InstanceDescriptor { backends: Backends::all(), ..Default::default() });
        let surface = unsafe { instance.create_surface(arc_window.as_ref()) }.expect("Failed to create surface");

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .expect("Failed to request adapter");

        let (device, queue) = pollster::block_on(adapter.request_device(&DeviceDescriptor {
            label: None,
            required_features: Features::empty(),
            required_limits: Limits::downlevel_defaults(),
            experimental_features: Default::default(),
            memory_hints: Default::default(),
            trace: Default::default(),
        }))
        .expect("Failed to request device");

        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats[0];
        let size = arc_window.inner_size();

        let config = SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: caps.present_modes[0],
            alpha_mode: caps.alpha_modes[0],
            desired_maximum_frame_latency: 2,
            view_formats: vec![],
        };

        surface.configure(&device, &config);

        self.surface = Some(unsafe { std::mem::transmute::<wgpu::Surface, wgpu::Surface<'static>>(surface) });
        self.device = Some(Arc::new(device));
        self.queue = Some(Arc::new(queue));
        self.config = Some(config);

        self.setup_world();
        event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        if Some(window_id) != self.window_id {
            return;
        }

        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                self.maybe_resize_surface(size);
            }
            WindowEvent::ScaleFactorChanged { inner_size_writer, .. } => {
                let size = self.window.as_ref().map(|w| w.inner_size()).unwrap_or_else(|| LogicalSize::new(1280.0, 720.0).to_physical(1.0));
                inner_size_writer.set_inner_size(size);
                self.maybe_resize_surface(size);
            }
            WindowEvent::KeyboardInput { event: KeyEvent { logical_key, state, .. }, .. } => {
                if logical_key == KeyCode::Escape && state == ElementState::Pressed {
                    event_loop.exit();
                }
            }
            WindowEvent::RedrawRequested => {
                self.render_frame();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(camera_id) = self.camera_id {
            let p = (self.frame as f32 * 0.02).sin() * 30.0;
            self.manager.update_camera_transform(camera_id, Vec3::new(p, 8.0, 0.0), Quat::IDENTITY);
            self.manager.tick(1.0 / 60.0);
            self.update_metrics();
            if let Some(ref window) = self.window {
                window.request_redraw();
            }
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        // cleanup if needed
    }
}

fn main() {
    env_logger::init();
    let mut app = AppState::new();
    let event_loop = EventLoop::new().expect("Failed to create event loop");
    event_loop.run_app(&mut app).expect("Event loop error");
}
