//! Translation layer: Stratum `RenderView` + `EntityStore` → Helio `Scene` + `Camera`.
//!
//! These are pure, side-effect-free functions. All GPU state is owned by
//! `HelioIntegration`; this module only converts data shapes.

use stratum::{RenderView, EntityStore, LightData};
use helio_render_v2::{Camera, Scene, SceneLight};
use helio_render_v2::features::BillboardInstance;
use crate::asset_registry::AssetRegistry;

// ── Camera ────────────────────────────────────────────────────────────────────

/// Build a Helio `Camera` from a `RenderView`.
///
/// Helio's `Camera` is just (view_proj, position, time) — exactly what
/// `RenderView` carries.
pub fn render_view_to_camera(view: &RenderView) -> Camera {
    Camera::new(view.view_proj, view.camera_position, view.time)
}

// ── Scene ─────────────────────────────────────────────────────────────────────

/// Build a Helio `Scene` from the entities visible in a `RenderView`.
///
/// Iterates `view.visible_entities`, looks each one up in `store`, and
/// emits mesh objects and lights. Entities missing either a transform or a
/// renderable component are silently skipped.
pub fn render_view_to_scene(
    view:   &RenderView,
    store:  &EntityStore,
    assets: &AssetRegistry,
) -> Scene {
    let mut scene = Scene::new();

    log::debug!(
        "render_view_to_scene: {} visible entities",
        view.visible_entities.len()
    );

    for &entity_id in &view.visible_entities {
        let Some(components) = store.get(entity_id) else { continue };

        // ── Mesh ──────────────────────────────────────────────────────────────
        if let Some(mesh_handle) = components.mesh {
            if let Some(gpu_mesh) = assets.get(mesh_handle) {
                // Use PBR material bind group when the entity has one registered.
                if let Some(mat_handle) = components.material {
                    if let Some(gpu_mat) = assets.get_material(mat_handle) {
                        scene = scene.add_object_with_material(
                            gpu_mesh.clone(),
                            gpu_mat.clone(),
                        );
                    } else {
                        log::warn!(
                            "Entity {:?} references unregistered MaterialHandle({:?})",
                            entity_id, mat_handle
                        );
                        scene = scene.add_object(gpu_mesh.clone());
                    }
                } else {
                    scene = scene.add_object(gpu_mesh.clone());
                }
            } else {
                log::warn!(
                    "Entity {:?} references unregistered MeshHandle({:?})",
                    entity_id, mesh_handle
                );
            }
        }

        // ── Light ─────────────────────────────────────────────────────────────
        if let (Some(light_data), Some(transform)) =
            (&components.light, &components.transform)
        {
            let scene_light = stratum_light_to_scene_light(
                light_data,
                transform.position.to_array(),
            );
            scene = scene.add_light(scene_light);
        }

        // ── Billboard ─────────────────────────────────────────────────────────
        if let (Some(billboard), Some(transform)) =
            (&components.billboard, &components.transform)
        {
            let bb = BillboardInstance::new(
                transform.position.to_array(),
                billboard.size,
            )
            .with_color(billboard.color)
            .with_screen_scale(billboard.screen_scale);
            scene = scene.add_billboard(bb);
        }
    }

    log::debug!(
        "render_view_to_scene: {} objects {} lights in scene",
        scene.objects.len(),
        scene.lights.len()
    );

    scene
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

        // Helio's public SceneLight API currently has point + directional.
        // Spot lights map to point as a compatible fallback until Helio
        // exposes a spot constructor.
        LightData::Spot { color, intensity, range, .. } => {
            SceneLight::point(position, *color, *intensity, *range)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Vec3};
    use stratum::camera::CameraId;
    use stratum::entity::{Components, EntityId, EntityStore, LightData, MeshHandle, Transform};
    use stratum::render_view::{RenderTargetHandle, Viewport};
    use stratum::RenderView;
    use crate::asset_registry::AssetRegistry;

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

    // ── render_view_to_scene ──────────────────────────────────────────────────

    #[test]
    fn scene_is_empty_when_no_visible_entities() {
        let mut store  = EntityStore::new();
        let _id        = store.spawn(Components::new().with_mesh(MeshHandle(1)));
        let assets     = AssetRegistry::new();
        let rv         = make_render_view(Mat4::IDENTITY, Vec3::ZERO);
        // visible_entities is empty → scene should have no objects
        let scene = render_view_to_scene(&rv, &store, &assets);
        assert!(scene.objects.is_empty());
    }

    #[test]
    fn scene_skips_unregistered_mesh_handle() {
        let mut store  = EntityStore::new();
        let id         = store.spawn(
            Components::new()
                .with_transform(Transform::from_position(Vec3::ZERO))
                .with_mesh(MeshHandle(999)), // not in AssetRegistry
        );
        let assets  = AssetRegistry::new();
        let rv      = RenderView {
            visible_entities: vec![id],
            ..make_render_view(Mat4::IDENTITY, Vec3::ZERO)
        };
        let scene = render_view_to_scene(&rv, &store, &assets);
        assert!(scene.objects.is_empty());
    }

    #[test]
    fn scene_includes_light_for_light_entity() {
        let mut store = EntityStore::new();
        let id        = store.spawn(
            Components::new()
                .with_transform(Transform::from_position(Vec3::new(0.0, 3.0, 0.0)))
                .with_light(LightData::Point {
                    color: [1.0, 0.5, 0.0], intensity: 5.0, range: 8.0,
                }),
        );
        let assets = AssetRegistry::new();
        let rv     = RenderView {
            visible_entities: vec![id],
            ..make_render_view(Mat4::IDENTITY, Vec3::ZERO)
        };
        let scene = render_view_to_scene(&rv, &store, &assets);
        assert_eq!(scene.lights.len(), 1);
    }

    #[test]
    fn scene_entity_missing_from_store_is_silently_skipped() {
        let store   = EntityStore::new(); // empty
        let assets  = AssetRegistry::new();
        let rv      = RenderView {
            visible_entities: vec![EntityId::new(42)],
            ..make_render_view(Mat4::IDENTITY, Vec3::ZERO)
        };
        // Should not panic
        let scene = render_view_to_scene(&rv, &store, &assets);
        assert!(scene.objects.is_empty());
        assert!(scene.lights.is_empty());
    }
}
