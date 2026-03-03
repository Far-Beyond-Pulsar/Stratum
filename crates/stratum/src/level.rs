//! Level — the fundamental world container.
//!
//! A `Level` owns:
//! * An `EntityStore` — all entities in this level.
//! * A `WorldPartition` — spatial grid and streaming state.
//! * A `StreamingState` — high-level lifecycle of the level itself.
//!
//! Levels live inside `Stratum`. Cameras are *not* level-local; they are
//! registered at the `Stratum` level and look into whichever level is active.
//!
//! ## Additive loading
//!
//! Multiple levels can coexist in `Stratum::levels`. Future work can use this
//! for additive streaming (e.g., Unreal-style sublevel system). Today, only
//! one level is treated as "active" at a time.

use glam::Vec3;
use crate::entity::{Components, EntityId, EntityStore, Transform};
use crate::partition::WorldPartition;

// ── LevelId ───────────────────────────────────────────────────────────────────

/// Unique identifier for a `Level` within a `Stratum` instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LevelId(u64);

impl LevelId {
    #[inline] pub fn new(val: u64) -> Self { Self(val) }
    #[inline] pub fn raw(self)     -> u64  { self.0 }
}

impl std::fmt::Display for LevelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Level({})", self.0)
    }
}

// ── StreamingState ────────────────────────────────────────────────────────────

/// High-level streaming lifecycle of a `Level`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamingState {
    /// Not loaded — no entities or partition data in memory.
    Unloaded,
    /// Streaming from storage; partially resident.
    Loading,
    /// Fully resident and active.
    Active,
    /// Eviction in progress; still resident until complete.
    Unloading,
}

// ── Level ─────────────────────────────────────────────────────────────────────

/// One world container — a map, zone, or gameplay area.
pub struct Level {
    id:              LevelId,
    pub name:        String,
    entities:        EntityStore,
    partition:       WorldPartition,
    streaming_state: StreamingState,
}

impl Level {
    /// Create a new empty level with the given spatial partition parameters.
    ///
    /// The level starts in `StreamingState::Active`; all entity spawning goes
    /// live immediately. For streamed levels the state should be set to
    /// `Loading` and transitioned by the streaming backend.
    pub fn new(
        id:               LevelId,
        name:             impl Into<String>,
        chunk_size:       f32,
        activation_radius: f32,
    ) -> Self {
        Self {
            id,
            name:            name.into(),
            entities:        EntityStore::new(),
            partition:       WorldPartition::new(chunk_size, activation_radius),
            streaming_state: StreamingState::Active,
        }
    }

    // ── Accessors ─────────────────────────────────────────────────────────────

    #[inline] pub fn id(&self)              -> LevelId       { self.id }
    #[inline] pub fn streaming_state(&self) -> StreamingState { self.streaming_state }
    #[inline] pub fn entities(&self)        -> &EntityStore   { &self.entities }
    #[inline] pub fn entities_mut(&mut self) -> &mut EntityStore { &mut self.entities }
    #[inline] pub fn partition(&self)        -> &WorldPartition  { &self.partition }
    #[inline] pub fn partition_mut(&mut self) -> &mut WorldPartition { &mut self.partition }

    // ── Entity management ─────────────────────────────────────────────────────

    /// Spawn an entity and register its position in the world partition.
    ///
    /// Uses `Transform::position` as the placement point; entities without a
    /// transform are placed at the origin chunk.
    pub fn spawn_entity(&mut self, components: Components) -> EntityId {
        let world_pos = components.transform
            .as_ref()
            .map(|t: &Transform| t.position)
            .unwrap_or(Vec3::ZERO);
        let id = self.entities.spawn(components);
        self.partition.place_entity(id, world_pos);
        id
    }

    /// Despawn an entity and remove it from the world partition.
    pub fn despawn_entity(&mut self, id: EntityId) -> Option<Components> {
        let components = self.entities.despawn(id)?;
        let world_pos = components.transform
            .as_ref()
            .map(|t: &Transform| t.position)
            .unwrap_or(Vec3::ZERO);
        self.partition.remove_entity(id, world_pos);
        Some(components)
    }

    // ── Partition helpers ─────────────────────────────────────────────────────

    /// Recompute chunk activation based on the provided world positions.
    ///
    /// Typically called by `Stratum::tick()` with active camera positions.
    pub fn activate_partition_around(&mut self, positions: &[Vec3]) {
        self.partition.update_activation(positions);
    }

    /// Force-activate every existing chunk — useful for small / demo levels
    /// that don't need streaming.
    pub fn activate_all_chunks(&mut self) {
        self.partition.activate_all();
    }

    // ── Streaming state ───────────────────────────────────────────────────────

    pub fn set_streaming_state(&mut self, state: StreamingState) {
        self.streaming_state = state;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use crate::entity::{Components, MeshHandle, Transform};
    use crate::chunk::ChunkState;

    fn make_level() -> Level {
        Level::new(LevelId::new(1), "test", 16.0, 32.0)
    }

    // ── Spawn / despawn ───────────────────────────────────────────────────────

    #[test]
    fn spawn_entity_is_retrievable() {
        let mut level = make_level();
        let id = level.spawn_entity(Components::new().with_mesh(MeshHandle(1)));
        assert!(level.entities().get(id).is_some());
    }

    #[test]
    fn spawn_places_entity_in_partition() {
        let mut level = make_level();
        let pos       = Vec3::new(5.0, 0.0, 5.0);
        let id        = level.spawn_entity(
            Components::new()
                .with_transform(Transform::from_position(pos))
                .with_mesh(MeshHandle(1)),
        );
        // The entity should appear in at least one chunk of the partition
        let coord = level.partition().coord_for(pos);
        let found = level.partition()
            .chunks()
            .find(|c| c.coord == coord)
            .map(|c| c.entities.contains(&id))
            .unwrap_or(false);
        assert!(found);
    }

    #[test]
    fn despawn_removes_from_entity_store() {
        let mut level = make_level();
        let id        = level.spawn_entity(Components::new().with_mesh(MeshHandle(2)));
        assert!(level.despawn_entity(id).is_some());
        assert!(level.entities().get(id).is_none());
    }

    #[test]
    fn despawn_removes_from_partition() {
        let mut level = make_level();
        let pos       = Vec3::new(8.0, 0.0, 0.0);
        let id        = level.spawn_entity(
            Components::new().with_transform(Transform::from_position(pos)),
        );
        level.despawn_entity(id);
        let coord = level.partition().coord_for(pos);
        let found = level.partition()
            .chunks()
            .find(|c| c.coord == coord)
            .map(|c| c.entities.contains(&id))
            .unwrap_or(false);
        assert!(!found);
    }

    #[test]
    fn despawn_nonexistent_returns_none() {
        let mut level = make_level();
        assert!(level.despawn_entity(EntityId::new(99999)).is_none());
    }

    // ── Partition activation ──────────────────────────────────────────────────

    #[test]
    fn activate_all_chunks_sets_active() {
        let mut level = make_level();
        level.spawn_entity(Components::new()
            .with_transform(Transform::from_position(Vec3::ZERO)));
        level.activate_all_chunks();
        assert!(level.partition().chunks().all(|c| c.state == ChunkState::Active));
    }

    #[test]
    fn activate_partition_around_activates_nearby() {
        let mut level = make_level();
        level.spawn_entity(Components::new()
            .with_transform(Transform::from_position(Vec3::ZERO)));
        level.activate_partition_around(&[Vec3::ZERO]);
        let coord     = level.partition().coord_for(Vec3::ZERO);
        let state     = level.partition()
            .chunks()
            .find(|c| c.coord == coord)
            .map(|c| c.state);
        assert_eq!(state, Some(ChunkState::Active));
    }

    // ── Streaming state ───────────────────────────────────────────────────────

    #[test]
    fn new_level_is_active_by_default() {
        let level = make_level();
        assert_eq!(level.streaming_state(), StreamingState::Active);
    }

    #[test]
    fn set_streaming_state_persists() {
        let mut level = make_level();
        level.set_streaming_state(StreamingState::Unloaded);
        assert_eq!(level.streaming_state(), StreamingState::Unloaded);
    }

    // ── Identity ─────────────────────────────────────────────────────────────

    #[test]
    fn level_id_is_stable() {
        let level = make_level();
        assert_eq!(level.id(), LevelId::new(1));
    }
}
