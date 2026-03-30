//! Voxel world example with procedural terrain generation and chunk streaming.
//!
//! This demonstrates:
//! - Procedural voxel terrain using noise
//! - Face-culled voxel meshing
//! - Chunk streaming with WorldPartitionManager
//! - Material/texture management
//!
//! Controls:
//! - WASD: Move
//! - Space/Shift: Up/Down
//! - Mouse: Look around (click to capture)
//! - ESC: Release cursor/Exit

mod voxel_world {
    pub mod blocks;
    pub mod voxel_chunk;
    pub mod terrain;
    pub mod meshing;
}

use voxel_world::*;

use stratum::{ChunkMetadata, WorldPartitionManager};
use stratum_helio::{
    StratumRenderer, MeshUpload, PackedVertex, MeshId, MaterialId, ObjectId,
    ObjectDescriptor, SceneActor, GpuMaterial, SceneActorId,
};
use helio::GroupMask;
use glam::{Quat, Vec3, Mat4};
use winit::{
    event::*,
    event_loop::EventLoop,
    keyboard::{KeyCode, PhysicalKey},
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

/// Camera controller (reused from visual_world).
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
            speed: 25.0,
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
            KeyCode::KeyA => self.left = pressed,
            KeyCode::KeyD => self.right = pressed,
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
        self.pitch = self.pitch.clamp(-std::f32::consts::FRAC_PI_2 + 0.01, std::f32::consts::FRAC_PI_2 - 0.01);
    }

    fn update(&mut self, dt: f32) -> (Vec3, Quat) {
        let rotation = Quat::from_euler(glam::EulerRot::YXZ, self.yaw, self.pitch, 0.0);

        let horizontal_rotation = Quat::from_rotation_y(self.yaw);
        let forward = (horizontal_rotation * Vec3::NEG_Z).normalize();
        let right = (horizontal_rotation * Vec3::X).normalize();

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

        (self.position, rotation)
    }
}

/// Generate voxel chunks on demand.
struct VoxelChunkGenerator {
    /// Cache of generated chunks.
    generated: HashMap<(i32, i32, i32), voxel_chunk::VoxelChunk>,
}

impl VoxelChunkGenerator {
    fn new() -> Self {
        Self {
            generated: HashMap::new(),
        }
    }

    fn get_or_generate(&mut self, chunk_x: i32, chunk_y: i32, chunk_z: i32) -> &voxel_chunk::VoxelChunk {
        self.generated.entry((chunk_x, chunk_y, chunk_z)).or_insert_with(|| {
            terrain::generate_chunk(chunk_x, chunk_y, chunk_z)
        })
    }
}

/// Manages voxel chunk meshes in the Helio scene.
struct VoxelChunkMeshManager {
    /// Map of chunk coords to (MeshId, ObjectId).
    chunk_objects: HashMap<(i32, i32, i32), (MeshId, ObjectId)>,
    /// Default material for voxels.
    material_id: MaterialId,
}

impl VoxelChunkMeshManager {
    fn new(renderer: &mut StratumRenderer) -> Self {
        // Create a simple white material for voxels (stone-like appearance)
        let material = GpuMaterial {
            base_color: [0.7, 0.7, 0.7, 1.0],  // Light gray
            emissive: [0.0, 0.0, 0.0, 0.0],
            roughness_metallic: [0.9, 0.0, 1.5, 0.5],  // Very rough, non-metallic
            tex_base_color: GpuMaterial::NO_TEXTURE,
            tex_normal: GpuMaterial::NO_TEXTURE,
            tex_roughness: GpuMaterial::NO_TEXTURE,
            tex_emissive: GpuMaterial::NO_TEXTURE,
            tex_occlusion: GpuMaterial::NO_TEXTURE,
            workflow: 0,
            flags: 0,
            _pad: 0,
        };

        let material_id = renderer.scene_mut().insert_material(material);

        Self {
            chunk_objects: HashMap::new(),
            material_id,
        }
    }

    fn sync_chunks(
        &mut self,
        renderer: &mut StratumRenderer,
        voxel_generator: &mut VoxelChunkGenerator,
        world: &WorldPartitionManager,
    ) {
        // Get currently visible chunks
        let visible_coords: std::collections::HashSet<(i32, i32, i32)> = world
            .chunks
            .iter()
            .filter(|(_, chunk)| chunk.is_visible())
            .map(|(id, _)| {
                let (x, y, z, _) = id.to_coords();
                (x, y, z)
            })
            .collect();

        // Remove objects for chunks that are no longer visible
        let to_remove: Vec<(i32, i32, i32)> = self
            .chunk_objects
            .keys()
            .filter(|coord| !visible_coords.contains(coord))
            .copied()
            .collect();

        for coord in to_remove {
            if let Some((_mesh_id, object_id)) = self.chunk_objects.remove(&coord) {
                renderer.scene_mut().remove_object(object_id);
                log::debug!("Removed mesh for chunk {:?}", coord);
            }
        }

        // Add objects for newly visible chunks
        let mut added_count = 0;
        let mut skipped_empty = 0;

        for &coord in &visible_coords {
            if !self.chunk_objects.contains_key(&coord) {
                let before_count = self.chunk_objects.len();
                self.add_chunk_mesh(renderer, voxel_generator, coord);
                let after_count = self.chunk_objects.len();

                if after_count > before_count {
                    added_count += 1;
                } else {
                    skipped_empty += 1;
                }
            }
        }

        if added_count > 0 || skipped_empty > 0 {
            log::info!("Chunk sync: {} visible, {} with meshes, {} added, {} skipped (empty)",
                visible_coords.len(), self.chunk_objects.len(), added_count, skipped_empty);
        }
    }

    fn add_chunk_mesh(
        &mut self,
        renderer: &mut StratumRenderer,
        voxel_generator: &mut VoxelChunkGenerator,
        (chunk_x, chunk_y, chunk_z): (i32, i32, i32),
    ) {
        // Generate voxel chunk data
        let voxel_chunk = voxel_generator.get_or_generate(chunk_x, chunk_y, chunk_z);

        // Generate face-culled mesh
        let (voxel_vertices, indices) = meshing::generate_chunk_mesh(voxel_chunk);

        // Skip empty chunks
        if voxel_vertices.is_empty() {
            log::debug!("Chunk ({}, {}, {}) is empty, skipping mesh generation", chunk_x, chunk_y, chunk_z);
            return;
        }

        // Convert to PackedVertex format for Helio
        let vertices: Vec<PackedVertex> = voxel_vertices
            .iter()
            .map(|v| {
                // Calculate tangent from normal (simple approach for voxels)
                let tangent = if v.normal[1].abs() > 0.9 {
                    // For up/down faces, use X as tangent
                    [1.0, 0.0, 0.0]
                } else {
                    // For other faces, use Y as tangent
                    [0.0, 1.0, 0.0]
                };

                PackedVertex::from_components(
                    v.position,
                    v.normal,
                    v.uv,
                    tangent,
                    1.0, // bitangent sign
                )
            })
            .collect();

        // Upload mesh to Helio using SceneActor
        let mesh_upload = MeshUpload {
            vertices,
            indices,
        };

        let mesh_id = renderer.scene_mut()
            .insert_actor(SceneActor::mesh(mesh_upload))
            .as_mesh()
            .expect("Failed to insert mesh actor");

        // Transform places mesh at chunk corner (mesh goes from 0-16 in local space)
        let chunk_size = voxel_chunk::CHUNK_SIZE as f32;
        let chunk_pos = Vec3::new(
            chunk_x as f32 * chunk_size,
            chunk_y as f32 * chunk_size,
            chunk_z as f32 * chunk_size,
        );
        let transform = Mat4::from_translation(chunk_pos);

        // Bounding sphere center is at chunk center
        let half_size = chunk_size * 0.5;
        let radius = (chunk_size * chunk_size * 3.0).sqrt() * 0.5;

        // Create object in scene using SceneActor
        let object_desc = ObjectDescriptor {
            mesh: mesh_id,
            material: self.material_id,
            transform,
            bounds: [
                chunk_pos.x + half_size,
                chunk_pos.y + half_size,
                chunk_pos.z + half_size,
                radius
            ],
            flags: 0,
            groups: GroupMask::NONE,
        };

        let object_id = renderer.scene_mut()
            .insert_actor(SceneActor::object(object_desc))
            .as_object()
            .expect("Failed to insert object actor");

        self.chunk_objects.insert((chunk_x, chunk_y, chunk_z), (mesh_id, object_id));
        log::debug!("Added mesh for chunk ({}, {}, {}) with {} vertices",
            chunk_x, chunk_y, chunk_z, voxel_vertices.len());
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    log::info!("=== Stratum Voxel World Example ===");
    log::info!("Controls: WASD to move, Space/Shift for up/down, Left Click to capture mouse, ESC to release cursor/exit");

    let event_loop = EventLoop::new().unwrap();
    let window_attrs = winit::window::Window::default_attributes()
        .with_title("Stratum Voxel World")
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

    // Create Stratum renderer
    let mut renderer = StratumRenderer::new(
        device.clone(),
        queue.clone(),
        size.width,
        size.height,
        surface_config.format,
    );
    renderer.set_clear_color([0.53, 0.81, 0.92, 1.0]); // Sky blue
    renderer.set_ambient([0.4, 0.4, 0.5], 0.3);

    // Initialize world partition manager
    // Chunk size is 16 voxels = 16 meters
    let mut world = WorldPartitionManager::new(
        16.0,   // 16m chunks (matches CHUNK_SIZE)
        80.0,   // Visible within 80m
        160.0,  // Preload within 160m
        240.0,  // Unload beyond 240m
    );

    // Create voxel chunk generator
    let mut voxel_generator = VoxelChunkGenerator::new();

    // Create voxel chunk mesh manager
    let mut mesh_manager = VoxelChunkMeshManager::new(&mut renderer);

    // Generate chunks in a grid around spawn
    log::info!("Generating initial chunk grid...");
    let grid_size = 8;
    let mut chunk_count = 0;

    for x in -grid_size..=grid_size {
        for y in 0..4 {  // 4 vertical chunks (0-64m height)
            for z in -grid_size..=grid_size {
                // Generate the voxel chunk
                let _voxel_chunk = voxel_generator.get_or_generate(x, y, z);

                // Register with Stratum world partition
                let metadata = ChunkMetadata::new(x, y, z, 0, 16.0);
                let file_path = PathBuf::from(format!(
                    "voxel_chunks/chunk_{}_{}_{}.bin",
                    x, y, z
                ));
                world.upsert_chunk(metadata, file_path);
                chunk_count += 1;
            }
        }
    }

    // Mark all chunks as loaded (demo mode - no actual files)
    for chunk in world.chunks.values_mut() {
        chunk.mark_loaded();
    }

    log::info!("Created {} voxel chunks", chunk_count);

    // Create camera at spawn height
    let mut camera_controller = CameraController::new(Vec3::new(0.0, 40.0, 0.0));
    let camera_id = world.register_camera(camera_controller.position, Quat::IDENTITY);

    // Set camera projection
    if let Some(camera) = world.registry.get_camera_mut(camera_id) {
        let aspect = size.width as f32 / size.height as f32;
        camera.set_projection(70.0_f32.to_radians(), aspect, 0.1, 1000.0);
    }

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
                                camera_controller.mouse_captured = false;
                                let _ = window.set_cursor_grab(winit::window::CursorGrabMode::None);
                                window.set_cursor_visible(true);
                            }
                            camera_controller.process_keyboard(keycode, key_event.state);
                        }
                    }
                    WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
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

                        // Sync voxel meshes with visible chunks
                        mesh_manager.sync_chunks(&mut renderer, &mut voxel_generator, &world);

                        // Render frame
                        let frame = match surface.get_current_texture() {
                            Ok(f) => f,
                            Err(e) => {
                                log::warn!("Surface error: {:?}", e);
                                return;
                            }
                        };
                        let view = frame.texture.create_view(&Default::default());

                        // Get the camera and render
                        if let Some(camera) = world.registry.get_camera(camera_id) {
                            if let Err(e) = renderer.render_world(&world, camera, &view) {
                                log::error!("Render error: {}", e);
                            }
                        }

                        frame.present();

                        // FPS counter
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
