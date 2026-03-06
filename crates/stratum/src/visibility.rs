//! Frustum extraction and per-entity visibility culling.
//!
//! Uses the Gribb/Hartmann method to extract six frustum planes from a
//! combined view-projection matrix.
//!
//! ## Coordinate conventions
//!
//! * View space: right-hand Y-up (glam default).
//! * Clip space: wgpu/Vulkan NDC — x ∈ [−w, w], y ∈ [−w, w], z ∈ [0, w].
//! * Planes are stored normalised as `Vec4(nx, ny, nz, d)` where a point P
//!   is *inside* plane i iff `dot(plane_i.xyz, P) + plane_i.w ≥ 0`.

use glam::{Mat4, Vec3, Vec4, Vec4Swizzles};
use crate::entity::{EntityId, EntityStore, LightData};

// ── Frustum ───────────────────────────────────────────────────────────────────

/// Six normalised frustum planes extracted from a view-projection matrix.
pub struct Frustum {
    /// Order: [left, right, bottom, top, near, far]
    planes: [Vec4; 6],
}

/// Normalise the 3-D normal part (xyz) of a plane equation, scaling d (w)
/// by the same factor so the plane equation remains equivalent.
fn normalise_plane(v: Vec4) -> Vec4 {
    let mag = v.xyz().length();
    if mag > 1e-8 { v / mag } else { Vec4::ZERO }
}

/// Extract row `i` from a column-major `Mat4`.
///
/// In glam column-major layout the matrix columns are `.x_axis`, `.y_axis`,
/// `.z_axis`, `.w_axis`. Row i is (col0[i], col1[i], col2[i], col3[i]).
/// Transposing swaps rows and columns, so the i-th column of the transpose
/// is the i-th row of the original.
fn mat_row(m: &Mat4, i: usize) -> Vec4 {
    let t = m.transpose();
    match i {
        0 => t.x_axis,
        1 => t.y_axis,
        2 => t.z_axis,
        3 => t.w_axis,
        _ => panic!("mat_row: index {i} out of range 0..=3"),
    }
}

impl Frustum {
    /// Extract frustum planes from a combined view-projection matrix.
    ///
    /// Works for both perspective and orthographic projections.
    /// Plane derivation (column-vector convention `clip = M * p`):
    ///
    /// ```text
    /// Left:   row3 + row0 ≥ 0   (w + x ≥ 0)
    /// Right:  row3 − row0 ≥ 0   (w − x ≥ 0)
    /// Bottom: row3 + row1 ≥ 0   (w + y ≥ 0)
    /// Top:    row3 − row1 ≥ 0   (w − y ≥ 0)
    /// Near:   row2        ≥ 0   (z ≥ 0, wgpu [0,1] depth)
    /// Far:    row3 − row2 ≥ 0   (w − z ≥ 0)
    /// ```
    pub fn from_view_proj(vp: &Mat4) -> Self {
        let r0 = mat_row(vp, 0);
        let r1 = mat_row(vp, 1);
        let r2 = mat_row(vp, 2);
        let r3 = mat_row(vp, 3);
        Self {
            planes: [
                normalise_plane(r3 + r0), // left
                normalise_plane(r3 - r0), // right
                normalise_plane(r3 + r1), // bottom
                normalise_plane(r3 - r1), // top
                normalise_plane(r2),      // near
                normalise_plane(r3 - r2), // far
            ],
        }
    }

    /// Returns `true` if the point is inside (or on) the frustum.
    #[inline]
    pub fn contains_point(&self, pos: Vec3) -> bool {
        let p = pos.extend(1.0);
        self.planes.iter().all(|&plane| plane.dot(p) >= 0.0)
    }

    /// Returns `true` if the sphere is not *entirely* outside any plane.
    ///
    /// Conservative: a sphere intersecting a plane boundary returns `true`.
    #[inline]
    pub fn intersects_sphere(&self, center: Vec3, radius: f32) -> bool {
        let p = center.extend(1.0);
        self.planes.iter().all(|&plane| plane.dot(p) >= -radius)
    }
}

// ── Visibility culling ────────────────────────────────────────────────────────

/// Cull `candidates` against `frustum` using transform data from `store`.
///
/// An entity is *visible* iff:
/// 1. It has a `Transform` component.
/// 2. It has at least one renderable component (`mesh` or `light`).
/// 3. Its bounding sphere overlaps the frustum.
///
/// Directional lights are always considered visible (infinite extent).
pub fn visibility_cull(
    candidates: &[EntityId],
    store:      &EntityStore,
    frustum:    &Frustum,
) -> Vec<EntityId> {
    candidates
        .iter()
        .copied()
        .filter(|&id| {
            let Some(components) = store.get(id) else { return false };
            if !components.is_renderable()           { return false };
            let Some(transform) = &components.transform else { return false };

            // Directional lights illuminate everything — never cull them.
            if let Some(LightData::Directional { .. }) = &components.light {
                return true;
            }

            // Skylight / sky atmosphere are scene-global — never cull them.
            if components.skylight.is_some() || components.sky_atmosphere.is_some() {
                return true;
            }

            // Effective bounding sphere radius:
            //  - mesh extent  → components.bounding_radius (set by caller from geometry)
            //  - light range  → light's own bounding_radius()
            //  - fallback     → 50 m if neither is provided (conservative)
            let mesh_radius  = components.bounding_radius;
            let light_radius = components.light
                .as_ref()
                .map(|l| l.bounding_radius())
                .unwrap_or(0.0);
            let radius = if mesh_radius > 0.0 || light_radius > 0.0 {
                mesh_radius.max(light_radius)
            } else {
                50.0 // generous fallback for entities with unspecified bounds
            };

            frustum.intersects_sphere(transform.position, radius)
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec3, Mat4};
    use std::f32::consts::FRAC_PI_4;
    use crate::entity::{Components, EntityStore, LightData, MeshHandle, Transform};

    /// Build a standard perspective view-proj pointing down −Z from the origin.
    fn standard_vp() -> Mat4 {
        let view = Mat4::look_at_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let proj = Mat4::perspective_rh(FRAC_PI_4, 1.0, 0.1, 100.0);
        proj * view
    }

    // ── Frustum plane extraction ──────────────────────────────────────────────

    #[test]
    fn frustum_from_identity_has_six_planes() {
        // Smoke test: no panic on identity matrix
        let _ = Frustum::from_view_proj(&Mat4::IDENTITY);
    }

    #[test]
    fn frustum_center_point_is_inside() {
        let vp      = standard_vp();
        let frustum = Frustum::from_view_proj(&vp);
        // A point directly in front of the camera, well within the frustum
        assert!(frustum.contains_point(Vec3::new(0.0, 0.0, -5.0)));
    }

    #[test]
    fn frustum_far_behind_is_outside() {
        let vp      = standard_vp();
        let frustum = Frustum::from_view_proj(&vp);
        // A point behind the camera (+Z in camera space)
        assert!(!frustum.contains_point(Vec3::new(0.0, 0.0, 200.0)));
    }

    #[test]
    fn frustum_extreme_side_is_outside() {
        let vp      = standard_vp();
        let frustum = Frustum::from_view_proj(&vp);
        // Very far to the right, same depth — outside left/right planes
        assert!(!frustum.contains_point(Vec3::new(1000.0, 0.0, -5.0)));
    }

    #[test]
    fn frustum_sphere_intersects_on_boundary() {
        let vp      = standard_vp();
        let frustum = Frustum::from_view_proj(&vp);
        // Large sphere centered behind camera but radius overlaps frustum
        // (1000 units right but 2000-unit radius — conservative test)
        assert!(frustum.intersects_sphere(Vec3::new(1000.0, 0.0, -5.0), 2000.0));
    }

    #[test]
    fn frustum_tiny_sphere_outside_is_culled() {
        let vp      = standard_vp();
        let frustum = Frustum::from_view_proj(&vp);
        assert!(!frustum.intersects_sphere(Vec3::new(500.0, 0.0, -5.0), 0.01));
    }

    #[test]
    fn frustum_sphere_inside_not_culled() {
        let vp      = standard_vp();
        let frustum = Frustum::from_view_proj(&vp);
        assert!(frustum.intersects_sphere(Vec3::new(0.0, 0.0, -10.0), 1.0));
    }

    // ── visibility_cull ───────────────────────────────────────────────────────

    fn make_store_with_entity(pos: Vec3, renderable: bool) -> (EntityStore, EntityId) {
        let mut store = EntityStore::new();
        let mut c     = Components::new().with_transform(Transform::from_position(pos));
        if renderable {
            c = c.with_mesh(MeshHandle(1));
        }
        let id = store.spawn(c);
        (store, id)
    }

    #[test]
    fn cull_entity_inside_frustum_is_visible() {
        let vp               = standard_vp();
        let frustum          = Frustum::from_view_proj(&vp);
        let (store, id)      = make_store_with_entity(Vec3::new(0.0, 0.0, -5.0), true);
        let visible          = visibility_cull(&[id], &store, &frustum);
        assert!(visible.contains(&id));
    }

    #[test]
    fn cull_entity_outside_frustum_is_hidden() {
        let vp               = standard_vp();
        let frustum          = Frustum::from_view_proj(&vp);
        let (store, id)      = make_store_with_entity(Vec3::new(500.0, 0.0, -5.0), true);
        let visible          = visibility_cull(&[id], &store, &frustum);
        assert!(!visible.contains(&id));
    }

    #[test]
    fn cull_non_renderable_entity_always_hidden() {
        let vp               = standard_vp();
        let frustum          = Frustum::from_view_proj(&vp);
        let (store, id)      = make_store_with_entity(Vec3::new(0.0, 0.0, -5.0), false);
        let visible          = visibility_cull(&[id], &store, &frustum);
        assert!(!visible.contains(&id));
    }

    #[test]
    fn cull_directional_light_always_visible() {
        let vp      = standard_vp();
        let frustum = Frustum::from_view_proj(&vp);
        // Directional light with no transform — should still be skipped (no transform)
        let mut store = EntityStore::new();
        let id = store.spawn(
            Components::new()
                .with_transform(Transform::from_position(Vec3::new(9999.0, 9999.0, 9999.0)))
                .with_light(LightData::Directional {
                    direction: [0.0, -1.0, 0.0],
                    color:     [1.0, 1.0, 1.0],
                    intensity: 1.0,
                })
        );
        let visible = visibility_cull(&[id], &store, &frustum);
        assert!(visible.contains(&id), "directional lights must never be frustum-culled");
    }

    #[test]
    fn cull_entity_without_transform_is_hidden() {
        let vp      = standard_vp();
        let frustum = Frustum::from_view_proj(&vp);
        let mut store = EntityStore::new();
        // Renderable but no transform — no position to test
        let id = store.spawn(Components::new().with_mesh(MeshHandle(1)));
        let visible = visibility_cull(&[id], &store, &frustum);
        assert!(!visible.contains(&id));
    }

    #[test]
    fn cull_empty_candidates_returns_empty() {
        let vp      = standard_vp();
        let frustum = Frustum::from_view_proj(&vp);
        let store   = EntityStore::new();
        assert!(visibility_cull(&[], &store, &frustum).is_empty());
    }
}
