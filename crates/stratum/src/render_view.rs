//! Render view descriptor — the contract between Stratum and the renderer.
//!
//! A `RenderView` is the complete, self-contained description of one camera
//! render pass. Stratum produces a `Vec<RenderView>` each frame; the
//! integration crate (`stratum-helio`) consumes it and drives Helio.
//!
//! Helio receives `RenderView`s without knowing anything about Levels,
//! Cameras, or Entities. The abstraction boundary is enforced here.

use glam::{Mat4, Vec3};
use crate::camera::CameraId;
use crate::entity::EntityId;

// ── RenderTargetHandle ────────────────────────────────────────────────────────

/// Identifies a render output destination.
///
/// The integration layer resolves these handles to concrete `wgpu::TextureView`
/// objects before submitting to the renderer.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RenderTargetHandle {
    /// The primary window surface (swapchain image).
    PrimarySurface,
    /// An offscreen texture identified by a logical name.
    /// Used for render-to-texture, reflections, portals, etc.
    OffscreenTexture(String),
    /// A numbered viewport slot for split-screen layouts.
    /// The integration maps slot indices to sub-regions of the surface.
    ViewportSlot(u32),
}

// ── Viewport ──────────────────────────────────────────────────────────────────

/// Normalized viewport rectangle within a render target (coordinates in 0..1).
///
/// | Layout          | x   | y   | width | height |
/// |-----------------|-----|-----|-------|--------|
/// | Full screen     | 0.0 | 0.0 | 1.0   | 1.0    |
/// | Left half       | 0.0 | 0.0 | 0.5   | 1.0    |
/// | Right half      | 0.5 | 0.0 | 0.5   | 1.0    |
/// | Top-right inset | 0.7 | 0.0 | 0.3   | 0.3    |
#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    pub x:      f32,
    pub y:      f32,
    pub width:  f32,
    pub height: f32,
}

impl Viewport {
    pub fn full() -> Self {
        Self { x: 0.0, y: 0.0, width: 1.0, height: 1.0 }
    }

    pub fn left_half() -> Self {
        Self { x: 0.0, y: 0.0, width: 0.5, height: 1.0 }
    }

    pub fn right_half() -> Self {
        Self { x: 0.5, y: 0.0, width: 0.5, height: 1.0 }
    }

    pub fn top_right(normalized_size: f32) -> Self {
        let s = normalized_size.clamp(0.0, 1.0);
        Self { x: 1.0 - s, y: 0.0, width: s, height: s }
    }

    /// Compute the aspect ratio given the actual target pixel dimensions.
    pub fn aspect(&self, target_width: u32, target_height: u32) -> f32 {
        let w = self.width  * target_width  as f32;
        let h = self.height * target_height as f32;
        if h > 0.0 { w / h } else { 1.0 }
    }

    /// Pixel-space origin (top-left) and dimensions for a given target size.
    pub fn to_pixels(&self, target_width: u32, target_height: u32) -> (u32, u32, u32, u32) {
        (
            (self.x      * target_width  as f32) as u32,
            (self.y      * target_height as f32) as u32,
            (self.width  * target_width  as f32).max(1.0) as u32,
            (self.height * target_height as f32).max(1.0) as u32,
        )
    }
}

// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    // ── Viewport arithmetic ───────────────────────────────────────────────────

    #[test]
    fn viewport_full_aspect_16_9() {
        let vp = Viewport::full();
        let aspect = vp.aspect(1920, 1080);
        assert!((aspect - 16.0 / 9.0).abs() < 0.001);
    }

    #[test]
    fn viewport_left_half_aspect_is_8_9() {
        let vp = Viewport::left_half();
        let aspect = vp.aspect(1920, 1080);
        assert!((aspect - 8.0 / 9.0).abs() < 0.01);
    }

    #[test]
    fn viewport_right_half_starts_at_half_width() {
        let vp = Viewport::right_half();
        let (x, _y, _w, _h) = vp.to_pixels(1920, 1080);
        assert_eq!(x, 960);
    }

    #[test]
    fn viewport_top_right_inset() {
        let vp = Viewport::top_right(0.25);
        assert!((vp.x - 0.75).abs() < 1e-5);
        assert!((vp.width - 0.25).abs() < 1e-5);
    }

    #[test]
    fn viewport_to_pixels_full_screen() {
        let vp = Viewport::full();
        let (x, y, w, h) = vp.to_pixels(1280, 720);
        assert_eq!((x, y, w, h), (0, 0, 1280, 720));
    }

    #[test]
    fn viewport_aspect_zero_height_returns_one() {
        let vp = Viewport { x: 0.0, y: 0.0, width: 1.0, height: 0.0 };
        assert_eq!(vp.aspect(100, 100), 1.0);
    }

    #[test]
    fn render_target_handle_equality() {
        let a = RenderTargetHandle::PrimarySurface;
        let b = RenderTargetHandle::PrimarySurface;
        let c = RenderTargetHandle::OffscreenTexture("foo".into());
        let d = RenderTargetHandle::OffscreenTexture("foo".into());
        let e = RenderTargetHandle::ViewportSlot(1);
        assert_eq!(a, b);
        assert_eq!(c, d);
        assert_ne!(a, c);
        assert_ne!(c, e);
    }
}

// ── RenderView ────────────────────────────────────────────────────────────────

/// Complete description of one rendered camera view, produced each frame by
/// `Stratum::build_views()`.
///
/// This is the only thing the renderer (Helio) needs. It contains no Level,
/// Entity, or Camera references — only plain data.
pub struct RenderView {
    /// Which camera produced this view (for diagnostics / post-processing).
    pub camera_id:        CameraId,
    /// Combined view-projection matrix (column-major, right-hand).
    pub view_proj:        Mat4,
    /// Camera eye position in world space (used for lighting attenuation, fog).
    pub camera_position:  Vec3,
    /// Frame time in seconds (forwarded to shader `globals.time`).
    pub time:             f32,
    /// Where to render this view.
    pub render_target:    RenderTargetHandle,
    /// Normalized viewport rectangle within the render target.
    pub viewport:         Viewport,
    /// Entity IDs visible to this camera after frustum + partition culling.
    pub visible_entities: Vec<EntityId>,
    /// Draw priority — lower renders first (skybox/background before scene).
    pub priority:         i32,
}
