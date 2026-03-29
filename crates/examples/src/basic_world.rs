use glam::{Quat, Vec3};
use log::info;
use std::{fs, path::PathBuf, time::{Duration, Instant}};

use wgpu::{Backends, PowerPreference};
use winit::{
    event::{ElementState, Event, KeyboardInput, VirtualKeyCode, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};

use stratum::{ChunkData, ChunkMetadata, WorldPartitionManager};

fn create_chunk_file(path: &PathBuf, chunk_metadata: &ChunkMetadata) {
    let chunk_data = ChunkData::new(chunk_metadata.clone());
    let bytes = chunk_data.serialize(true).expect("serialize chunk");
    fs::create_dir_all(path.parent().unwrap()).expect("create chunk dir");
    fs::write(path, bytes).expect("write chunk file");
}

fn main() {
    env_logger::init();

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Stratum World Partition Demo")
        .with_inner_size(winit::dpi::PhysicalSize::new(1280, 720))
        .build(&event_loop)
        .expect("Failed to create window");

    let instance = wgpu::Instance::new(Backends::all());
    let surface = unsafe { instance.create_surface(&window) }.expect("create surface");
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: Some(&surface),
    })).expect("Failed to request adapter");

    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: None,
            features: wgpu::Features::empty(),
            limits: wgpu::Limits::default(),
        },
        None,
    )).expect("Failed to request device");

    let surface_format = surface.get_supported_formats(&adapter)[0];
    let mut config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: surface_format,
        width: window.inner_size().width,
        height: window.inner_size().height,
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: wgpu::CompositeAlphaMode::Auto,
    };
    surface.configure(&device, &config);

    let base_dir = std::env::temp_dir().join("stratum_world_partition_demo");
    let _ = fs::remove_dir_all(&base_dir);

    let mut manager = WorldPartitionManager::new(16.0, 40.0, 80.0, 120.0);
    for x in -2..=2 {
        for y in -2..=2 {
            let metadata = ChunkMetadata::new(x, y, 0, 0, 16.0);
            let path = base_dir.join(format!("chunk_{}_{}.bin", x, y));
            create_chunk_file(&path, &metadata);
            manager.upsert_chunk(metadata, path);
        }
    }

    let camera_id = manager.register_camera(Vec3::new(0.0, 8.0, 0.0), Quat::IDENTITY);
    let mut frame: u64 = 0;
    let mut last_title_update = Instant::now();
    let mut fps_start = Instant::now();
    let mut frame_count = 0u64;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Poll;

        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => *control_flow = ControlFlow::Exit,
            Event::WindowEvent { event: WindowEvent::KeyboardInput { input, .. }, .. } => {
                if let KeyboardInput { virtual_keycode: Some(VirtualKeyCode::Escape), state: ElementState::Pressed, .. } = input {
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::WindowEvent { event: WindowEvent::Resized(size), .. } => {
                config.width = size.width.max(1);
                config.height = size.height.max(1);
                surface.configure(&device, &config);
            }
            Event::WindowEvent { event: WindowEvent::ScaleFactorChanged { new_inner_size, .. }, .. } => {
                config.width = new_inner_size.width.max(1);
                config.height = new_inner_size.height.max(1);
                surface.configure(&device, &config);
            }
            Event::MainEventsCleared => {
                let p = (frame as f32 * 0.02).sin() * 30.0;
                manager.update_camera_transform(camera_id, Vec3::new(p, 8.0, 0.0), Quat::IDENTITY);
                manager.tick(1.0 / 60.0);

                let output = match surface.get_current_texture() {
                    Ok(frame) => frame,
                    Err(wgpu::SurfaceError::Lost) => { surface.configure(&device, &config); return; }
                    Err(wgpu::SurfaceError::OutOfMemory) => { *control_flow = ControlFlow::Exit; return; }
                    Err(e) => { eprintln!("Surface error: {e:?}"); return; }
                };

                let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("render") });
                {
                    let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("clear_pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.04, g: 0.08, b: 0.14, a: 1.0 }),
                                store: true,
                            },
                        })],
                        depth_stencil_attachment: None,
                    });
                }
                queue.submit(Some(encoder.finish()));
                output.present();

                frame += 1;
                frame_count += 1;

                if last_title_update.elapsed() >= Duration::from_secs(1) {
                    let metrics = manager.metrics();
                    let fps = frame_count as f64 / fps_start.elapsed().as_secs_f64();
                    let title = format!(
                        "Stratum World Partition | FPS: {fps:.1} | Loaded: {} | Evicted: {} | Pending: {} | Visible: {}",
                        metrics.chunks_loaded,
                        metrics.chunks_evicted,
                        metrics.pending_load_tasks,
                        manager.visible_chunk_ids().len(),
                    );
                    window.set_title(&title);
                    fps_start = Instant::now();
                    frame_count = 0;
                    last_title_update = Instant::now();
                }
            }
            _ => {}
        }
    });
}
