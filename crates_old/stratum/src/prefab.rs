//! Prefab — reusable stamped structures for the voxel world.
//!
//! A `Prefab` is a named template of entities whose transforms are expressed
//! relative to the prefab's local origin.  When *placed* into a `Level` via
//! [`PlacementContext::place`], every entity is re-emitted with its transform
//! offset by the instance's world-space position/rotation/scale.
//!
//! ## Hollow vs solid volumes
//!
//! Each prefab optionally declares a set of [`PrefabVolume`]s that describe the
//! space it occupies:
//!
//! * **Solid** (`hollow: false`) — the volume must not overlap any other solid
//!   volume already claimed by the active [`PlacementContext`].  Used for
//!   walls, floors, tree trunks — anything that cannot physically coexist.
//! * **Hollow** (`hollow: true`) — may freely overlap any other volume.  Used
//!   for foliage canopies, interior air, decorative auras, etc.
//!
//! ## Typical workflow
//!
//! ```text
//! let tree = Prefab::builder("tree_oak")
//!     .with_entity(trunk_components)          // relative pos (0,0,0)..(0,4,0)
//!     .with_entity(leaf_components)
//!     .with_volume(PrefabVolume::solid(Aabb::new(…)))   // trunk keep-out
//!     .with_volume(PrefabVolume::hollow(Aabb::new(…)))  // canopy
//!     .build();
//!
//! let mut ctx = PlacementContext::new();
//! ctx.place(&tree, Vec3::new(10.0, 5.0, 10.0), &mut level)?;
//! ctx.place(&tree, Vec3::new(14.0, 5.0, 14.0), &mut level)?; // checked against first
//! ```

use glam::{Mat4, Quat, Vec3};

use crate::chunk::Aabb;
use crate::entity::{Components, EntityId, Transform};
use crate::level::Level;

// ── PrefabId ──────────────────────────────────────────────────────────────────

/// Opaque numeric identifier for a `Prefab` within a `PrefabRegistry`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PrefabId(pub u64);

impl PrefabId {
    #[inline] pub fn new(val: u64) -> Self { Self(val) }
    #[inline] pub fn raw(self)     -> u64  { self.0 }
}

// ── PrefabVolume ──────────────────────────────────────────────────────────────

/// A labeled region of space within a prefab's local coordinate system.
#[derive(Debug, Clone)]
pub struct PrefabVolume {
    /// Local-space axis-aligned bounding box.
    pub bounds: Aabb,
    /// When `true` this volume is *hollow* and may overlap any other volume.
    /// When `false` the volume is *solid* and a [`PlacementContext`] will
    /// reject any placement that would cause it to intersect another solid.
    pub hollow: bool,
}

impl PrefabVolume {
    pub fn solid(bounds: Aabb)  -> Self { Self { bounds, hollow: false } }
    pub fn hollow(bounds: Aabb) -> Self { Self { bounds, hollow: true  } }
}

// ── Prefab ────────────────────────────────────────────────────────────────────

/// A named, reusable entity template.
///
/// All entity transforms inside a `Prefab` are **local** (relative to the
/// prefab's own origin).  Use [`PrefabBuilder`] to construct one.
#[derive(Debug, Clone)]
pub struct Prefab {
    pub id:       PrefabId,
    pub name:     String,
    /// Entity component sets with local-space transforms.
    pub entities: Vec<Components>,
    /// Optional spatial volumes describing occupied space.
    pub volumes:  Vec<PrefabVolume>,
}

impl Prefab {
    /// Start building a prefab with the given human-readable name.
    pub fn builder(name: impl Into<String>) -> PrefabBuilder {
        PrefabBuilder {
            name:     name.into(),
            entities: Vec::new(),
            volumes:  Vec::new(),
        }
    }
}

// ── PrefabBuilder ─────────────────────────────────────────────────────────────

pub struct PrefabBuilder {
    name:     String,
    entities: Vec<Components>,
    volumes:  Vec<PrefabVolume>,
}

impl PrefabBuilder {
    pub fn with_entity(mut self, c: Components) -> Self {
        self.entities.push(c);
        self
    }

    pub fn with_volume(mut self, v: PrefabVolume) -> Self {
        self.volumes.push(v);
        self
    }

    /// Finalise.  The `PrefabId` is set to `0` (the registry re-assigns it
    /// when you call [`PrefabRegistry::register`]).
    pub fn build(self) -> Prefab {
        Prefab {
            id:       PrefabId(0),
            name:     self.name,
            entities: self.entities,
            volumes:  self.volumes,
        }
    }
}

// ── PrefabRegistry ────────────────────────────────────────────────────────────

/// Stores all known `Prefab`s for a session keyed by numeric id and name.
#[derive(Default)]
pub struct PrefabRegistry {
    next_id: u64,
    by_id:   std::collections::HashMap<PrefabId, Prefab>,
    name_to_id: std::collections::HashMap<String, PrefabId>,
}

impl PrefabRegistry {
    pub fn new() -> Self { Self::default() }

    /// Register a prefab, assigning it a stable `PrefabId`.  Returns the id.
    pub fn register(&mut self, mut prefab: Prefab) -> PrefabId {
        self.next_id += 1;
        let id = PrefabId(self.next_id);
        prefab.id = id;
        self.name_to_id.insert(prefab.name.clone(), id);
        self.by_id.insert(id, prefab);
        id
    }

    pub fn get(&self, id: PrefabId) -> Option<&Prefab> { self.by_id.get(&id) }

    pub fn get_by_name(&self, name: &str) -> Option<&Prefab> {
        self.name_to_id.get(name).and_then(|id| self.by_id.get(id))
    }
}

// ── PlacementError ────────────────────────────────────────────────────────────

/// Returned by [`PlacementContext::place`] when a solid volume would overlap.
#[derive(Debug)]
pub struct OverlapError {
    pub prefab_name: String,
    pub world_pos:   Vec3,
}

impl std::fmt::Display for OverlapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "prefab '{}' at {:?}: solid volume overlaps existing placement",
               self.prefab_name, self.world_pos)
    }
}

// ── PlacementContext ──────────────────────────────────────────────────────────

/// Tracks already-occupied solid world-space AABBs within a placement session
/// (typically one chunk-generation pass) and validates new placements.
///
/// A placement session is cheap to create — just a `Vec` of AABBs.
pub struct PlacementContext {
    solid_volumes: Vec<Aabb>,
}

impl PlacementContext {
    pub fn new() -> Self {
        Self { solid_volumes: Vec::new() }
    }

    /// Returns `true` if the world-space AABB `a` overlaps any already-claimed
    /// solid AABB.
    pub fn overlaps_solid(&self, a: &Aabb) -> bool {
        self.solid_volumes.iter().any(|b| aabbs_overlap(a, b))
    }

    /// Attempt to place `prefab` at `world_pos` (with identity rotation/scale).
    ///
    /// * All solid volumes are checked for overlap; returns `Err` on conflict.
    /// * On success, claims the solid volumes and spawns all entities into `level`.
    /// * Hollow volumes are ignored during overlap testing.
    ///
    /// Returns the list of spawned `EntityId`s.
    pub fn place(
        &mut self,
        prefab:    &Prefab,
        world_pos: Vec3,
        level:     &mut Level,
    ) -> Result<Vec<EntityId>, OverlapError> {
        self.place_with_rotation(prefab, world_pos, Quat::IDENTITY, level)
    }

    /// Like [`place`] but with an arbitrary rotation applied around `world_pos`.
    pub fn place_with_rotation(
        &mut self,
        prefab:    &Prefab,
        world_pos: Vec3,
        rotation:  Quat,
        level:     &mut Level,
    ) -> Result<Vec<EntityId>, OverlapError> {
        let xform = Mat4::from_rotation_translation(rotation, world_pos);

        // Compute world-space solid volumes and check for overlaps.
        let world_solid: Vec<Aabb> = prefab.volumes.iter()
            .filter(|v| !v.hollow)
            .map(|v| transform_aabb(&v.bounds, xform))
            .collect();

        for ws in &world_solid {
            if self.overlaps_solid(ws) {
                return Err(OverlapError {
                    prefab_name: prefab.name.clone(),
                    world_pos,
                });
            }
        }

        // Claim solid volumes.
        self.solid_volumes.extend(world_solid);

        // Spawn entities with world-space transforms.
        let mut ids = Vec::with_capacity(prefab.entities.len());
        for template in &prefab.entities {
            let mut c = template.clone();
            if let Some(ref local_t) = template.transform {
                let local_mat = Mat4::from_scale_rotation_translation(
                    local_t.scale,
                    local_t.rotation,
                    local_t.position,
                );
                let world_mat = xform * local_mat;
                let (scale, rot, pos) = world_mat.to_scale_rotation_translation();
                c.transform = Some(Transform { position: pos, rotation: rot, scale });
            } else {
                c.transform = Some(Transform::from_position(world_pos));
            }
            ids.push(level.spawn_entity(c));
        }
        Ok(ids)
    }
}

impl Default for PlacementContext {
    fn default() -> Self { Self::new() }
}

// ── AABB helpers ──────────────────────────────────────────────────────────────

fn aabbs_overlap(a: &Aabb, b: &Aabb) -> bool {
    a.min.x < b.max.x && a.max.x > b.min.x &&
    a.min.y < b.max.y && a.max.y > b.min.y &&
    a.min.z < b.max.z && a.max.z > b.min.z
}

/// Transform a local-space AABB through a 4×4 matrix and return the new
/// world-space axis-aligned bounding box.
///
/// This is the standard "transform all 8 corners and re-fit" algorithm that
/// works correctly even when the matrix includes rotation.
fn transform_aabb(aabb: &Aabb, m: Mat4) -> Aabb {
    let corners = [
        Vec3::new(aabb.min.x, aabb.min.y, aabb.min.z),
        Vec3::new(aabb.max.x, aabb.min.y, aabb.min.z),
        Vec3::new(aabb.min.x, aabb.max.y, aabb.min.z),
        Vec3::new(aabb.max.x, aabb.max.y, aabb.min.z),
        Vec3::new(aabb.min.x, aabb.min.y, aabb.max.z),
        Vec3::new(aabb.max.x, aabb.min.y, aabb.max.z),
        Vec3::new(aabb.min.x, aabb.max.y, aabb.max.z),
        Vec3::new(aabb.max.x, aabb.max.y, aabb.max.z),
    ];
    let mut mn = Vec3::splat(f32::MAX);
    let mut mx = Vec3::splat(f32::MIN);
    for c in corners {
        let w = m.transform_point3(c);
        mn = mn.min(w);
        mx = mx.max(w);
    }
    Aabb::new(mn, mx)
}

// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use crate::entity::{Components, MeshHandle, Transform};
    use crate::level::{Level, LevelId};

    fn make_level() -> Level {
        Level::new(LevelId::new(1), "test", 16.0, 64.0)
    }

    fn cube_aabb(cx: f32, cy: f32, cz: f32, half: f32) -> Aabb {
        Aabb::new(
            Vec3::new(cx - half, cy - half, cz - half),
            Vec3::new(cx + half, cy + half, cz + half),
        )
    }

    fn simple_prefab(name: &str, solid: bool) -> Prefab {
        Prefab::builder(name)
            .with_entity(
                Components::new()
                    .with_transform(Transform::from_position(Vec3::ZERO))
                    .with_mesh(MeshHandle(1)),
            )
            .with_volume(if solid {
                PrefabVolume::solid(cube_aabb(0.0, 0.0, 0.0, 1.0))
            } else {
                PrefabVolume::hollow(cube_aabb(0.0, 0.0, 0.0, 1.0))
            })
            .build()
    }

    // ── Registry ──────────────────────────────────────────────────────────────

    #[test]
    fn register_and_lookup_by_name() {
        let mut reg = PrefabRegistry::new();
        let id = reg.register(simple_prefab("tree", true));
        assert!(reg.get(id).is_some());
        assert!(reg.get_by_name("tree").is_some());
    }

    #[test]
    fn registry_assigns_unique_ids() {
        let mut reg = PrefabRegistry::new();
        let a = reg.register(simple_prefab("a", true));
        let b = reg.register(simple_prefab("b", true));
        assert_ne!(a, b);
    }

    // ── PlacementContext — solid ───────────────────────────────────────────────

    #[test]
    fn place_solid_succeeds_when_empty() {
        let mut ctx   = PlacementContext::new();
        let mut level = make_level();
        let p = simple_prefab("p", true);
        assert!(ctx.place(&p, Vec3::ZERO, &mut level).is_ok());
    }

    #[test]
    fn place_solid_fails_on_overlap() {
        let mut ctx   = PlacementContext::new();
        let mut level = make_level();
        let p = simple_prefab("p", true);
        ctx.place(&p, Vec3::ZERO, &mut level).unwrap();
        // Same position → overlap
        assert!(ctx.place(&p, Vec3::ZERO, &mut level).is_err());
    }

    #[test]
    fn place_solid_succeeds_when_far_apart() {
        let mut ctx   = PlacementContext::new();
        let mut level = make_level();
        let p = simple_prefab("p", true);
        ctx.place(&p, Vec3::ZERO, &mut level).unwrap();
        // 10 m away — no overlap
        assert!(ctx.place(&p, Vec3::new(10.0, 0.0, 0.0), &mut level).is_ok());
    }

    // ── PlacementContext — hollow ──────────────────────────────────────────────

    #[test]
    fn place_hollow_never_blocked() {
        let mut ctx   = PlacementContext::new();
        let mut level = make_level();
        let p = simple_prefab("p", false);
        ctx.place(&p, Vec3::ZERO, &mut level).unwrap();
        // Hollow volumes don't claim space; placing again is fine.
        assert!(ctx.place(&p, Vec3::ZERO, &mut level).is_ok());
    }

    // ── Entity spawning ────────────────────────────────────────────────────────

    #[test]
    fn place_spawns_entities_in_level() {
        let mut ctx   = PlacementContext::new();
        let mut level = make_level();
        let p = simple_prefab("p", true);
        let ids = ctx.place(&p, Vec3::new(5.0, 0.0, 5.0), &mut level).unwrap();
        assert_eq!(ids.len(), 1);
        assert!(level.entities().get(ids[0]).is_some());
    }

    #[test]
    fn place_applies_world_offset_to_transform() {
        let mut ctx   = PlacementContext::new();
        let mut level = make_level();
        let p = simple_prefab("p", true);
        let origin = Vec3::new(7.0, 0.0, 3.0);
        let ids = ctx.place(&p, origin, &mut level).unwrap();
        let pos = level.entities().get(ids[0]).unwrap()
            .transform.as_ref().unwrap().position;
        assert!((pos - origin).length() < 0.01, "pos={pos:?} expected≈{origin:?}");
    }
}
