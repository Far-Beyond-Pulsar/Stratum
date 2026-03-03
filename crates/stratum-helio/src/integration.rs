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
//! * `OffscreenTexture` / `ViewportSlot` → falls back to primary surface for
//!   now; a texture pool can be added here without touching Stratum or Helio.

use stratum::{Level, RenderTargetHandle, RenderView};
use helio_render_v2::Renderer;

use crate::asset_registry::AssetRegistry;
use crate::bridge::{render_view_to_camera, render_view_to_scene};

/// Owns the Helio renderer and the mesh asset registry, and drives render
/// submission for each frame.
pub struct HelioIntegration {
    renderer: Renderer,
    assets:   AssetRegistry,
}

impl HelioIntegration {
    pub fn new(renderer: Renderer, assets: AssetRegistry) -> Self {
        Self { renderer, assets }
    }

    // ── Accessors ─────────────────────────────────────────────────────────────

    pub fn renderer    (&self)     -> &Renderer         { &self.renderer }
    pub fn renderer_mut(&mut self) -> &mut Renderer     { &mut self.renderer }
    pub fn assets      (&self)     -> &AssetRegistry    { &self.assets }
    pub fn assets_mut  (&mut self) -> &mut AssetRegistry { &mut self.assets }

    // ── Lifecycle ─────────────────────────────────────────────────────────────

    /// Notify the renderer that the output surface was resized.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.renderer.resize(width, height);
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
            // ── Resolve render target ─────────────────────────────────────────
            let target = match &view.render_target {
                RenderTargetHandle::PrimarySurface => primary_surface,
                other => {
                    // Extension point: resolve from an offscreen texture pool.
                    log::warn!(
                        "Unresolved render target {:?} — routing to primary surface",
                        other
                    );
                    primary_surface
                }
            };

            // ── Translate to Helio types ──────────────────────────────────────
            let scene  = render_view_to_scene(view, store, &self.assets);
            let camera = render_view_to_camera(view);

            // ── Submit ────────────────────────────────────────────────────────
            self.renderer.render_scene(&scene, &camera, target, delta_time)?;
        }

        Ok(())
    }
}
