//! Translation layer: Stratum `RenderView` → Helio `Camera`, plus light
//! conversion helpers used by `HelioIntegration`.
//!
//! All functions here are pure (no side effects, no GPU state).
//!
//! ## Camera
//!
//! Helio's new `Camera` requires separate view and projection matrices plus
//! near/far planes for shadow mapping.  `RenderView` now carries all four so
//! `render_view_to_camera` can forward them directly without recomputing.
//!
//! TAA sub-pixel jitter is applied internally by `Renderer::render()` using a
//! 16-sample Halton sequence — callers do not need to supply it.
//!
//! ## Lights
//!
//! `GpuLight` is a flat GPU struct. `stratum_light_to_gpu_light` converts
//! Stratum's ergonomic `LightData` enum into the packed representation.

use glam::Vec3;
use stratum::{LightData, RenderView};
use helio::{Camera, GpuLight, LightType};

// ── Camera ────────────────────────────────────────────────────────────────────

/// Build a Helio `Camera` from a `RenderView`.
///
/// Uses the separately-stored `view`, `proj`, `near`, and `far` fields that
/// `build_render_views` populates.  TAA jitter is applied by `Renderer::render`.
pub fn render_view_to_camera(view: &RenderView) -> Camera {
    Camera::from_matrices(view.view, view.proj, view.camera_position, view.near, view.far)
}

// ── Light conversion ──────────────────────────────────────────────────────────

/// Convert a Stratum `LightData` + world-space position to a Helio `GpuLight`.
pub(crate) fn stratum_light_to_gpu_light(light: &LightData, position: Vec3) -> GpuLight {
    match light {
        LightData::Point { color, intensity, range } => GpuLight {
            position_range:  [position.x, position.y, position.z, *range],
            direction_outer: [0.0, -1.0, 0.0, 0.0],
            color_intensity: [color[0], color[1], color[2], *intensity],
            shadow_index:    u32::MAX,
            light_type:      LightType::Point as u32,
            inner_angle:     0.0,
            _pad:            0,
        },

        LightData::Directional { direction, color, intensity } => {
            let d = Vec3::from(*direction).normalize_or_zero();
            GpuLight {
                position_range:  [0.0, 0.0, 0.0, f32::MAX],
                direction_outer: [d.x, d.y, d.z, 0.0],
                color_intensity: [color[0], color[1], color[2], *intensity],
                shadow_index:    u32::MAX,
                light_type:      LightType::Directional as u32,
                inner_angle:     0.0,
                _pad:            0,
            }
        }

        LightData::Spot { direction, color, intensity, range, inner_angle, outer_angle } => {
            let d = Vec3::from(*direction).normalize_or_zero();
            GpuLight {
                position_range:  [position.x, position.y, position.z, *range],
                direction_outer: [d.x, d.y, d.z, outer_angle.cos()],
                color_intensity: [color[0], color[1], color[2], *intensity],
                shadow_index:    u32::MAX,
                light_type:      LightType::Spot as u32,
                inner_angle:     inner_angle.cos(),
                _pad:            0,
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Vec3};
    use std::f32::consts::FRAC_PI_4;
    use stratum::camera::CameraId;
    use stratum::render_view::{RenderTargetHandle, Viewport};
    use stratum::RenderView;

    fn make_render_view(pos: Vec3) -> RenderView {
        let view = Mat4::look_at_rh(pos, Vec3::ZERO, Vec3::Y);
        let proj = Mat4::perspective_rh(FRAC_PI_4, 16.0 / 9.0, 0.1, 1000.0);
        RenderView {
            camera_id:        CameraId::new(1),
            view,
            proj,
            view_proj:        proj * view,
            camera_position:  pos,
            near:             0.1,
            far:              1000.0,
            render_target:    RenderTargetHandle::PrimarySurface,
            viewport:         Viewport::full(),
            visible_entities: vec![],
            priority:         0,
        }
    }

    #[test]
    fn camera_position_is_preserved() {
        let pos = Vec3::new(3.0, 5.0, -2.0);
        let rv  = make_render_view(pos);
        let cam = render_view_to_camera(&rv);
        assert_eq!(cam.position, pos);
    }

    #[test]
    fn camera_near_far_preserved() {
        let rv  = make_render_view(Vec3::new(0.0, 2.0, 5.0));
        let cam = render_view_to_camera(&rv);
        assert!((cam.near - 0.1).abs() < 1e-5);
        assert!((cam.far - 1000.0).abs() < 0.1);
    }

    #[test]
    fn point_light_has_correct_type() {
        let light = LightData::Point { color: [1.0, 0.5, 0.0], intensity: 100.0, range: 10.0 };
        let gpu   = stratum_light_to_gpu_light(&light, Vec3::new(1.0, 2.0, 3.0));
        use helio::LightType;
        assert_eq!(gpu.light_type, LightType::Point as u32);
        assert!((gpu.position_range[3] - 10.0).abs() < 1e-5);
    }

    #[test]
    fn directional_light_has_correct_type() {
        let light = LightData::Directional {
            direction: [0.0, -1.0, 0.0], color: [1.0, 1.0, 1.0], intensity: 3.0,
        };
        let gpu = stratum_light_to_gpu_light(&light, Vec3::ZERO);
        use helio::LightType;
        assert_eq!(gpu.light_type, LightType::Directional as u32);
    }

    #[test]
    fn spot_outer_angle_stored_as_cosine() {
        let outer = std::f32::consts::FRAC_PI_4; // 45°
        let light = LightData::Spot {
            direction: [0.0, -1.0, 0.0], color: [1.0, 1.0, 1.0],
            intensity: 50.0, range: 8.0, inner_angle: 0.2, outer_angle: outer,
        };
        let gpu = stratum_light_to_gpu_light(&light, Vec3::ZERO);
        let expected_cos = outer.cos();
        assert!((gpu.direction_outer[3] - expected_cos).abs() < 1e-5);
    }
}

