//! Translation layer: Stratum `RenderView` + `EntityStore` → Helio `SceneEnv` + `Camera`.
//!
//! These are pure, side-effect-free functions. All GPU state is owned by
//! `HelioIntegration`; this module only converts data shapes.
//!
//! Note: mesh objects are **not** built here. `HelioIntegration` registers them
//! once via `add_object` and removes them via `remove_object`; transforms are
//! updated each frame via `update_transform`. This function only produces the
//! per-frame environment: lights, sky, ambient, and billboards.

use stratum::{RenderView, EntityStore, LightData, SkylightData, SkyAtmosphereData};
use helio_render_v2::{Camera, SceneEnv, SceneLight};
use helio_render_v2::scene::{Skylight, SkyAtmosphere};
use helio_render_v2::features::BillboardInstance;

// ── Camera ────────────────────────────────────────────────────────────────────

/// Build a Helio `Camera` from a `RenderView`.
///
/// Helio's `Camera` is just (view_proj, position, time) — exactly what
/// `RenderView` carries.
pub fn render_view_to_camera(view: &RenderView) -> Camera {
    Camera::new(view.view_proj, view.camera_position, view.time)
}

// ── Scene environment ─────────────────────────────────────────────────────────

/// Build a per-frame Helio `SceneEnv` from the entities visible in a `RenderView`.
///
/// Iterates `view.visible_entities` and collects lights, billboards, and
/// scene-global sky data. Mesh geometry is **not** emitted here — that is
/// handled by `HelioIntegration::sync_entity_objects` which maintains a
/// persistent `add_object` registry. Entities missing required components are
/// silently skipped.
pub fn render_view_to_scene_env(
    view:  &RenderView,
    store: &EntityStore,
) -> SceneEnv {
    let mut lights         = Vec::new();
    let mut billboards     = Vec::new();
    let mut sky_atmosphere = None;
    let mut skylight       = None;

    log::debug!(
        "render_view_to_scene_env: {} visible entities",
        view.visible_entities.len()
    );

    for &entity_id in &view.visible_entities {
        let Some(components) = store.get(entity_id) else { continue };

        // ── Light ─────────────────────────────────────────────────────────────
        if let (Some(light_data), Some(transform)) =
            (&components.light, &components.transform)
        {
            lights.push(stratum_light_to_scene_light(
                light_data,
                transform.position.to_array(),
            ));
        }

        // ── Billboard ─────────────────────────────────────────────────────────
        if let (Some(billboard), Some(transform)) =
            (&components.billboard, &components.transform)
        {
            billboards.push(
                BillboardInstance::new(transform.position.to_array(), billboard.size)
                    .with_color(billboard.color)
                    .with_screen_scale(billboard.screen_scale),
            );
        }

        // ── Skylight / Sky Atmosphere ────────────────────────────────────────
        // Scene-global: first entity with these components wins.
        if sky_atmosphere.is_none() {
            if let Some(sky_atm) = &components.sky_atmosphere {
                sky_atmosphere = Some(stratum_sky_atmosphere_to_helio(sky_atm));
            }
        }
        if skylight.is_none() {
            if let Some(sl) = &components.skylight {
                skylight = Some(stratum_skylight_to_helio(sl));
            }
        }
    }

    log::debug!(
        "render_view_to_scene_env: {} lights {} billboards",
        lights.len(), billboards.len()
    );

    SceneEnv {
        lights,
        billboards,
        sky_atmosphere,
        skylight,
        ..Default::default()
    }
}

// ── Skylight / Sky Atmosphere conversion ──────────────────────────────────────

fn stratum_skylight_to_helio(s: &SkylightData) -> Skylight {
    Skylight::new()
        .with_intensity(s.intensity)
        .with_tint(s.color_tint)
}

fn stratum_sky_atmosphere_to_helio(a: &SkyAtmosphereData) -> SkyAtmosphere {
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

fn stratum_light_to_scene_light(light: &LightData, position: [f32; 3]) -> SceneLight {
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
    use stratum::entity::{Components, EntityId, EntityStore, LightData, Transform};
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

    // ── render_view_to_scene_env ──────────────────────────────────────────────

    #[test]
    fn scene_env_has_no_lights_when_no_visible_entities() {
        let store  = EntityStore::new();
        let rv     = make_render_view(Mat4::IDENTITY, Vec3::ZERO);
        let env    = render_view_to_scene_env(&rv, &store);
        assert!(env.lights.is_empty());
        assert!(env.billboards.is_empty());
    }

    #[test]
    fn scene_env_includes_light_for_light_entity() {
        let mut store = EntityStore::new();
        let id        = store.spawn(
            Components::new()
                .with_transform(Transform::from_position(Vec3::new(0.0, 3.0, 0.0)))
                .with_light(LightData::Point {
                    color: [1.0, 0.5, 0.0], intensity: 5.0, range: 8.0,
                }),
        );
        let rv  = RenderView {
            visible_entities: vec![id],
            ..make_render_view(Mat4::IDENTITY, Vec3::ZERO)
        };
        let env = render_view_to_scene_env(&rv, &store);
        assert_eq!(env.lights.len(), 1);
    }

    #[test]
    fn scene_env_entity_missing_from_store_is_silently_skipped() {
        let store = EntityStore::new(); // empty
        let rv    = RenderView {
            visible_entities: vec![EntityId::new(42)],
            ..make_render_view(Mat4::IDENTITY, Vec3::ZERO)
        };
        // Should not panic
        let env = render_view_to_scene_env(&rv, &store);
        assert!(env.lights.is_empty());
    }
}
