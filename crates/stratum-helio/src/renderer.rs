use helio::{Camera, Renderer, RendererConfig, required_wgpu_features, required_wgpu_limits, GpuLight, SceneActor};
use stratum::{WorldPartitionManager, Camera as StratumCamera};
use glam::{Vec3, Mat4};
use std::sync::Arc;

/// Wrapper that integrates Helio renderer with Stratum world partition.
/// This hides all Helio details from the user.
pub struct StratumRenderer {
    renderer: Renderer,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
}

impl StratumRenderer {
    /// Create a new StratumRenderer with Helio integration.
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Self {
        let mut renderer = Renderer::new(device.clone(), queue.clone(), RendererConfig::new(width, height, format));

        // Enable editor mode for debug rendering
        renderer.set_editor_mode(true);

        // Add default directional light to avoid crashes (Helio requires at least 1 light)
        let direction = Vec3::new(0.3, -1.0, 0.5).normalize();
        let directional_light = GpuLight {
            position_range: [0.0, 0.0, 0.0, 0.0],  // Not used for directional
            direction_outer: [direction.x, direction.y, direction.z, 0.0],
            color_intensity: [1.0, 0.95, 0.9, 2.5],  // Warm white, intensity 2.5
            shadow_index: u32::MAX,  // No shadow
            light_type: 0,  // LightType::Directional = 0
            inner_angle: 0.0,
            _pad: 0,
        };
        let _ = renderer.scene_mut().insert_actor(SceneActor::light(directional_light));

        Self {
            renderer,
            device,
            queue,
        }
    }

    /// Update render size on window resize.
    pub fn set_render_size(&mut self, width: u32, height: u32) {
        self.renderer.set_render_size(width, height);
    }

    /// Set clear color.
    pub fn set_clear_color(&mut self, color: [f32; 4]) {
        self.renderer.set_clear_color(color);
    }

    /// Set ambient lighting.
    pub fn set_ambient(&mut self, color: [f32; 3], intensity: f32) {
        self.renderer.set_ambient(color, intensity);
    }

    /// Render the world with chunk visualization.
    pub fn render_world(
        &mut self,
        world: &WorldPartitionManager,
        stratum_camera: &StratumCamera,
        output_view: &wgpu::TextureView,
    ) -> Result<(), String> {
        // Clear debug shapes
        self.renderer.debug_clear();

        // Draw world axis lines (always visible for debugging)
        self.renderer.debug_line([-50.0, 0.0, 0.0], [50.0, 0.0, 0.0], [1.0, 0.0, 0.0, 1.0]); // X axis - red
        self.renderer.debug_line([0.0, -50.0, 0.0], [0.0, 50.0, 0.0], [0.0, 1.0, 0.0, 1.0]); // Y axis - green
        self.renderer.debug_line([0.0, 0.0, -50.0], [0.0, 0.0, 50.0], [0.0, 0.0, 1.0, 1.0]); // Z axis - blue

        // Draw a test cross at origin
        let cross_size = 10.0;
        self.renderer.debug_line([-cross_size, 0.0, 0.0], [cross_size, 0.0, 0.0], [1.0, 1.0, 0.0, 1.0]);
        self.renderer.debug_line([0.0, -cross_size, 0.0], [0.0, cross_size, 0.0], [1.0, 1.0, 0.0, 1.0]);
        self.renderer.debug_line([0.0, 0.0, -cross_size], [0.0, 0.0, cross_size], [1.0, 1.0, 0.0, 1.0]);

        // Draw line from origin to camera (white) - should always be visible if camera position is correct
        self.renderer.debug_line([0.0, 0.0, 0.0], stratum_camera.position.to_array(), [1.0, 1.0, 1.0, 1.0]);

        // Draw camera position as a LARGE bright sphere (hard to miss)
        self.renderer.debug_sphere(stratum_camera.position.to_array(), 5.0, [1.0, 1.0, 0.0, 1.0], 16);

        // Draw camera forward direction (what chunk system sees)
        let cam_forward_start = stratum_camera.position;
        let cam_forward_end = stratum_camera.position + stratum_camera.forward * 100.0;
        self.renderer.debug_line(cam_forward_start.to_array(), cam_forward_end.to_array(), [1.0, 0.0, 1.0, 1.0]); // Magenta

        // Draw crosses along forward direction to show the "trail"
        for i in 1..=10 {
            let dist = i as f32 * 10.0;
            let point = stratum_camera.position + stratum_camera.forward * dist;
            let cross_size = 2.0;
            let color = [1.0, 0.0, 1.0, 1.0]; // Magenta

            // Draw 3D cross at this point
            self.renderer.debug_line(
                (point - Vec3::X * cross_size).to_array(),
                (point + Vec3::X * cross_size).to_array(),
                color
            );
            self.renderer.debug_line(
                (point - Vec3::Y * cross_size).to_array(),
                (point + Vec3::Y * cross_size).to_array(),
                color
            );
            self.renderer.debug_line(
                (point - Vec3::Z * cross_size).to_array(),
                (point + Vec3::Z * cross_size).to_array(),
                color
            );
        }

        // Draw camera right direction
        let cam_right = stratum_camera.right();
        let cam_right_end = stratum_camera.position + cam_right * 20.0;
        self.renderer.debug_line(cam_forward_start.to_array(), cam_right_end.to_array(), [0.0, 1.0, 1.0, 1.0]); // Cyan

        // Draw camera up direction
        let cam_up_end = stratum_camera.position + stratum_camera.up() * 20.0;
        self.renderer.debug_line(cam_forward_start.to_array(), cam_up_end.to_array(), [1.0, 1.0, 0.0, 1.0]); // Yellow

        // Log camera info with rotation
        log::info!("Camera pos: {:?}, forward: {:?}, right: {:?}, up: {:?}",
            stratum_camera.position, stratum_camera.forward, stratum_camera.right(), stratum_camera.up());

        // Draw visible chunks as wireframe boxes
        let mut visible_count = 0;
        for chunk in world.chunks.values() {
            if chunk.is_visible() {
                visible_count += 1;
                let aabb = &chunk.metadata.aabb;
                let center = aabb.center();
                let size = aabb.size();

                // Color based on state: green for visible, yellow for loaded
                let color = if chunk.is_visible() {
                    [0.2, 1.0, 0.3, 1.0]
                } else {
                    [1.0, 1.0, 0.2, 0.8]
                };

                // Draw wireframe box for chunk
                self.draw_chunk_box(center, size, color);
            }
        }

        log::debug!("Drawing {} visible chunks", visible_count);

        // Convert Stratum camera to Helio camera
        let helio_camera = Camera::perspective_look_at(
            stratum_camera.position,
            stratum_camera.position + stratum_camera.forward,
            stratum_camera.up(),
            stratum_camera.fov,
            stratum_camera.aspect_ratio,
            stratum_camera.near,
            stratum_camera.far,
        );

        // Render
        self.renderer.render(&helio_camera, output_view)
            .map_err(|e| format!("Render error: {:?}", e))
    }

    /// Draw a wireframe box for a chunk.
    fn draw_chunk_box(&mut self, center: Vec3, size: Vec3, color: [f32; 4]) {
        let half = size * 0.5;

        // Bottom face
        let b0 = center + Vec3::new(-half.x, -half.y, -half.z);
        let b1 = center + Vec3::new( half.x, -half.y, -half.z);
        let b2 = center + Vec3::new( half.x, -half.y,  half.z);
        let b3 = center + Vec3::new(-half.x, -half.y,  half.z);

        // Top face
        let t0 = center + Vec3::new(-half.x, half.y, -half.z);
        let t1 = center + Vec3::new( half.x, half.y, -half.z);
        let t2 = center + Vec3::new( half.x, half.y,  half.z);
        let t3 = center + Vec3::new(-half.x, half.y,  half.z);

        // Bottom edges
        self.renderer.debug_line(b0.to_array(), b1.to_array(), color);
        self.renderer.debug_line(b1.to_array(), b2.to_array(), color);
        self.renderer.debug_line(b2.to_array(), b3.to_array(), color);
        self.renderer.debug_line(b3.to_array(), b0.to_array(), color);

        // Top edges
        self.renderer.debug_line(t0.to_array(), t1.to_array(), color);
        self.renderer.debug_line(t1.to_array(), t2.to_array(), color);
        self.renderer.debug_line(t2.to_array(), t3.to_array(), color);
        self.renderer.debug_line(t3.to_array(), t0.to_array(), color);

        // Vertical edges
        self.renderer.debug_line(b0.to_array(), t0.to_array(), color);
        self.renderer.debug_line(b1.to_array(), t1.to_array(), color);
        self.renderer.debug_line(b2.to_array(), t2.to_array(), color);
        self.renderer.debug_line(b3.to_array(), t3.to_array(), color);
    }

    /// Get required WGPU features for Helio.
    pub fn required_features(available: wgpu::Features) -> wgpu::Features {
        required_wgpu_features(available)
    }

    /// Get required WGPU limits for Helio.
    pub fn required_limits(available: wgpu::Limits) -> wgpu::Limits {
        required_wgpu_limits(available)
    }
}
