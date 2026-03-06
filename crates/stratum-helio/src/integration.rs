//! `HelioIntegration` — Stratum-to-Helio render submission.
//!
//! `HelioIntegration` wraps a Helio `Renderer` and an `AssetRegistry`.
//! Each frame, the host calls `submit_frame()` with the `Vec<RenderView>`
//! produced by `Stratum::build_views()`. For each view the integration:
//!
//! 1. Resolves `RenderTargetHandle` → `&wgpu::TextureView`.
//! 2. Builds a Helio `Scene` from the visible entities.
//! 3. Builds a Helio `Camera` from the view matrices.
//! 4. Calls `renderer.render_scene()`.
//!
//! ## Render target resolution
//!
//! * `RenderTargetHandle::PrimarySurface` → the `wgpu::TextureView` passed
//!   in directly by the caller (the swapchain image acquired each frame).
//! * `RenderTargetHandle::OffscreenTexture(name)` → a `wgpu::TextureView`
//!   registered via `register_offscreen_view`. Falls back to primary surface
//!   if the name is unknown.
//! * `ViewportSlot` → falls back to primary surface.

use std::collections::HashMap;

use glam::{Quat, Vec3};
use stratum::{ChunkState, EntityStore, Level, LightData, RenderTargetHandle, RenderView, WorldPartition};
use helio_render_v2::Renderer;

use helio_render_v2::features::BillboardInstance;

use crate::asset_registry::AssetRegistry;
use crate::bridge::{render_view_to_camera, render_view_to_scene};

/// Owns the Helio renderer and the mesh asset registry, and drives render
/// submission for each frame.
pub struct HelioIntegration {
    renderer:         Renderer,
    assets:           AssetRegistry,
    /// Named offscreen render targets. Populated by the host when
    /// `RenderTargetHandle::OffscreenTexture` cameras are in use.
    offscreen_views:  HashMap<String, wgpu::TextureView>,
    /// Extra billboards injected by the host (e.g. probe visualisation).
    /// Merged into every scene in `submit_frame`.
    extra_billboards: Vec<BillboardInstance>,
}

impl HelioIntegration {
    pub fn new(renderer: Renderer, assets: AssetRegistry) -> Self {
        Self { renderer, assets, offscreen_views: HashMap::new(), extra_billboards: Vec::new() }
    }

    // ── Accessors ─────────────────────────────────────────────────────────────

    pub fn renderer    (&self)     -> &Renderer         { &self.renderer }
    pub fn renderer_mut(&mut self) -> &mut Renderer     { &mut self.renderer }
    pub fn assets      (&self)     -> &AssetRegistry    { &self.assets }
    pub fn assets_mut  (&mut self) -> &mut AssetRegistry { &mut self.assets }

    // ── Material creation ─────────────────────────────────────────────────────

    /// Create a GPU material from a `helio_render_v2::Material` descriptor and
    /// return its `GpuMaterial`. The result can be stored in the `AssetRegistry`
    /// via `assets_mut().add_material(mat)` to obtain a `MaterialHandle`.
    pub fn create_material(&mut self, material: &helio_render_v2::Material) -> helio_render_v2::GpuMaterial {
        self.renderer.create_material(material)
    }

    // ── Offscreen texture registry ────────────────────────────────────────────

    /// Register a named offscreen `TextureView` as a render target.
    ///
    /// Cameras whose `render_target` is `RenderTargetHandle::OffscreenTexture(name)`
    /// will render to this view. Overwrites any previous registration for `name`.
    pub fn register_offscreen_view(&mut self, name: impl Into<String>, view: wgpu::TextureView) {
        self.offscreen_views.insert(name.into(), view);
    }

    /// Remove a named offscreen view. The contained `TextureView` is dropped.
    pub fn unregister_offscreen_view(&mut self, name: &str) {
        self.offscreen_views.remove(name);
    }

    // ── Extra billboards ──────────────────────────────────────────────────────

    /// Set extra billboards that will be merged into every scene during
    /// `submit_frame`. Useful for debug overlays like RC probe grids.
    pub fn set_extra_billboards(&mut self, billboards: Vec<BillboardInstance>) {
        self.extra_billboards = billboards;
    }

    /// Clear all extra billboards.
    pub fn clear_extra_billboards(&mut self) {
        self.extra_billboards.clear();
    }

    // ── Lifecycle ─────────────────────────────────────────────────────────────

    /// Notify the renderer that the output surface was resized.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.renderer.resize(width, height);
    }

    // ── Debug drawing ─────────────────────────────────────────────────────────

    /// Submit debug wireframe boxes for every chunk in `partition`.
    ///
    /// Color-coding by [`ChunkState`]:
    /// * **Active**    — green  (`[0.0, 1.0, 0.0, 0.4]`)
    /// * **Loading**   — yellow (`[1.0, 1.0, 0.0, 0.4]`)
    /// * **Unloading** — orange (`[1.0, 0.5, 0.0, 0.3]`)
    /// * **Unloaded**  — gray   (`[0.5, 0.5, 0.5, 0.15]`)
    ///
    /// Call this after `submit_frame` (or before — shapes are transient and
    /// cleared automatically by the renderer after each render call).
    pub fn debug_draw_world_partition(&mut self, partition: &WorldPartition) {
        for chunk in partition.chunks() {
            let center       = chunk.bounds.center();
            let half_extents = chunk.bounds.half_extents();
            let color = match chunk.state {
                ChunkState::Active    => [0.0, 1.0, 0.0, 0.4],
                ChunkState::Loading   => [1.0, 1.0, 0.0, 0.4],
                ChunkState::Unloading => [1.0, 0.5, 0.0, 0.3],
                ChunkState::Unloaded  => [0.5, 0.5, 0.5, 0.15],
            };
            self.renderer.debug_box(center, half_extents, Quat::IDENTITY, color, 0.03);
        }
    }

    /// Submit debug attenuation volumes for every light entity in `store`.
    ///
    /// * **Point** light → wireframe sphere at the light position with radius = `range`.
    /// * **Spot**  light → wireframe cone with apex at position, pointing along
    ///   `direction`, height = `range`, base radius = `range * tan(outer_angle)`.
    /// * **Directional** light → three short arrows in the light direction (no
    ///   attenuation to visualise, so just a direction indicator).
    ///
    /// Call before `submit_frame` so shapes are flushed with the same render call.
    pub fn debug_draw_lights(&mut self, store: &EntityStore) {
        for (_id, components) in store.iter() {
            let (Some(light), Some(transform)) = (&components.light, &components.transform)
            else { continue };

            let pos = transform.position;

            match light {
                LightData::Point { color, range, .. } => {
                    let c = [color[0], color[1], color[2], 0.5];
                    self.renderer.debug_sphere(pos, *range, c, 0.03);
                }

                LightData::Spot { direction, color, range, outer_angle, .. } => {
                    let dir = Vec3::from(*direction).normalize_or_zero();
                    let base_radius = range * outer_angle.tan();
                    let c = [color[0], color[1], color[2], 0.55];
                    self.renderer.debug_cone(pos, dir, *range, base_radius, c, 0.03);
                }

                LightData::Directional { direction, color, .. } => {
                    // Three parallel arrow shafts to show direction (no range).
                    let dir  = Vec3::from(*direction).normalize_or_zero();
                    let c    = [color[0], color[1], color[2], 0.6];
                    for offset in [Vec3::ZERO, Vec3::X * 0.4, Vec3::Z * 0.4] {
                        let start = pos + offset;
                        self.renderer.debug_line(start, start + dir * 3.0, c, 0.03);
                    }
                }
            }
        }
    }

    // ── Frame submission ──────────────────────────────────────────────────────
    /// Submit all render views for one frame.
    ///
    /// # Parameters
    ///
    /// | Name                  | Description                                 |
    /// |-----------------------|---------------------------------------------|
    /// | `views`               | Output of `Stratum::build_views()`          |
    /// | `level`               | Active level (entity data for scene build)  |
    /// | `primary_surface`     | The swapchain image acquired this frame     |
    /// | `delta_time`          | Frame delta in seconds                      |
    ///
    /// Views are already sorted by priority when produced by Stratum; this
    /// function submits them in order.
    pub fn submit_frame(
        &mut self,
        views:          &[RenderView],
        level:          &Level,
        primary_surface: &wgpu::TextureView,
        delta_time:     f32,
    ) -> helio_render_v2::Result<()> {
        let store = level.entities();

        for view in views {
            // ── Translate to Helio types ──────────────────────────────────────
            let mut scene = render_view_to_scene(view, store, &self.assets);
            let camera    = render_view_to_camera(view);

            // Merge extra billboards (probe vis, etc.) into the scene.
            if !self.extra_billboards.is_empty() {
                scene.billboards.extend(self.extra_billboards.iter().cloned());
            }

            // ── Resolve render target then submit ─────────────────────────────
            let result = match &view.render_target {
                RenderTargetHandle::PrimarySurface => {
                    self.renderer.render_scene(&scene, &camera, primary_surface, delta_time)
                }
                RenderTargetHandle::OffscreenTexture(name) => {
                    if let Some(offscreen) = self.offscreen_views.get(name.as_str()) {
                        self.renderer.render_scene(&scene, &camera, offscreen, delta_time)
                    } else {
                        log::warn!(
                            "Unresolved offscreen texture '{}' — routing to primary surface",
                            name
                        );
                        self.renderer.render_scene(&scene, &camera, primary_surface, delta_time)
                    }
                }
                other => {
                    log::warn!(
                        "Unresolved render target {:?} — routing to primary surface",
                        other
                    );
                    self.renderer.render_scene(&scene, &camera, primary_surface, delta_time)
                }
            };
            result?;
        }

        Ok(())
    }
}
