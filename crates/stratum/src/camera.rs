//! Camera types — every viewport into the world is a `StratumCamera`.
//!
//! Cameras are first-class citizens in Stratum: they live in the
//! `CameraRegistry` at the `Stratum` level, independent of any `Level` or
//! entity. This allows editor cameras to exist without a loaded level, and
//! enables multi-window / split-screen layouts where one camera looks into
//! a different level than another.

use glam::{Mat4, Vec3};
use crate::render_view::{RenderTargetHandle, Viewport};

// ── CameraId ──────────────────────────────────────────────────────────────────

/// Unique identifier for a camera in the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CameraId(u64);

impl CameraId {
    /// A zero-valued placeholder. The registry overwrites this on registration.
    pub const PLACEHOLDER: CameraId = CameraId(0);

    #[inline] pub fn new(val: u64) -> Self { Self(val) }
    #[inline] pub fn raw(self)     -> u64  { self.0 }
}

impl std::fmt::Display for CameraId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Camera({})", self.0)
    }
}

// ── CameraKind ────────────────────────────────────────────────────────────────

/// Semantic role of a camera — controls visibility per `SimulationMode`.
///
/// | Kind                   | Editor mode | Game mode |
/// |------------------------|-------------|-----------|
/// | `EditorPerspective`    | ✓ renders   | ✗ hidden  |
/// | `EditorOrthographic`   | ✓ renders   | ✗ hidden  |
/// | `GameCamera`           | ✗ hidden    | ✓ renders |
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CameraKind {
    /// A perspective viewport used in the editor (3-D scene view).
    EditorPerspective,
    /// An orthographic viewport used in the editor (top / side / front views).
    EditorOrthographic,
    /// An in-game camera rendered during gameplay.
    GameCamera {
        /// Logical slot name (e.g. `"main"`, `"minimap"`, `"splitscreen_p2"`).
        tag: String,
    },
}

// ── Projection ────────────────────────────────────────────────────────────────

/// Projection parameters for a `StratumCamera`.
#[derive(Debug, Clone)]
pub enum Projection {
    Perspective {
        /// Vertical field-of-view in radians.
        fov_y: f32,
        near:  f32,
        far:   f32,
    },
    Orthographic {
        left:   f32,
        right:  f32,
        bottom: f32,
        top:    f32,
        near:   f32,
        far:    f32,
    },
}

impl Projection {
    pub fn perspective(fov_y: f32, near: f32, far: f32) -> Self {
        Self::Perspective { fov_y, near, far }
    }

    pub fn orthographic_symmetric(half_w: f32, half_h: f32, near: f32, far: f32) -> Self {
        Self::Orthographic {
            left: -half_w, right:  half_w,
            bottom: -half_h, top: half_h,
            near, far,
        }
    }

    /// Build the projection matrix.
    ///
    /// `aspect` is the width-to-height ratio of the target viewport.
    /// Orthographic projections bake l/r/b/t and ignore `aspect`.
    pub fn to_matrix(&self, aspect: f32) -> Mat4 {
        match *self {
            Self::Perspective { fov_y, near, far } =>
                Mat4::perspective_rh(fov_y, aspect, near, far),
            Self::Orthographic { left, right, bottom, top, near, far } =>
                Mat4::orthographic_rh(left, right, bottom, top, near, far),
        }
    }
}

// ── StratumCamera ─────────────────────────────────────────────────────────────

/// One camera registered with Stratum.
///
/// Cameras drive two things:
/// 1. Partition activation (which chunks are resident).
/// 2. `RenderView` production (what Helio renders each frame).
///
/// The `id` field is assigned by `CameraRegistry::register()`; any value set
/// before registration is overwritten.
pub struct StratumCamera {
    pub id:            CameraId,
    pub kind:          CameraKind,
    /// Eye position in world space.
    pub position:      Vec3,
    /// Yaw (horizontal) rotation in radians. 0 = looking down −Z.
    pub yaw:           f32,
    /// Pitch (vertical) rotation in radians. Caller is responsible for clamping.
    pub pitch:         f32,
    pub projection:    Projection,
    pub render_target: RenderTargetHandle,
    /// Normalized viewport rectangle (0..1 relative to render target size).
    pub viewport:      Viewport,
    /// Draw priority — lower values render first (background before foreground).
    pub priority:      i32,
    /// When `false` this camera is excluded from `Stratum::build_views`.
    pub active:        bool,
}

impl StratumCamera {
    /// Compute the forward direction from yaw + pitch (standard FPS convention).
    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sy * cp, sp, -cy * cp)
    }

    /// Right vector (perpendicular to forward, lying in the XZ plane).
    pub fn right(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(cy, 0.0, sy)
    }

    /// Build the view matrix (right-hand, Y-up).
    pub fn view_matrix(&self) -> Mat4 {
        let forward = self.forward();
        Mat4::look_at_rh(self.position, self.position + forward, Vec3::Y)
    }

    /// Build the projection matrix for a given viewport aspect ratio.
    pub fn proj_matrix(&self, aspect: f32) -> Mat4 {
        self.projection.to_matrix(aspect)
    }

    /// Combined view-projection matrix.
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.proj_matrix(aspect) * self.view_matrix()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};
    use crate::render_view::{RenderTargetHandle, Viewport};

    fn default_camera(yaw: f32, pitch: f32) -> StratumCamera {
        StratumCamera {
            id:            CameraId::PLACEHOLDER,
            kind:          CameraKind::GameCamera { tag: "test".into() },
            position:      Vec3::ZERO,
            yaw,
            pitch,
            projection:    Projection::perspective(FRAC_PI_4, 0.1, 100.0),
            render_target: RenderTargetHandle::PrimarySurface,
            viewport:      Viewport::full(),
            priority:      0,
            active:        true,
        }
    }

    // ── Projection::to_matrix ─────────────────────────────────────────────────

    #[test]
    fn perspective_proj_is_finite() {
        let proj = Projection::perspective(FRAC_PI_4, 0.1, 1000.0);
        let m    = proj.to_matrix(16.0 / 9.0);
        // All entries should be finite
        for col in [m.x_axis, m.y_axis, m.z_axis, m.w_axis] {
            assert!(col.x.is_finite());
            assert!(col.y.is_finite());
            assert!(col.z.is_finite());
            assert!(col.w.is_finite());
        }
    }

    #[test]
    fn orthographic_proj_is_finite() {
        let proj = Projection::orthographic_symmetric(10.0, 5.0, 0.1, 100.0);
        let m    = proj.to_matrix(2.0); // aspect ignored for ortho
        for col in [m.x_axis, m.y_axis, m.z_axis, m.w_axis] {
            assert!(col.x.is_finite());
        }
    }

    // ── forward / right vectors ───────────────────────────────────────────────

    #[test]
    fn forward_at_zero_yaw_pitch_points_neg_z() {
        let cam = default_camera(0.0, 0.0);
        let fwd = cam.forward();
        assert!((fwd.x).abs() < 1e-5);
        assert!((fwd.y).abs() < 1e-5);
        assert!((fwd.z + 1.0).abs() < 1e-5, "expected -Z, got {fwd:?}");
    }

    #[test]
    fn forward_yaw_90_points_pos_x() {
        let cam = default_camera(FRAC_PI_2, 0.0);
        let fwd = cam.forward();
        assert!((fwd.x - 1.0).abs() < 1e-5, "expected +X, got {fwd:?}");
        assert!((fwd.y).abs() < 1e-5);
        assert!((fwd.z).abs() < 1e-5);
    }

    #[test]
    fn forward_pitch_90_points_up() {
        let cam = default_camera(0.0, FRAC_PI_2);
        let fwd = cam.forward();
        assert!((fwd.y - 1.0).abs() < 1e-5, "expected +Y, got {fwd:?}");
    }

    #[test]
    fn forward_is_unit_length() {
        for yaw in [0.0f32, 0.7, 1.57, 3.14] {
            for pitch in [-1.0f32, 0.0, 0.5, 1.0] {
                let cam = default_camera(yaw, pitch);
                let len = cam.forward().length();
                assert!((len - 1.0).abs() < 1e-5, "yaw={yaw} pitch={pitch} len={len}");
            }
        }
    }

    #[test]
    fn right_is_unit_length() {
        for yaw in [0.0f32, 0.3, 1.0, 2.5] {
            let cam = default_camera(yaw, 0.0);
            let len = cam.right().length();
            assert!((len - 1.0).abs() < 1e-5);
        }
    }

    // ── view_matrix ───────────────────────────────────────────────────────────

    #[test]
    fn view_matrix_at_origin_zero_angles_is_finite() {
        let cam = default_camera(0.0, 0.0);
        let v   = cam.view_matrix();
        for col in [v.x_axis, v.y_axis, v.z_axis, v.w_axis] {
            assert!(col.x.is_finite());
        }
    }

    // ── view_proj ─────────────────────────────────────────────────────────────

    #[test]
    fn view_proj_changes_with_yaw() {
        let cam_a = default_camera(0.0,       0.0);
        let cam_b = default_camera(FRAC_PI_4, 0.0);
        assert_ne!(cam_a.view_proj(1.0), cam_b.view_proj(1.0));
    }

    #[test]
    fn view_proj_changes_with_position() {
        let mut cam_a = default_camera(0.0, 0.0);
        let mut cam_b = default_camera(0.0, 0.0);
        cam_b.position = Vec3::new(10.0, 0.0, 0.0);
        assert_ne!(cam_a.view_proj(1.0), cam_b.view_proj(1.0));
        let _ = (&mut cam_a, &mut cam_b); // suppress unused mut
    }
}
