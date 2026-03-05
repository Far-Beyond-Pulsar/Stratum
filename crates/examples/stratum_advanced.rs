//! `stratum_advanced` — Multi-zone world with streaming, multi-camera, and animated lights.
//!
//! Demonstrates all major Stratum features:
//!
//! * **4-zone world** — City · Factory · Forest · Void, each in its own 30 m chunk
//! * **World partition streaming** — fly from the centre to zone D and watch
//!   zones A/B/C deactivate as they leave the 50 m activation radius
//! * **Multi-camera registry** — main perspective, top-down overview, editor
//! * **Editor / Game mode** — Tab toggles; only matching cameras render
//! * **Animated lights with billboards** — 12 orbiting point lights (3 per zone)
//!   each carry both `LightData` and `BillboardData` components
//!
//! ## Controls
//!
//! | Key              | Action                                        |
//! |------------------|-----------------------------------------------|
//! | WASD             | Fly main camera forward / left / back / right |
//! | Space / LShift   | Fly up / down                                 |
//! | Left mouse drag  | Look (click window first to grab cursor)      |
//! | Tab              | Toggle Editor ↔ Game mode                     |
//! | F1               | Main perspective camera (Game mode)           |
//! | F2               | Top-down overview camera (Game mode)          |
//! | F3               | Print partition + entity debug stats          |
//! | P                | Toggle world-partition chunk AABB wireframes  |
//! | 1 / 2            | Single fullscreen view / 4-up multiview       |
//! | Escape           | Release cursor (or exit if cursor is free)    |
//!
//! ## World Layout
//!
//! ```text
//!  Z
//!  ^
//!  │  [Zone C  Forest ]   [Zone D  Void   ]   ← chunk (0,0,2) / (2,0,2)
//!  │  chunk 0-30 Z        chunk 60-90 Z
//!  │
//!  │  [Zone A  City   ]   [Zone B  Factory]   ← chunk (0,0,0) / (2,0,0)
//!  │  chunk 0-30 X        chunk 60-90 X
//!  └──────────────────────────────────────────> X
//! ```
//!
//! Camera starts at world (45, 15, 95) — centred between all zones.

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
    BillboardData, CameraId, CameraKind, Components, LightData, MaterialHandle,
    Projection, RenderTargetHandle, SimulationMode,
    Stratum, StratumCamera, Transform, Viewport, EntityId,
};

use stratum_helio::{AssetRegistry, HelioIntegration};

// ── World parameters ──────────────────────────────────────────────────────────

/// Side length of each world-partition chunk (metres).
const CHUNK_SIZE: f32 = 30.0;

/// Chunks within this distance of the main camera stay resident.
/// Designed so that zones 45+ m away deactivate when the player
/// reaches an opposite corner of the world.
const ACTIVATION_RADIUS: f32 = 50.0;

/// World-space centres of each zone (XZ). Y is always 0 (ground level).
const ZONE_A: Vec3 = Vec3::new(15.0, 0.0, 15.0); // City
const ZONE_B: Vec3 = Vec3::new(75.0, 0.0, 15.0); // Factory
const ZONE_C: Vec3 = Vec3::new(15.0, 0.0, 75.0); // Forest
const ZONE_D: Vec3 = Vec3::new(75.0, 0.0, 75.0); // Void

// ── Animated light descriptor ─────────────────────────────────────────────────

/// Describes one orbiting light. The entity stores both `LightData` and
/// `BillboardData`; we update its `Transform` every frame.
struct ZoneLight {
    entity_id: EntityId,
    /// XZ centre around which this light orbits.
    center:    Vec3,
    /// Orbit radius (metres). Kept within the zone's chunk boundary.
    radius:    f32,
    /// Fixed height (Y) of the orbit.
    height:    f32,
    /// Angular speed (radians per second).
    speed:     f32,
    /// Starting phase offset (radians).
    phase:     f32,
}

// ── Camera view mode (in-game only) ──────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    /// Main perspective free-fly camera.
    Main,
    /// Top-down orthographic overview camera.
    Overview,
}

// ── Sprite loader ─────────────────────────────────────────────────────────────

fn load_sprite(path: &str) -> (Vec<u8>, u32, u32) {
    let asset_bytes: Option<&'static [u8]> = match path {
        "image.png" => Some(include_bytes!("../../assets/image.png")),
        "spotlight.png" => Some(include_bytes!("../../assets/spotlight.png")),
        _ => None,
    };

    let img = asset_bytes
        .and_then(|bytes| image::load_from_memory(bytes).ok())
        .unwrap_or_else(|| {
            log::warn!("Could not decode embedded '{}', using 1×1 white opaque fallback", path);
            // new_rgba8 zero-initialises pixels (alpha=0). The billboard shader
            // multiplies tex_color.a * instance_alpha — a zero texture alpha
            // causes all billboards to be silently discarded.
            // Always use a fully-opaque white pixel so colour tints show correctly.
            let mut px = image::RgbaImage::new(1, 1);
            px.put_pixel(0, 0, image::Rgba([255, 255, 255, 255]));
            image::DynamicImage::ImageRgba8(px)
        })
        .into_rgba8();
    let (w, h) = img.dimensions();
    (img.into_raw(), w, h)
}

// ── Multiview — offscreen blit infrastructure ─────────────────────────────────

const MV_NAMES: [&str; 4] = ["mv_freelook", "mv_top", "mv_side", "mv_front"];

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

const BLIT_SHADER: &str = r#"
struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0)       uv:  vec2<f32>,
};
@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VOut {
    var xy: array<vec2<f32>, 6>;
    xy[0] = vec2(-1.0, -1.0); xy[1] = vec2( 1.0, -1.0); xy[2] = vec2(-1.0,  1.0);
    xy[3] = vec2(-1.0,  1.0); xy[4] = vec2( 1.0, -1.0); xy[5] = vec2( 1.0,  1.0);
    let p = xy[vi];
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
                    format:     surface_format,
                    blend:      None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive:      wgpu::PrimitiveState::default(),
            depth_stencil:  None,
            multisample:    wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache:          None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label:      Some("mv_blit_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self { pipeline, sampler, bg_layout }
    }

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
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("mv_blit_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view:           dst,
                resolve_target: None,
                depth_slice:    None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
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

struct MultiviewTextures {
    textures:     [wgpu::Texture; 4],
    sample_views: [wgpu::TextureView; 4],
    blit:         BlitPipeline,
    width:        u32,
    height:       u32,
}

impl MultiviewTextures {
    fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat, width: u32, height: u32) -> Self {
        let sample_format = non_srgb_format(surface_format);
        let textures: [wgpu::Texture; 4] = std::array::from_fn(|i| {
            device.create_texture(&wgpu::TextureDescriptor {
                label:           Some(MV_NAMES[i]),
                size:            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count:    1,
                dimension:       wgpu::TextureDimension::D2,
                format:          surface_format,
                usage:           wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats:    &[sample_format],
            })
        });
        let sample_views: [wgpu::TextureView; 4] = std::array::from_fn(|i| {
            textures[i].create_view(&wgpu::TextureViewDescriptor {
                format: Some(sample_format),
                ..Default::default()
            })
        });
        Self { textures, sample_views, blit: BlitPipeline::new(device, surface_format), width, height }
    }

    fn make_render_views(&self) -> [wgpu::TextureView; 4] {
        std::array::from_fn(|i| self.textures[i].create_view(&wgpu::TextureViewDescriptor::default()))
    }

    fn blit_to_surface(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, surface_view: &wgpu::TextureView) {
        {
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mv_clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view:           surface_view,
                    resolve_target: None,
                    depth_slice:    None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set:      None,
                timestamp_writes:         None,
                multiview_mask:           None,
            });
        }
        let qw = (self.width  / 2) as f32;
        let qh = (self.height / 2) as f32;
        let origins: [(f32, f32); 4] = [(0.0, 0.0), (qw, 0.0), (0.0, qh), (qw, qh)];
        for (i, (ox, oy)) in origins.iter().enumerate() {
            self.blit.blit(device, encoder, &self.sample_views[i], surface_view, *ox, *oy, qw, qh);
        }
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    env_logger::init();
    log::info!("Stratum Advanced — starting");
    log::info!("Controls: WASD fly | Space/Shift up/down | Tab mode | F1/F2 camera | F3 debug");

    let event_loop = EventLoop::new().expect("event loop");
    let mut app    = App::new();
    event_loop.run_app(&mut app).expect("event loop error");
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
    stratum:          Stratum,
    integration:      HelioIntegration,

    // ── Camera IDs ────────────────────────────────────────────────────────────
    main_cam_id:      CameraId,
    overview_cam_id:  CameraId,
    #[allow(dead_code)]
    editor_cam_id:    CameraId,

    // ── Multiview cameras ─────────────────────────────────────────────────────
    cam_top_id:       CameraId,
    cam_side_id:      CameraId,
    cam_front_id:     CameraId,
    multiview_active: bool,
    mv_textures:      Option<MultiviewTextures>,
    queue:            Arc<wgpu::Queue>,

    // ── Scene state ───────────────────────────────────────────────────────────
    zone_lights:      Vec<ZoneLight>,
    time:             f32,

    // ── Active view mode (only applies in Game mode) ──────────────────────────
    view_mode:        ViewMode,

    // ── Per-frame timing ──────────────────────────────────────────────────────
    last_frame:       std::time::Instant,

    // ── Input state ───────────────────────────────────────────────────────────
    keys:             HashSet<KeyCode>,
    cursor_grabbed:   bool,
    mouse_delta:      (f32, f32),

    // ── Debug overlays ────────────────────────────────────────────────────────
    show_partition_debug: bool,
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
                        .with_title("Stratum Advanced — 4-Zone Streaming World")
                        .with_inner_size(winit::dpi::LogicalSize::new(1280u32, 720u32)),
                )
                .expect("window"),
        );

        // ── wgpu setup ───────────────────────────────────────────────────────
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let surface = instance.create_surface(window.clone()).expect("surface");

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference:       wgpu::PowerPreference::HighPerformance,
            compatible_surface:     Some(&surface),
            force_fallback_adapter: false,
        }))
        .expect("GPU adapter");

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label:                Some("Stratum Advanced Device"),
                required_features:    wgpu::Features::EXPERIMENTAL_RAY_QUERY,
                required_limits:      wgpu::Limits::default()
                    .using_minimum_supported_acceleration_structure_values(),
                memory_hints:         wgpu::MemoryHints::default(),
                experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
                trace:                wgpu::Trace::Off,
            },
        ))
        .expect("wgpu device (ray tracing required)");

        let device = Arc::new(device);
        let queue  = Arc::new(queue);

        let surface_caps   = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats.iter()
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

        // ── Feature registry (wider world bounds than the basic demo) ─────────
        let (sprite_rgba, sprite_w, sprite_h) = load_sprite("spotlight.png");
        let feature_registry = FeatureRegistry::builder()
            .with_feature(LightingFeature::new())
            .with_feature(BloomFeature::new().with_intensity(0.5).with_threshold(0.8))
            .with_feature(ShadowsFeature::new().with_atlas_size(2048).with_max_lights(8))
            .with_feature(BillboardsFeature::new()
                .with_sprite(sprite_rgba, sprite_w, sprite_h))
            .with_feature(
                RadianceCascadesFeature::new()
                    .with_world_bounds([0.0, -1.0, 0.0], [90.0, 20.0, 90.0]),
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
        .expect("Helio renderer");

        // ── Asset registry ────────────────────────────────────────────────────
        let mut assets = AssetRegistry::new();

        // Ground — a single large plane covering the whole demo world
        let h_ground = assets.add(GpuMesh::plane(&device, [45.0, 0.0, 45.0], 47.5));

        // Zone A — City: tall cube towers (chunk 0,0,0 — world x0..30, z0..30)
        let h_a_tower1 = assets.add(GpuMesh::cube(&device, [ 8.0,  4.0,  8.0], 4.0));
        let h_a_tower2 = assets.add(GpuMesh::cube(&device, [20.0,  3.0, 12.0], 3.0));
        let h_a_tower3 = assets.add(GpuMesh::cube(&device, [12.0,  2.0, 22.0], 2.0));
        let h_a_tower4 = assets.add(GpuMesh::cube(&device, [23.0,  5.5,  7.0], 5.5));

        // Zone B — Factory: large industrial boxes (chunk 2,0,0 — world x60..90, z0..30)
        let h_b_box1   = assets.add(GpuMesh::cube(&device, [68.0,  4.0, 12.0], 4.0));
        let h_b_box2   = assets.add(GpuMesh::cube(&device, [82.0,  3.0, 10.0], 3.0));
        let h_b_box3   = assets.add(GpuMesh::cube(&device, [75.0,  3.0, 22.0], 3.0));
        let h_b_plat   = assets.add(GpuMesh::plane(&device, [75.0, 0.5, 12.0],  8.0));

        // Zone C — Forest: scattered small "tree" cubes (chunk 0,0,2 — world x0..30, z60..90)
        let h_c_tree1  = assets.add(GpuMesh::cube(&device, [ 8.0,  2.0, 68.0], 2.0));
        let h_c_tree2  = assets.add(GpuMesh::cube(&device, [18.0,  1.5, 76.0], 1.5));
        let h_c_tree3  = assets.add(GpuMesh::cube(&device, [10.0,  1.0, 83.0], 1.0));
        let h_c_tree4  = assets.add(GpuMesh::cube(&device, [24.0,  3.0, 70.0], 3.0));
        let h_c_tree5  = assets.add(GpuMesh::cube(&device, [14.0,  0.8, 79.0], 0.8));

        // Zone D — Void: monolith + scattered rocks (chunk 2,0,2 — world x60..90, z60..90)
        let h_d_mono   = assets.add(GpuMesh::cube(&device, [75.0,  6.0, 75.0], 6.0));
        let h_d_rock1  = assets.add(GpuMesh::cube(&device, [63.0,  0.8, 63.0], 0.8));
        let h_d_rock2  = assets.add(GpuMesh::cube(&device, [84.0,  0.7, 83.0], 0.7));
        let h_d_rock3  = assets.add(GpuMesh::cube(&device, [70.0,  0.5, 83.0], 0.5));

        let integration = HelioIntegration::new(renderer, assets);

        // ── PBR cube material: image.png as base-colour texture ───────────────
        // All cube meshes across every zone share the same material.
        // The `load_sprite` fallback (1×1 white pixel) ensures no crash when
        // image.png is unavailable.
        let (cube_tex_rgba, cube_tex_w, cube_tex_h) = load_sprite("image.png");
        let cube_material_desc = Material::new()
            .with_roughness(0.45)
            .with_metallic(0.0)
            .with_base_color_texture(TextureData::new(cube_tex_rgba, cube_tex_w, cube_tex_h));

        let mut integration = integration;
        let gpu_cube_mat = integration.create_material(&cube_material_desc);
        let h_cube_mat: MaterialHandle = integration.assets_mut().add_material(gpu_cube_mat);
        // ── Stratum world setup ───────────────────────────────────────────────
        let mut stratum = Stratum::new(SimulationMode::Game);
        let level_id    = stratum.create_level("advanced_world", CHUNK_SIZE, ACTIVATION_RADIUS);
        let level       = stratum.level_mut(level_id).unwrap();

        // ── Ground plane (chunk 1,0,1 — world centre) ─────────────────────────
        // Bounding sphere: half_extent=47.5 → 47.5 * sqrt(2) ≈ 67.2 m
        level.spawn_entity(
            Components::new()
                .with_transform(Transform::from_position(Vec3::new(45.0, 0.0, 45.0)))
                .with_mesh(h_ground)
                .with_bounding_radius(47.5 * f32::sqrt(2.0))
                .with_tag("ground"),
        );

        // ── Zone A — City (warm orange) ───────────────────────────────────────
        // half_sizes: 4, 3, 2, 5.5 → radii ≈ 6.9, 5.2, 3.5, 9.5
        for (pos, handle, radius) in [
            (Vec3::new( 8.0,  4.0,  8.0), h_a_tower1, 4.0_f32 * f32::sqrt(3.0)),
            (Vec3::new(20.0,  3.0, 12.0), h_a_tower2, 3.0_f32 * f32::sqrt(3.0)),
            (Vec3::new(12.0,  2.0, 22.0), h_a_tower3, 2.0_f32 * f32::sqrt(3.0)),
            (Vec3::new(23.0,  5.5,  7.0), h_a_tower4, 5.5_f32 * f32::sqrt(3.0)),
        ] {
            level.spawn_entity(
                Components::new()
                    .with_transform(Transform::from_position(pos))
                    .with_mesh(handle)
                    .with_material(h_cube_mat)
                    .with_bounding_radius(radius)
                    .with_tag("zone_a"),
            );
        }

        // ── Zone B — Factory (cool blue) ─────────────────────────────────────
        for (pos, handle, radius) in [
            (Vec3::new(68.0,  4.0, 12.0), h_b_box1, 4.0_f32 * f32::sqrt(3.0)),
            (Vec3::new(82.0,  3.0, 10.0), h_b_box2, 3.0_f32 * f32::sqrt(3.0)),
            (Vec3::new(75.0,  3.0, 22.0), h_b_box3, 3.0_f32 * f32::sqrt(3.0)),
            (Vec3::new(75.0,  0.5, 12.0), h_b_plat, 8.0_f32 * f32::sqrt(2.0)), // plane
        ] {
            level.spawn_entity(
                Components::new()
                    .with_transform(Transform::from_position(pos))
                    .with_mesh(handle)
                    .with_material(h_cube_mat)
                    .with_bounding_radius(radius)
                    .with_tag("zone_b"),
            );
        }

        // ── Zone C — Forest (emerald green) ──────────────────────────────────
        for (pos, handle, radius) in [
            (Vec3::new( 8.0,  2.0, 68.0), h_c_tree1, 2.0_f32 * f32::sqrt(3.0)),
            (Vec3::new(18.0,  1.5, 76.0), h_c_tree2, 1.5_f32 * f32::sqrt(3.0)),
            (Vec3::new(10.0,  1.0, 83.0), h_c_tree3, 1.0_f32 * f32::sqrt(3.0)),
            (Vec3::new(24.0,  3.0, 70.0), h_c_tree4, 3.0_f32 * f32::sqrt(3.0)),
            (Vec3::new(14.0,  0.8, 79.0), h_c_tree5, 0.8_f32 * f32::sqrt(3.0)),
        ] {
            level.spawn_entity(
                Components::new()
                    .with_transform(Transform::from_position(pos))
                    .with_mesh(handle)
                    .with_material(h_cube_mat)
                    .with_bounding_radius(radius)
                    .with_tag("zone_c"),
            );
        }

        // ── Zone D — Void (blood red / violet) ───────────────────────────────
        for (pos, handle, radius) in [
            (Vec3::new(75.0,  6.0, 75.0), h_d_mono,  6.0_f32 * f32::sqrt(3.0)),
            (Vec3::new(63.0,  0.8, 63.0), h_d_rock1, 0.8_f32 * f32::sqrt(3.0)),
            (Vec3::new(84.0,  0.7, 83.0), h_d_rock2, 0.7_f32 * f32::sqrt(3.0)),
            (Vec3::new(70.0,  0.5, 83.0), h_d_rock3, 0.5_f32 * f32::sqrt(3.0)),
        ] {
            level.spawn_entity(
                Components::new()
                    .with_transform(Transform::from_position(pos))
                    .with_mesh(handle)
                    .with_material(h_cube_mat)
                    .with_bounding_radius(radius)
                    .with_tag("zone_d"),
            );
        }

        // ── Orbiting lights with billboards ───────────────────────────────────
        //
        // Each light entity carries:
        //  * Transform   — updated every frame
        //  * LightData   — point light casting and shading
        //  * BillboardData — camera-facing halo sprite
        //
        // Billboard size [2.5, 2.5] m world-space. The bridge translates this
        // into a `BillboardInstance` each frame. Bloom at threshold 0.8 will
        // produce a visible glow even on the 1×1 white fallback sprite.

        let mut zone_lights: Vec<ZoneLight> = Vec::new();

        // Helper: spawn one orbiting light and record it for animation.
        macro_rules! spawn_light {
            ($center:expr, $radius:expr, $height:expr, $speed:expr, $phase:expr,
             $color:expr, $intensity:expr, $range:expr) => {{
                let spawn_pos = Vec3::new(
                    $center.x + ($phase as f32).cos() * $radius,
                    $height,
                    $center.z + ($phase as f32).sin() * $radius,
                );
                let eid = level.spawn_entity(
                    Components::new()
                        .with_transform(Transform::from_position(spawn_pos))
                        .with_light(LightData::Point {
                            color:     $color,
                            intensity: $intensity,
                            range:     $range,
                        })
                        .with_billboard(BillboardData::new(2.5, 2.5, [
                            $color[0], $color[1], $color[2], 0.92,
                        ]))
                        .with_tag("light"),
                );
                zone_lights.push(ZoneLight {
                    entity_id: eid,
                    center:    $center,
                    radius:    $radius,
                    height:    $height,
                    speed:     $speed,
                    phase:     $phase,
                });
            }};
        }

        // Zone A — City: warm orange / amber (orbit r=7 m, stays within x0..30, z0..30)
        spawn_light!(ZONE_A, 7.0, 5.0, 0.80, 0.00, [1.00, 0.55, 0.15], 10.0, 14.0);
        spawn_light!(ZONE_A, 7.0, 4.5, 0.60, 2.09, [1.00, 0.80, 0.20], 9.0,  12.0);
        spawn_light!(ZONE_A, 7.0, 6.5, 1.10, 4.19, [1.00, 0.92, 0.70], 8.0,  13.0);

        // Zone B — Factory: cool blue / cyan / violet (orbit r=7 m)
        spawn_light!(ZONE_B, 7.0, 5.0, 0.70, 0.00, [0.20, 0.45, 1.00], 10.0, 14.0);
        spawn_light!(ZONE_B, 7.0, 4.0, 0.50, 2.09, [0.10, 0.80, 1.00], 9.0,  12.0);
        spawn_light!(ZONE_B, 7.0, 6.0, 0.90, 4.19, [0.50, 0.30, 1.00], 8.0,  13.0);

        // Zone C — Forest: emerald / lime / teal (orbit r=6 m)
        spawn_light!(ZONE_C, 6.0, 4.0, 0.65, 0.00, [0.10, 0.90, 0.20], 9.0,  12.0);
        spawn_light!(ZONE_C, 6.0, 3.5, 0.85, 2.09, [0.40, 1.00, 0.10], 8.0,  11.0);
        spawn_light!(ZONE_C, 6.0, 5.5, 1.05, 4.19, [0.00, 0.80, 0.50], 9.0,  13.0);

        // Zone D — Void: blood red / magenta / crimson (orbit r=6 m)
        spawn_light!(ZONE_D, 6.0, 5.0, 0.90, 0.00, [1.00, 0.10, 0.10], 10.0, 13.0);
        spawn_light!(ZONE_D, 6.0, 4.0, 0.70, 2.09, [0.90, 0.00, 0.70], 9.0,  12.0);
        spawn_light!(ZONE_D, 6.0, 6.5, 1.20, 4.19, [0.70, 0.00, 0.20], 8.0,  14.0);

        // ── Activate all chunks for initial frame ─────────────────────────────
        // Streaming-driven activation (via tick) will take over once the camera
        // starts moving. Force-activate here so the first frame has content.
        level.activate_all_chunks();

        // ── Camera registry ───────────────────────────────────────────────────
        //
        // Three cameras:
        //  1. main_cam     — GameCamera, free-fly perspective, starts at edge
        //  2. overview_cam — GameCamera, top-down orthographic, centre above world
        //  3. editor_cam   — EditorPerspective, same start as main (Tab to access)
        //
        // Only main_cam starts as active Game camera.

        let main_cam_id = stratum.register_camera(StratumCamera {
            id:            CameraId::PLACEHOLDER,
            kind:          CameraKind::GameCamera { tag: "main".into() },
            position:      Vec3::new(45.0, 15.0, 95.0),
            yaw:           0.0,
            pitch:         -0.28,
            projection:    Projection::perspective(
                               std::f32::consts::FRAC_PI_3, // 60° FOV
                               0.1, 500.0,
                           ),
            render_target: RenderTargetHandle::PrimarySurface,
            viewport:      Viewport::full(),
            priority:      0,
            active:        true,
        });

        // Overview camera — initially inactive; activate via F2.
        let overview_cam_id = stratum.register_camera(StratumCamera {
            id:            CameraId::PLACEHOLDER,
            kind:          CameraKind::GameCamera { tag: "overview".into() },
            position:      Vec3::new(45.0, 140.0, 45.0),
            yaw:           0.0,
            pitch:         -1.50, // close to straight-down (avoids gimbal singularity)
            projection:    Projection::orthographic_symmetric(60.0, 60.0, 1.0, 300.0),
            render_target: RenderTargetHandle::PrimarySurface,
            viewport:      Viewport::full(),
            priority:      0,
            active:        false,
        });

        // Editor camera — only renders when Tab has switched to Editor mode.
        let editor_cam_id = stratum.register_camera(StratumCamera {
            id:            CameraId::PLACEHOLDER,
            kind:          CameraKind::EditorPerspective,
            position:      Vec3::new(45.0, 15.0, 95.0),
            yaw:           0.0,
            pitch:         -0.28,
            projection:    Projection::perspective(
                               std::f32::consts::FRAC_PI_3,
                               0.1, 500.0,
                           ),
            render_target: RenderTargetHandle::PrimarySurface,
            viewport:      Viewport::full(),
            priority:      0,
            active:        true,
        });

        // ── Multiview cameras (inactive until 2 is pressed) ───────────────────
        // World spans ~90 m; ortho half-height 55 covers it with margin.
        const ORTHO_HALF: f32 = 55.0;
        const ORTHO_DIST: f32 = 150.0;
        let world_center = Vec3::new(45.0, 0.0, 45.0);

        let cam_top_id = stratum.register_camera(StratumCamera {
            id:            CameraId::PLACEHOLDER,
            kind:          CameraKind::GameCamera { tag: "multiview_top".into() },
            position:      world_center + Vec3::Y * ORTHO_DIST,
            yaw:           0.0,
            pitch:         -std::f32::consts::FRAC_PI_2,
            projection:    Projection::orthographic_symmetric(ORTHO_HALF, ORTHO_HALF, 0.1, 500.0),
            render_target: RenderTargetHandle::OffscreenTexture(MV_NAMES[1].into()),
            viewport:      Viewport::full(),
            priority:      1,
            active:        false,
        });

        let cam_side_id = stratum.register_camera(StratumCamera {
            id:            CameraId::PLACEHOLDER,
            kind:          CameraKind::GameCamera { tag: "multiview_side".into() },
            position:      world_center + Vec3::X * ORTHO_DIST,
            yaw:           -std::f32::consts::FRAC_PI_2,
            pitch:         0.0,
            projection:    Projection::orthographic_symmetric(ORTHO_HALF, ORTHO_HALF, 0.1, 500.0),
            render_target: RenderTargetHandle::OffscreenTexture(MV_NAMES[2].into()),
            viewport:      Viewport::full(),
            priority:      2,
            active:        false,
        });

        let cam_front_id = stratum.register_camera(StratumCamera {
            id:            CameraId::PLACEHOLDER,
            kind:          CameraKind::GameCamera { tag: "multiview_front".into() },
            position:      world_center + Vec3::Z * ORTHO_DIST,
            yaw:           std::f32::consts::PI,
            pitch:         0.0,
            projection:    Projection::orthographic_symmetric(ORTHO_HALF, ORTHO_HALF, 0.1, 500.0),
            render_target: RenderTargetHandle::OffscreenTexture(MV_NAMES[3].into()),
            viewport:      Viewport::full(),
            priority:      3,
            active:        false,
        });

        log::info!(
            "World ready — {} entities across 4 zones | {} orbiting lights",
            stratum.active_level().map(|l| l.entities().len()).unwrap_or(0),
            zone_lights.len(),
        );

        self.state = Some(AppState {
            window,
            surface,
            device,
            queue,
            surface_format,
            stratum,
            integration,
            main_cam_id,
            overview_cam_id,
            editor_cam_id,
            cam_top_id,
            cam_side_id,
            cam_front_id,
            multiview_active: false,
            mv_textures:      None,
            zone_lights,
            time:           0.0,
            view_mode:      ViewMode::Main,
            last_frame:     std::time::Instant::now(),
            keys:           HashSet::new(),
            cursor_grabbed: false,
            mouse_delta:    (0.0, 0.0),
            show_partition_debug: true,
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
            WindowEvent::CloseRequested => {
                log::info!("Close requested — exiting");
                event_loop.exit();
            }

            // ── Escape: release cursor or exit ────────────────────────────────
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

            // ── F1 — main perspective camera ─────────────────────────────────
            WindowEvent::KeyboardInput {
                event: KeyEvent {
                    state:        ElementState::Pressed,
                    physical_key: PhysicalKey::Code(KeyCode::F1),
                    ..
                },
                ..
            } => {
                state.set_game_camera(ViewMode::Main);
            }

            // ── F2 — top-down overview camera ─────────────────────────────────
            WindowEvent::KeyboardInput {
                event: KeyEvent {
                    state:        ElementState::Pressed,
                    physical_key: PhysicalKey::Code(KeyCode::F2),
                    ..
                },
                ..
            } => {
                state.set_game_camera(ViewMode::Overview);
            }

            // ── F3 — partition / entity debug dump ────────────────────────────
            WindowEvent::KeyboardInput {
                event: KeyEvent {
                    state:        ElementState::Pressed,
                    physical_key: PhysicalKey::Code(KeyCode::F3),
                    ..
                },
                ..
            } => {
                state.print_debug_stats();
            }

            // ── Generic key hold ──────────────────────────────────────────────
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
                if ks == ElementState::Pressed {
                    match key {
                        KeyCode::Digit1 => state.set_multiview(false),
                        KeyCode::Digit2 => state.set_multiview(true),
                        KeyCode::KeyP   => state.show_partition_debug = !state.show_partition_debug,
                        _ => {}
                    }
                }
            }

            // ── Mouse grab on click ───────────────────────────────────────────
            WindowEvent::MouseInput {
                state:  ElementState::Pressed,
                button: MouseButton::Left,
                ..
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

            // ── Resize ────────────────────────────────────────────────────────
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
                if state.multiview_active {
                    state.rebuild_mv_textures(s.width, s.height);
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
        if let Some(s) = &self.state {
            s.window.request_redraw();
        }
    }
}

// ── Per-frame render logic ────────────────────────────────────────────────────

impl AppState {
    fn render(&mut self, dt: f32) {
        const SPEED:     f32 = 8.0;
        const LOOK_SENS: f32 = 0.002;

        self.time += dt;

        // ── 1. Animate orbiting lights ────────────────────────────────────────
        //
        // Update each light entity's Transform every frame so the light and its
        // billboard both track the orbit. The partition system keeps lights in
        // their spawn chunk (orbit radius fits within chunk boundary), so no
        // re-partitioning is required.
        {
            let level = self.stratum.active_level_mut()
                .expect("active level");

            for zl in &self.zone_lights {
                let angle = self.time * zl.speed + zl.phase;
                let new_pos = Vec3::new(
                    zl.center.x + angle.cos() * zl.radius,
                    zl.height,
                    zl.center.z + angle.sin() * zl.radius,
                );
                if let Some(comp) = level.entities_mut().get_mut(zl.entity_id) {
                    if let Some(t) = &mut comp.transform {
                        t.position = new_pos;
                    }
                }
            }
        }

        // ── 2. Move the active main camera from input ─────────────────────────
        //
        // Only update the perspective camera — the orthographic overview is
        // fixed in place. Mouse look applies to whichever mode is in use.
        if self.view_mode == ViewMode::Main {
            let cam = self.stratum.cameras_mut()
                .get_mut(self.main_cam_id)
                .expect("main camera");

            cam.yaw   += self.mouse_delta.0 * LOOK_SENS;
            cam.pitch  = (cam.pitch - self.mouse_delta.1 * LOOK_SENS).clamp(-1.5, 1.5);

            let forward = cam.forward();
            let right   = cam.right();
            let keys    = &self.keys;
            if keys.contains(&KeyCode::KeyW)      { cam.position += forward * SPEED * dt; }
            if keys.contains(&KeyCode::KeyS)      { cam.position -= forward * SPEED * dt; }
            if keys.contains(&KeyCode::KeyA)      { cam.position -= right   * SPEED * dt; }
            if keys.contains(&KeyCode::KeyD)      { cam.position += right   * SPEED * dt; }
            if keys.contains(&KeyCode::Space)     { cam.position += Vec3::Y * SPEED * dt; }
            if keys.contains(&KeyCode::ShiftLeft) { cam.position -= Vec3::Y * SPEED * dt; }
        }
        self.mouse_delta = (0.0, 0.0);

        // ── 2b. Track fixed multiview cameras onto the main camera position ────
        if self.multiview_active {
            const MV_DIST: f32 = 150.0;
            let main_pos = self.stratum.cameras()
                .get(self.main_cam_id)
                .map(|c| c.position)
                .unwrap_or(Vec3::ZERO);

            if let Some(cam) = self.stratum.cameras_mut().get_mut(self.cam_top_id) {
                cam.position = main_pos + Vec3::Y * MV_DIST;
            }
            if let Some(cam) = self.stratum.cameras_mut().get_mut(self.cam_side_id) {
                cam.position = main_pos + Vec3::X * MV_DIST;
            }
            if let Some(cam) = self.stratum.cameras_mut().get_mut(self.cam_front_id) {
                cam.position = main_pos + Vec3::Z * MV_DIST;
            }
        }

        // ── 3. Tick world ─────────────────────────────────────────────────────
        self.stratum.tick(dt);

        // ── 4. Build render views ─────────────────────────────────────────────
        let size  = self.window.inner_size();
        let views = self.stratum.build_views(size.width, size.height, self.time);

        if views.is_empty() {
            log::debug!("No active cameras for mode {:?}", self.stratum.mode());
            return;
        }

        // ── 5. Acquire swapchain image ────────────────────────────────────────
        let output = match self.surface.get_current_texture() {
            Ok(t)  => t,
            Err(e) => { log::warn!("Surface error: {e:?}"); return; }
        };
        let surface_view = output.texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // ── 6. Submit to Helio ────────────────────────────────────────────────
        let level = self.stratum.active_level()
            .expect("active level exists when views are produced");

        if self.show_partition_debug {
            self.integration.debug_draw_world_partition(level.partition());
        }

        if let Err(e) = self.integration.submit_frame(&views, level, &surface_view, dt) {
            log::error!("Render error: {e:?}");
        }

        // ── 7. Multiview composite ────────────────────────────────────────────
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

    // ── Camera switching ─────────────────────────────────────────────────────

    fn set_multiview(&mut self, active: bool) {
        if self.multiview_active == active { return; }
        self.multiview_active = active;

        if active {
            let size = self.window.inner_size();
            self.rebuild_mv_textures(size.width, size.height);

            if let Some(cam) = self.stratum.cameras_mut().get_mut(self.main_cam_id) {
                cam.render_target = RenderTargetHandle::OffscreenTexture(MV_NAMES[0].into());
            }
            for &id in &[self.cam_top_id, self.cam_side_id, self.cam_front_id] {
                if let Some(cam) = self.stratum.cameras_mut().get_mut(id) {
                    cam.active = true;
                }
            }
            log::info!("Multiview ON");
        } else {
            if let Some(cam) = self.stratum.cameras_mut().get_mut(self.main_cam_id) {
                cam.render_target = RenderTargetHandle::PrimarySurface;
            }
            for &id in &[self.cam_top_id, self.cam_side_id, self.cam_front_id] {
                if let Some(cam) = self.stratum.cameras_mut().get_mut(id) {
                    cam.active = false;
                }
            }
            for name in &MV_NAMES {
                self.integration.unregister_offscreen_view(name);
            }
            self.mv_textures = None;
            log::info!("Multiview OFF");
        }
    }

    fn rebuild_mv_textures(&mut self, width: u32, height: u32) {
        for name in &MV_NAMES {
            self.integration.unregister_offscreen_view(name);
        }
        let mv = MultiviewTextures::new(&self.device, self.surface_format, width, height);
        let render_views = mv.make_render_views();
        for (i, view) in render_views.into_iter().enumerate() {
            self.integration.register_offscreen_view(MV_NAMES[i], view);
        }
        self.mv_textures = Some(mv);

        let qw = (width  / 2) as f32;
        let qh = (height / 2) as f32;
        let aspect = qw / qh;
        const ORTHO_HALF_H: f32 = 55.0;
        let ortho_half_w = ORTHO_HALF_H * aspect;
        for &id in &[self.cam_top_id, self.cam_side_id, self.cam_front_id] {
            if let Some(cam) = self.stratum.cameras_mut().get_mut(id) {
                cam.projection = Projection::orthographic_symmetric(ortho_half_w, ORTHO_HALF_H, 0.1, 500.0);
            }
        }
    }

    /// Only affects Game-mode cameras; editor camera is controlled by Tab.
    fn set_game_camera(&mut self, mode: ViewMode) {
        if self.view_mode == mode { return; }
        self.view_mode = mode;

        let main_active     = mode == ViewMode::Main;
        let overview_active = mode == ViewMode::Overview;

        if let Some(cam) = self.stratum.cameras_mut().get_mut(self.main_cam_id) {
            cam.active = main_active;
        }
        if let Some(cam) = self.stratum.cameras_mut().get_mut(self.overview_cam_id) {
            cam.active = overview_active;
        }

        log::info!("Game camera → {:?}", mode);
    }

    // ── Debug / diagnostic output ─────────────────────────────────────────────

    /// Print a snapshot of world-partition state (F3).
    fn print_debug_stats(&self) {
        let Some(level) = self.stratum.active_level() else {
            log::info!("[F3] No active level");
            return;
        };

        let part          = level.partition();
        let total_chunks  = part.chunks().count();
        let active_chunks = part.active_chunks().count();
        let total_ents    = level.entities().len();
        let active_ents   = part.active_entities().len();

        let cam_pos = self.stratum.cameras()
            .get(self.main_cam_id)
            .map(|c| c.position)
            .unwrap_or_default();

        log::info!(
            "═══ Partition Debug ════════════════════════════════\n\
             Mode        : {:?}\n\
             Camera pos  : ({:.1}, {:.1}, {:.1})\n\
             Chunks      : {active_chunks} / {total_chunks} active\n\
             Entities    : {active_ents} / {total_ents} in active chunks\n\
             Orbiting    : {} animated lights\n\
             ────────────────────────────────────────────────────\n\
             Zone A  (City   ) chunk (0,0,0) state : {}\n\
             Zone B  (Factory) chunk (2,0,0) state : {}\n\
             Zone C  (Forest ) chunk (0,0,2) state : {}\n\
             Zone D  (Void   ) chunk (2,0,2) state : {}",
            self.stratum.mode(),
            cam_pos.x, cam_pos.y, cam_pos.z,
            self.zone_lights.len(),
            chunk_state_str(level, 0, 0, 0),
            chunk_state_str(level, 2, 0, 0),
            chunk_state_str(level, 0, 0, 2),
            chunk_state_str(level, 2, 0, 2),
        );
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Return a display string for the state of a specific chunk coordinate.
fn chunk_state_str(level: &stratum::Level, x: i32, y: i32, z: i32) -> &'static str {
    use stratum::ChunkCoord;
    use stratum::chunk::ChunkState;
    let coord = ChunkCoord::new(x, y, z);
    let state = level.partition()
        .chunks()
        .find(|c| c.coord == coord)
        .map(|c| c.state)
        .unwrap_or(ChunkState::Unloaded);
    match state {
        ChunkState::Active   => "Active",
        ChunkState::Unloaded => "Unloaded",
        ChunkState::Loading  => "Loading",
        ChunkState::Unloading => "Unloading",
    }
}
