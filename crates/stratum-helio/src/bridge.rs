//! Translation layer: Stratum `RenderView` → Helio `Camera`, plus data
//! conversion helpers used by `HelioIntegration`.
//!
//! These are pure, side-effect-free functions. All GPU state is owned by
//! `HelioIntegration`; this module only converts data shapes.
//!
//! Mesh objects are **not** built here. `HelioIntegration` registers them once
//! via `add_object` and removes them via `remove_object`; transforms are
//! updated each frame via `update_transform`.
//!
//! Lights and billboards are tracked persistently by `HelioIntegration` via
//! `add_light`/`remove_light`/`update_light` and the equivalent billboard APIs.
//! Sky atmosphere and skylight are set via `set_sky_atmosphere` / `set_skylight`
//! only when their data changes.

use stratum::{RenderView, LightData, SkylightData, SkyAtmosphereData};
use helio_render_v2::{Camera, SceneLight};
use helio_render_v2::scene::{Skylight, SkyAtmosphere};

// ── Camera ────────────────────────────────────────────────────────────────────

/// Build a Helio `Camera` from a `RenderView`.
///
/// Helio's `Camera` is just (view_proj, position, time) — exactly what
/// `RenderView` carries.
pub fn render_view_to_camera(view: &RenderView) -> Camera {
    Camera::new(view.view_proj, view.camera_position, view.time)
}

// ── Skylight / Sky Atmosphere conversion ──────────────────────────────────────

pub(crate) fn stratum_skylight_to_helio(s: &SkylightData) -> Skylight {
    Skylight::new()
        .with_intensity(s.intensity)
        .with_tint(s.color_tint)
}

pub(crate) fn stratum_sky_atmosphere_to_helio(a: &SkyAtmosphereData) -> SkyAtmosphere {
    SkyAtmosphere {
        rayleigh_scatter: a.rayleigh_scatter,
        rayleigh_h_scale: a.rayleigh_h_scale,
        mie_scatter:      a.mie_scatter,
        mie_h_scale:      a.mie_h_scale,
        mie_g:            a.mie_g,
        sun_intensity:    a.sun_intensity,
        sun_disk_angle:   a.sun_disk_angle,
        earth_radius:     a.earth_radius,
        atm_radius:       a.atm_radius,
        exposure:         a.exposure,
        clouds:           None,
    }
}

// ── Light conversion ──────────────────────────────────────────────────────────

pub(crate) fn stratum_light_to_scene_light(light: &LightData, position: [f32; 3]) -> SceneLight {
    match light {
        LightData::Point { color, intensity, range } => {
            SceneLight::point(position, *color, *intensity, *range)
        }

        LightData::Directional { direction, color, intensity } => {
            SceneLight::directional(*direction, *color, *intensity)
        }

        LightData::Spot { direction, color, intensity, range, inner_angle, outer_angle } => {
            SceneLight::spot(position, *direction, *color, *intensity, *range, *inner_angle, *outer_angle)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Vec3};
    use stratum::camera::CameraId;
    use stratum::render_view::{RenderTargetHandle, Viewport};
    use stratum::RenderView;

    fn make_render_view(view_proj: Mat4, pos: Vec3) -> RenderView {
        RenderView {
            camera_id:        CameraId::new(1),
            view_proj,
            camera_position:  pos,
            time:             1.234,
            render_target:    RenderTargetHandle::PrimarySurface,
            viewport:         Viewport::full(),
            visible_entities: vec![],
            priority:         0,
        }
    }

    // ── render_view_to_camera ─────────────────────────────────────────────────

    #[test]
    fn camera_position_is_preserved() {
        let vp  = Mat4::IDENTITY;
        let pos = Vec3::new(3.0, 5.0, -2.0);
        let view = make_render_view(vp, pos);
        let cam  = render_view_to_camera(&view);
        assert_eq!(cam.position, pos);
    }

    #[test]
    fn camera_view_proj_is_preserved() {
        use std::f32::consts::FRAC_PI_4;
        let proj = Mat4::perspective_rh(FRAC_PI_4, 16.0 / 9.0, 0.1, 1000.0);
        let view = Mat4::look_at_rh(Vec3::new(0.0, 2.0, 5.0), Vec3::ZERO, Vec3::Y);
        let vp   = proj * view;
        let rv   = make_render_view(vp, Vec3::new(0.0, 2.0, 5.0));
        let cam  = render_view_to_camera(&rv);
        assert_eq!(cam.view_proj, vp);
    }

    #[test]
    fn camera_time_is_preserved() {
        let rv  = make_render_view(Mat4::IDENTITY, Vec3::ZERO);
        let cam = render_view_to_camera(&rv);
        assert!((cam.time - 1.234).abs() < 1e-6);
    }
}
