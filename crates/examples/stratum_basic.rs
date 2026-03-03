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
//!   1 / 2       — single fullscreen view / 4-up multiview (top / side / front)

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
use stratum_helio::{Material, TextureData};

use stratum::{
    CameraId, CameraKind, Components, LightData, MaterialHandle,
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
// Multiview — offscreen blit infrastructure
// ─────────────────────────────────────────────────────────────────────────────

/// Logical names for the 4 offscreen render targets.
/// Order: 0 = top-left (freelook), 1 = top-right (top), 2 = bot-left (side), 3 = bot-right (front)
const MV_NAMES: [&str; 4] = ["mv_freelook", "mv_top", "mv_side", "mv_front"];

/// Return the linear (non-sRGB) equivalent of `fmt`.
///
/// sRGB textures on Vulkan/DX12 require declaring a non-sRGB `view_formats`
/// entry before they can be used as `TEXTURE_BINDING`. To avoid that complexity
/// we simply use the linear format for the offscreen textures: Helio outputs
/// linear light values that the hardware encodes to sRGB when writing to the
/// actual swapchain surface, so the blit just needs to carry those linear
/// values across unchanged.
fn non_srgb_format(fmt: wgpu::TextureFormat) -> wgpu::TextureFormat {
    use wgpu::TextureFormat::*;
    match fmt {
        Rgba8UnormSrgb   => Rgba8Unorm,
        Bgra8UnormSrgb   => Bgra8Unorm,
        Bc1RgbaUnormSrgb => Bc1RgbaUnorm,
        Bc2RgbaUnormSrgb => Bc2RgbaUnorm,
        Bc3RgbaUnormSrgb => Bc3RgbaUnorm,
        Bc7RgbaUnormSrgb => Bc7RgbaUnorm,
        other            => other,
    }
}

/// Fullscreen-triangle blit: samples one texture and writes it into the
/// viewport/scissor region of a colour attachment. No uniforms — placement
/// is controlled entirely by `set_viewport` / `set_scissor_rect`.
const BLIT_SHADER: &str = r#"
struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0)       uv:  vec2<f32>,
};
@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VOut {
    // Two-triangle fullscreen quad.
    var xy: array<vec2<f32>, 6>;
    xy[0] = vec2(-1.0, -1.0); xy[1] = vec2( 1.0, -1.0); xy[2] = vec2(-1.0,  1.0);
    xy[3] = vec2(-1.0,  1.0); xy[4] = vec2( 1.0, -1.0); xy[5] = vec2( 1.0,  1.0);
    let p = xy[vi];
    // NDC → UV (flip Y: row 0 of the texture is the top of the image).
    return VOut(vec4(p, 0.0, 1.0), vec2(p.x * 0.5 + 0.5, p.y * -0.5 + 0.5));
}
@group(0) @binding(0) var t_src: texture_2d<f32>;
@group(0) @binding(1) var s_src: sampler;
@fragment
fn fs_main(v: VOut) -> @location(0) vec4<f32> {
    return textureSample(t_src, s_src, v.uv);
}
"#;

struct BlitPipeline {
    pipeline:  wgpu::RenderPipeline,
    sampler:   wgpu::Sampler,
    bg_layout: wgpu::BindGroupLayout,
}

impl BlitPipeline {
    fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("mv_blit_shader"),
            source: wgpu::ShaderSource::Wgsl(BLIT_SHADER.into()),
        });

        let bg_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("mv_blit_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding:    0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type:    wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled:   false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding:    1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        // Extract to a named local so the borrow inside the descriptor is valid.
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:              Some("mv_blit_layout"),
            bind_group_layouts: &[Some(&bg_layout)],
            immediate_size:     0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("mv_blit_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module:              &shader,
                entry_point:         Some("vs_main"),
                buffers:             &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module:      &shader,
                entry_point: Some("fs_main"),
                targets:     &[Some(wgpu::ColorTargetState {
                    format:     surface_format,   // writes to the sRGB swapchain
                    blend:      None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive:     wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample:   wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache:         None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label:      Some("mv_blit_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Self { pipeline, sampler, bg_layout }
    }

    /// Encode one blit draw into `encoder`, writing `src` into the rectangle
    /// `(ox, oy, w, h)` of the render pass attachment `dst`.
    fn blit(
        &self,
        device:  &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        src:     &wgpu::TextureView,
        dst:     &wgpu::TextureView,
        ox: f32, oy: f32, w: f32, h: f32,
    ) {
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("mv_blit_bg"),
            layout:  &self.bg_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding:  0,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding:  1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("mv_blit_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view:           dst,
                resolve_target: None,
                depth_slice:    None,
                ops: wgpu::Operations {
                    // Load preserves the output of the previous quadrant blit.
                    load:  wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set:      None,
            timestamp_writes:         None,
            multiview_mask:           None,
        });

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bg, &[]);
        pass.set_viewport(ox, oy, w, h, 0.0, 1.0);
        pass.set_scissor_rect(ox as u32, oy as u32, w as u32, h as u32);
        pass.draw(0..6, 0..1);
    }
}

/// Four full-size offscreen textures — one per multiview camera.
///
/// Each is the same pixel dimensions as the window so Helio's internal
/// buffers (depth, g-buffers, etc.) remain the right size. The
/// `blit_to_surface` call then scales each one into its screen quadrant.
struct MultiviewTextures {
    textures:     [wgpu::Texture; 4],
    /// Views for sampling FROM in the blit shader.  Separate `TextureView`
    /// objects from the render views registered with `HelioIntegration`
    /// (which are used for writing TO), but backed by the same GPU memory.
    sample_views: [wgpu::TextureView; 4],
    blit:         BlitPipeline,
    width:        u32,
    height:       u32,
}

impl MultiviewTextures {
    fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat, width: u32, height: u32) -> Self {
        // Offscreen textures must use the same format as Helio's pipelines
        // (i.e. `surface_format`, which is sRGB).  To sample them in the blit
        // shader we declare the non-sRGB equivalent in `view_formats` and then
        // create `sample_views` with that format — avoiding the Vulkan/DX12
        // restriction that sRGB TEXTURE_BINDING requires a non-sRGB view_format.
        let sample_format = non_srgb_format(surface_format);

        let textures: [wgpu::Texture; 4] = std::array::from_fn(|i| {
            device.create_texture(&wgpu::TextureDescriptor {
                label:           Some(MV_NAMES[i]),
                size:            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count:    1,
                dimension:       wgpu::TextureDimension::D2,
                format:          surface_format,       // sRGB — matches Helio's pipeline
                usage:           wgpu::TextureUsages::RENDER_ATTACHMENT
                               | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats:    &[sample_format],     // allow non-sRGB sampling view
            })
        });

        let sample_views: [wgpu::TextureView; 4] = std::array::from_fn(|i| {
            textures[i].create_view(&wgpu::TextureViewDescriptor {
                format: Some(sample_format),           // blit shader reads linear values
                ..Default::default()
            })
        });

        Self {
            textures,
            sample_views,
            blit: BlitPipeline::new(device, surface_format),
            width,
            height,
        }
    }

    /// Create fresh render views to hand to `HelioIntegration` (write targets).
    /// Distinct `TextureView` objects from `sample_views`; both refer to the
    /// same underlying GPU textures.
    fn make_render_views(&self) -> [wgpu::TextureView; 4] {
        std::array::from_fn(|i| {
            self.textures[i].create_view(&wgpu::TextureViewDescriptor::default())
        })
    }

    /// Composite the 4 offscreen textures into the 4 quadrants of `surface_view`.
    ///
    /// A clear pass runs first so no stale pixels from the previous frame
    /// leak through. Then one blit pass per quadrant:
    ///
    ///   top-left  │ top-right
    ///   ──────────┼──────────
    ///   bot-left  │ bot-right
    fn blit_to_surface(
        &self,
        device:       &wgpu::Device,
        encoder:      &mut wgpu::CommandEncoder,
        surface_view: &wgpu::TextureView,
    ) {
        // Clear the entire surface to black before we start blitting quadrants.
        {
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mv_clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view:           surface_view,
                    resolve_target: None,
                    depth_slice:    None,
                    ops: wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set:      None,
                timestamp_writes:         None,
                multiview_mask:           None,
            });
            // No draw calls — just the clear.
        }

        let qw = (self.width  / 2) as f32;
        let qh = (self.height / 2) as f32;

        // top-left, top-right, bot-left, bot-right
        let origins: [(f32, f32); 4] = [(0.0, 0.0), (qw, 0.0), (0.0, qh), (qw, qh)];

        for (i, (ox, oy)) in origins.iter().enumerate() {
            self.blit.blit(device, encoder, &self.sample_views[i], surface_view, *ox, *oy, qw, qh);
        }
    }
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
    queue:          Arc<wgpu::Queue>,
    surface_format: wgpu::TextureFormat,

    // ── World orchestration ───────────────────────────────────────────────────
    stratum:         Stratum,
    integration:     HelioIntegration,
    main_camera_id:  CameraId,

    // ── Multiview cameras (inactive until 2 is pressed) ───────────────────────
    cam_top_id:       CameraId,
    cam_side_id:      CameraId,
    cam_front_id:     CameraId,
    multiview_active: bool,
    mv_textures:      Option<MultiviewTextures>,

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

        // ── PBR cube material: image.png as base-colour texture ───────────────
        let (cube_tex_rgba, cube_tex_w, cube_tex_h) = load_sprite("image.png");
        let cube_material_desc = Material::new()
            .with_roughness(0.45)
            .with_metallic(0.0)
            .with_base_color_texture(TextureData::new(cube_tex_rgba, cube_tex_w, cube_tex_h));

        let mut integration = HelioIntegration::new(renderer, assets);

        let gpu_cube_mat = integration.create_material(&cube_material_desc);
        let h_cube_mat: MaterialHandle = integration.assets_mut().add_material(gpu_cube_mat);

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
                .with_material(h_cube_mat)
                .with_bounding_radius(0.5 * f32::sqrt(3.0)),
        );
        level.spawn_entity(
            Components::new()
                .with_transform(Transform::from_position(Vec3::new(-2.0, 0.4, -1.0)))
                .with_mesh(h_cube2)
                .with_material(h_cube_mat)
                .with_bounding_radius(0.4 * f32::sqrt(3.0)),
        );
        level.spawn_entity(
            Components::new()
                .with_transform(Transform::from_position(Vec3::new( 2.0, 0.3,  0.5)))
                .with_mesh(h_cube3)
                .with_material(h_cube_mat)
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

        // ── Register multiview cameras (Game mode, initially inactive) ─────────
        //
        // Each camera renders to its own full-size offscreen texture (registered
        // when multiview is toggled on).  Viewports are Viewport::full() because
        // the texture is that camera's private canvas — the quadrant placement is
        // handled entirely by the blit pass.
        //
        //   top-right : top-down orthographic
        //   bot-left  : side orthographic (from +X looking −X)
        //   bot-right : front orthographic (from +Z looking −Z)
        const ORTHO_HALF: f32 = 8.0; // half-extent for ortho views

        let cam_top_id = stratum.register_camera(StratumCamera {
            id:            CameraId::PLACEHOLDER,
            kind:          CameraKind::GameCamera { tag: "multiview_top".into() },
            position:      Vec3::new(0.0, 2.5, 7.0),
            yaw:           0.0,
            pitch:         -std::f32::consts::FRAC_PI_2,
            projection:    Projection::orthographic_symmetric(ORTHO_HALF, ORTHO_HALF, 0.1, 200.0),
            render_target: RenderTargetHandle::OffscreenTexture(MV_NAMES[1].into()),
            viewport:      Viewport::full(),
            priority:      1,
            active:        false,
        });

        let cam_side_id = stratum.register_camera(StratumCamera {
            id:            CameraId::PLACEHOLDER,
            kind:          CameraKind::GameCamera { tag: "multiview_side".into() },
            position:      Vec3::new(0.0, 2.5, 7.0),
            yaw:           -std::f32::consts::FRAC_PI_2, // looks toward −X
            pitch:         0.0,
            projection:    Projection::orthographic_symmetric(ORTHO_HALF, ORTHO_HALF, 0.1, 200.0),
            render_target: RenderTargetHandle::OffscreenTexture(MV_NAMES[2].into()),
            viewport:      Viewport::full(),
            priority:      2,
            active:        false,
        });

        let cam_front_id = stratum.register_camera(StratumCamera {
            id:            CameraId::PLACEHOLDER,
            kind:          CameraKind::GameCamera { tag: "multiview_front".into() },
            position:      Vec3::new(0.0, 2.5, 7.0),
            yaw:           0.0,  // looks toward −Z
            pitch:         0.0,
            projection:    Projection::orthographic_symmetric(ORTHO_HALF, ORTHO_HALF, 0.1, 200.0),
            render_target: RenderTargetHandle::OffscreenTexture(MV_NAMES[3].into()),
            viewport:      Viewport::full(),
            priority:      3,
            active:        false,
        });

        self.state = Some(AppState {
            window,
            surface,
            device,
            queue,
            surface_format,
            stratum,
            integration,
            main_camera_id,
            cam_top_id,
            cam_side_id,
            cam_front_id,
            multiview_active: false,
            mv_textures:      None,
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
            } => {
                match ks {
                    ElementState::Pressed  => { state.keys.insert(key); }
                    ElementState::Released => { state.keys.remove(&key); }
                }
                // 1 → single fullscreen view; 2 → 4-up multiview
                if ks == ElementState::Pressed {
                    match key {
                        KeyCode::Digit1 => state.set_multiview(false),
                        KeyCode::Digit2 => state.set_multiview(true),
                        _ => {}
                    }
                }
            }

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
                // Recreate offscreen textures to match the new window size.
                if state.multiview_active {
                    state.rebuild_mv_textures(size.width, size.height);
                }
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
    /// Switch to 4-up multiview (`true`) or back to single fullscreen view (`false`).
    fn set_multiview(&mut self, active: bool) {
        if self.multiview_active == active { return; }
        self.multiview_active = active;

        if active {
            let size = self.window.inner_size();
            self.rebuild_mv_textures(size.width, size.height);

            // Main camera renders to its own offscreen buffer (top-left quadrant).
            if let Some(cam) = self.stratum.cameras_mut().get_mut(self.main_camera_id) {
                cam.render_target = RenderTargetHandle::OffscreenTexture(MV_NAMES[0].into());
                cam.viewport      = Viewport::full();
            }
            // Activate the 3 fixed ortho cameras.
            for &id in &[self.cam_top_id, self.cam_side_id, self.cam_front_id] {
                if let Some(cam) = self.stratum.cameras_mut().get_mut(id) {
                    cam.active = true;
                }
            }
            log::info!("Multiview ON");
        } else {
            // Restore main camera to the primary surface.
            if let Some(cam) = self.stratum.cameras_mut().get_mut(self.main_camera_id) {
                cam.render_target = RenderTargetHandle::PrimarySurface;
                cam.viewport      = Viewport::full();
            }
            for &id in &[self.cam_top_id, self.cam_side_id, self.cam_front_id] {
                if let Some(cam) = self.stratum.cameras_mut().get_mut(id) {
                    cam.active = false;
                }
            }
            // Unregister all 4 offscreen views and drop the textures.
            for name in &MV_NAMES {
                self.integration.unregister_offscreen_view(name);
            }
            self.mv_textures = None;
            log::info!("Multiview OFF");
        }
    }

    /// (Re)create the 4 offscreen textures and register their render views with
    /// the integration. Called on toggle-on and on window resize while active.
    fn rebuild_mv_textures(&mut self, width: u32, height: u32) {
        // Drop old views first so the integration doesn't hold stale handles.
        for name in &MV_NAMES {
            self.integration.unregister_offscreen_view(name);
        }
        let mv = MultiviewTextures::new(&self.device, self.surface_format, width, height);
        let render_views = mv.make_render_views();
        for (i, view) in render_views.into_iter().enumerate() {
            self.integration.register_offscreen_view(MV_NAMES[i], view);
        }
        self.mv_textures = Some(mv);

        // Update ortho camera projections to match the quadrant aspect ratio.
        // Each quadrant is (width/2) × (height/2) pixels.
        let qw = (width  / 2) as f32;
        let qh = (height / 2) as f32;
        let aspect = qw / qh;
        const ORTHO_HALF_H: f32 = 8.0;
        let ortho_half_w = ORTHO_HALF_H * aspect;
        for &id in &[self.cam_top_id, self.cam_side_id, self.cam_front_id] {
            if let Some(cam) = self.stratum.cameras_mut().get_mut(id) {
                cam.projection = Projection::orthographic_symmetric(
                    ortho_half_w, ORTHO_HALF_H, 0.1, 200.0,
                );
            }
        }
    }

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

        // ── Track fixed multiview cameras onto the main camera position ────────
        if self.multiview_active {
            const ORTHO_DIST: f32 = 20.0;
            let main_pos = self.stratum.cameras()
                .get(self.main_camera_id)
                .map(|c| c.position)
                .unwrap_or(Vec3::ZERO);

            // Top: directly above, looking straight down
            if let Some(cam) = self.stratum.cameras_mut().get_mut(self.cam_top_id) {
                cam.position = main_pos + Vec3::Y * ORTHO_DIST;
            }
            // Side: offset along +X, looking toward −X
            if let Some(cam) = self.stratum.cameras_mut().get_mut(self.cam_side_id) {
                cam.position = main_pos + Vec3::X * ORTHO_DIST;
            }
            // Front: offset along +Z, looking toward −Z
            if let Some(cam) = self.stratum.cameras_mut().get_mut(self.cam_front_id) {
                cam.position = main_pos + Vec3::Z * ORTHO_DIST;
            }
        }

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

        // ── Multiview composite: blit 4 offscreen textures into quadrants ─────
        if self.multiview_active {
            if let Some(mv) = &self.mv_textures {
                let mut encoder = self.device.create_command_encoder(
                    &wgpu::CommandEncoderDescriptor { label: Some("mv_composite") },
                );
                mv.blit_to_surface(&self.device, &mut encoder, &surface_view);
                self.queue.submit([encoder.finish()]);
            }
        }

        output.present();
    }
}
