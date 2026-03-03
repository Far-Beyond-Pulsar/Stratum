//! Entity data model.
//!
//! Stratum uses a flat, explicit component model rather than a trait-based ECS.
//! Each entity is a bag of optional typed components stored in a per-level
//! `EntityStore`.
//!
//! ## Design rationale
//!
//! Stratum is a *world orchestrator*, not a general-purpose ECS runtime.
//! Fine-grained game logic belongs in a dedicated ECS crate (e.g., hecs or
//! bevy_ecs). Stratum's entity model is intentionally minimal: it carries only
//! the data needed to produce `RenderView`s and drive spatial streaming.

use std::collections::HashMap;
use glam::{Vec3, Quat};

// ── EntityId ──────────────────────────────────────────────────────────────────

/// Opaque, stable entity identifier.
///
/// IDs are unique within a `Level`; uniqueness across levels is not guaranteed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntityId(u64);

impl EntityId {
    #[inline] pub fn new(val: u64) -> Self { Self(val) }
    #[inline] pub fn raw(self)     -> u64  { self.0 }
}

impl std::fmt::Display for EntityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Entity({})", self.0)
    }
}

// ── Component types ───────────────────────────────────────────────────────────

/// World-space transform.
#[derive(Debug, Clone)]
pub struct Transform {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale:    Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale:    Vec3::ONE,
        }
    }
}

impl Transform {
    pub fn from_position(position: Vec3) -> Self {
        Self { position, ..Default::default() }
    }
}

/// Opaque reference to a GPU mesh asset.
///
/// Stratum never touches GPU resources. The `stratum-helio` integration crate
/// maintains an `AssetRegistry` that maps `MeshHandle → GpuMesh`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MeshHandle(pub u64);

/// Light source definition attached to an entity.
#[derive(Debug, Clone)]
pub enum LightData {
    Point {
        color:     [f32; 3],
        intensity: f32,
        range:     f32,
    },
    Directional {
        direction: [f32; 3],
        color:     [f32; 3],
        intensity: f32,
    },
    Spot {
        direction:   [f32; 3],
        color:       [f32; 3],
        intensity:   f32,
        range:       f32,
        inner_angle: f32,
        outer_angle: f32,
    },
}

impl LightData {
    /// Conservative bounding radius for frustum culling.
    /// Directional lights return `f32::MAX` (always visible).
    pub fn bounding_radius(&self) -> f32 {
        match self {
            LightData::Point       { range, .. } => *range,
            LightData::Spot        { range, .. } => *range,
            LightData::Directional { .. }        => f32::MAX,
        }
    }
}

/// Camera-facing billboard sprite attached to an entity.
///
/// Renders a screen-aligned quad at the entity's world position. Useful for
/// light halos, particles, editor icons, and any effect that should always
/// face the viewer.
///
/// Stratum carries pure data here (no GPU handle). The `stratum-helio`
/// integration crate translates this to a `BillboardInstance` for Helio.
#[derive(Debug, Clone)]
pub struct BillboardData {
    /// Width and height of the quad in world-space metres.
    pub size:         [f32; 2],
    /// RGBA linear-colour tint multiplied with the billboard sprite texture.
    pub color:        [f32; 4],
    /// When `true` the size stays constant in screen space regardless of depth.
    pub screen_scale: bool,
}

impl BillboardData {
    pub fn new(width: f32, height: f32, color: [f32; 4]) -> Self {
        Self { size: [width, height], color, screen_scale: false }
    }

    pub fn with_screen_scale(mut self) -> Self {
        self.screen_scale = true;
        self
    }
}

/// All components an entity may carry.
///
/// All fields are optional. Stratum processes only the components that are
/// present — absent components cost nothing.
#[derive(Debug, Clone, Default)]
pub struct Components {
    pub transform:       Option<Transform>,
    pub mesh:            Option<MeshHandle>,
    pub light:           Option<LightData>,
    /// Camera-facing billboard rendered at the entity's world position.
    pub billboard:       Option<BillboardData>,
    /// Radius (metres) of the bounding sphere centred at `transform.position`.
    ///
    /// Used by frustum culling to avoid discarding large objects whose
    /// centre happens to be just outside the frustum planes.
    ///
    /// * Cube with half-size `h`       → `h * f32::sqrt(3.0)` (≈ h × 1.73)
    /// * Plane with half-extent `e`    → `e * f32::sqrt(2.0)` (≈ e × 1.41)
    /// * Light-only entity             → leave at `0.0`; light range is used.
    /// * Unset (`0.0`)                 → 50 m conservative fallback.
    pub bounding_radius: f32,
    /// Arbitrary string tags for runtime queries (e.g., "player", "static").
    pub tags:            Vec<String>,
}

impl Components {
    pub fn new() -> Self { Self::default() }

    pub fn with_transform     (mut self, t: Transform)     -> Self { self.transform       = Some(t); self }
    pub fn with_mesh          (mut self, h: MeshHandle)    -> Self { self.mesh             = Some(h); self }
    pub fn with_light         (mut self, l: LightData)     -> Self { self.light            = Some(l); self }
    pub fn with_billboard     (mut self, b: BillboardData) -> Self { self.billboard        = Some(b); self }
    pub fn with_bounding_radius(mut self, r: f32)          -> Self { self.bounding_radius  = r;       self }
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into()); self
    }

    /// Returns `true` if this entity contributes anything to a render view.
    #[inline]
    pub fn is_renderable(&self) -> bool {
        self.mesh.is_some() || self.light.is_some() || self.billboard.is_some()
    }
}

// ── EntityStore ───────────────────────────────────────────────────────────────

/// Flat `HashMap`-backed entity storage for one `Level`.
pub struct EntityStore {
    next_id:  u64,
    entities: HashMap<EntityId, Components>,
}

impl EntityStore {
    pub fn new() -> Self {
        Self { next_id: 1, entities: HashMap::new() }
    }

    /// Spawn a new entity. Returns its assigned `EntityId`.
    pub fn spawn(&mut self, components: Components) -> EntityId {
        let id = EntityId::new(self.next_id);
        self.next_id += 1;
        self.entities.insert(id, components);
        id
    }

    /// Despawn (remove) an entity. Returns its components if it existed.
    pub fn despawn(&mut self, id: EntityId) -> Option<Components> {
        self.entities.remove(&id)
    }

    pub fn get    (&self,     id: EntityId) -> Option<&Components>     { self.entities.get(&id) }
    pub fn get_mut(&mut self, id: EntityId) -> Option<&mut Components> { self.entities.get_mut(&id) }

    pub fn iter    (&self)     -> impl Iterator<Item = (EntityId, &Components)>     { self.entities.iter().map(|(&id, c)| (id, c)) }
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (EntityId, &mut Components)> { self.entities.iter_mut().map(|(&id, c)| (id, c)) }

    pub fn len     (&self) -> usize { self.entities.len()     }
    pub fn is_empty(&self) -> bool  { self.entities.is_empty() }
}

impl Default for EntityStore {
    fn default() -> Self { Self::new() }
}

// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec3, Quat};

    // ── EntityId ──────────────────────────────────────────────────────────────

    #[test]
    fn entity_id_round_trip() {
        let id = EntityId::new(42);
        assert_eq!(id.raw(), 42);
    }

    #[test]
    fn entity_id_display() {
        assert_eq!(format!("{}", EntityId::new(7)), "Entity(7)");
    }

    #[test]
    fn entity_id_ordering() {
        assert!(EntityId::new(1) < EntityId::new(2));
    }

    // ── Transform ─────────────────────────────────────────────────────────────

    #[test]
    fn transform_default_is_identity() {
        let t = Transform::default();
        assert_eq!(t.position, Vec3::ZERO);
        assert_eq!(t.rotation, Quat::IDENTITY);
        assert_eq!(t.scale,    Vec3::ONE);
    }

    #[test]
    fn transform_from_position() {
        let p = Vec3::new(1.0, 2.0, 3.0);
        let t = Transform::from_position(p);
        assert_eq!(t.position, p);
        assert_eq!(t.scale, Vec3::ONE);
    }

    // ── Components ────────────────────────────────────────────────────────────

    #[test]
    fn components_is_renderable_with_mesh() {
        let c = Components::new().with_mesh(MeshHandle(1));
        assert!(c.is_renderable());
    }

    #[test]
    fn components_is_renderable_with_light() {
        let c = Components::new().with_light(LightData::Point {
            color: [1.0, 1.0, 1.0], intensity: 1.0, range: 5.0,
        });
        assert!(c.is_renderable());
    }

    #[test]
    fn components_not_renderable_empty() {
        let c = Components::new();
        assert!(!c.is_renderable());
    }

    #[test]
    fn components_builder_chain() {
        let c = Components::new()
            .with_transform(Transform::from_position(Vec3::ONE))
            .with_mesh(MeshHandle(7))
            .with_tag("player");
        assert!(c.transform.is_some());
        assert!(c.mesh.is_some());
        assert_eq!(c.tags, vec!["player"]);
    }

    #[test]
    fn components_multiple_tags() {
        let c = Components::new().with_tag("a").with_tag("b").with_tag("c");
        assert_eq!(c.tags.len(), 3);
        assert!(c.tags.contains(&"b".to_string()));
    }

    // ── LightData ─────────────────────────────────────────────────────────────

    #[test]
    fn light_data_bounding_radius_point() {
        let l = LightData::Point { color: [1.0; 3], intensity: 1.0, range: 10.0 };
        assert_eq!(l.bounding_radius(), 10.0);
    }

    #[test]
    fn light_data_bounding_radius_directional_is_infinite() {
        let l = LightData::Directional { direction: [0.0, -1.0, 0.0], color: [1.0; 3], intensity: 1.0 };
        assert_eq!(l.bounding_radius(), f32::MAX);
    }

    #[test]
    fn light_data_bounding_radius_spot() {
        let l = LightData::Spot {
            direction: [0.0, -1.0, 0.0], color: [1.0; 3],
            intensity: 1.0, range: 15.0,
            inner_angle: 0.2, outer_angle: 0.4,
        };
        assert_eq!(l.bounding_radius(), 15.0);
    }

    // ── EntityStore ───────────────────────────────────────────────────────────

    #[test]
    fn entity_store_spawn_assigns_unique_ids() {
        let mut store = EntityStore::new();
        let id1 = store.spawn(Components::new());
        let id2 = store.spawn(Components::new());
        assert_ne!(id1, id2);
    }

    #[test]
    fn entity_store_spawn_ids_monotonically_increasing() {
        let mut store = EntityStore::new();
        let id1 = store.spawn(Components::new());
        let id2 = store.spawn(Components::new());
        assert!(id1 < id2);
    }

    #[test]
    fn entity_store_despawn_removes_entity() {
        let mut store = EntityStore::new();
        let id = store.spawn(Components::new().with_tag("temp"));
        assert!(store.get(id).is_some());
        let removed = store.despawn(id);
        assert!(removed.is_some());
        assert!(store.get(id).is_none());
    }

    #[test]
    fn entity_store_despawn_nonexistent_returns_none() {
        let mut store = EntityStore::new();
        assert!(store.despawn(EntityId::new(9999)).is_none());
    }

    #[test]
    fn entity_store_get_mut_allows_modification() {
        let mut store = EntityStore::new();
        let id = store.spawn(Components::new().with_mesh(MeshHandle(1)));
        store.get_mut(id).unwrap().mesh = None;
        assert!(store.get(id).unwrap().mesh.is_none());
    }

    #[test]
    fn entity_store_len_and_is_empty() {
        let mut store = EntityStore::new();
        assert!(store.is_empty());
        store.spawn(Components::new());
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn entity_store_iter_yields_all() {
        let mut store = EntityStore::new();
        let ids: Vec<EntityId> = (0..5).map(|_| store.spawn(Components::new())).collect();
        let seen: Vec<EntityId> = store.iter().map(|(id, _)| id).collect();
        for id in ids {
            assert!(seen.contains(&id));
        }
    }
}
