//! World partition system — grid-based spatial chunk management.
//!
//! `WorldPartition` is the streaming backbone of a `Level`. It maintains a
//! lazy map of `Chunk`s and activates/deactivates cells based on camera
//! proximity each frame.
//!
//! ## Streaming Hooks
//!
//! Async disk IO is **not** implemented here, but every state transition is
//! labelled `// STREAMING HOOK` so a real streaming backend can be spliced in
//! without restructuring the rest of the system.

use std::collections::{HashMap, HashSet};
use glam::Vec3;

use crate::chunk::{Chunk, ChunkCoord, ChunkState};
use crate::entity::EntityId;

/// Manages the grid of spatial chunks for one `Level`.
pub struct WorldPartition {
    /// Side length of each cubic chunk in world units.
    pub chunk_size: f32,
    /// Distance (from a camera position to a chunk center) at which a chunk
    /// is considered active.
    pub activation_radius: f32,
    chunks: HashMap<ChunkCoord, Chunk>,
}

impl WorldPartition {
    pub fn new(chunk_size: f32, activation_radius: f32) -> Self {
        Self {
            chunk_size,
            activation_radius,
            chunks: HashMap::new(),
        }
    }

    // ── Chunk access ──────────────────────────────────────────────────────────

    /// Return the chunk at `coord`, inserting a fresh unloaded chunk if absent.
    pub fn get_or_create(&mut self, coord: ChunkCoord) -> &mut Chunk {
        let chunk_size = self.chunk_size;
        self.chunks
            .entry(coord)
            .or_insert_with(|| Chunk::new(coord, chunk_size))
    }

    /// Map a world position to its containing chunk coordinate.
    #[inline]
    pub fn coord_for(&self, pos: Vec3) -> ChunkCoord {
        ChunkCoord::from_world(pos, self.chunk_size)
    }

    // ── Entity placement ──────────────────────────────────────────────────────

    /// Insert `id` into the chunk that contains `world_pos`.
    pub fn place_entity(&mut self, id: EntityId, world_pos: Vec3) {
        let coord      = ChunkCoord::from_world(world_pos, self.chunk_size);
        let chunk_size = self.chunk_size;
        self.chunks
            .entry(coord)
            .or_insert_with(|| Chunk::new(coord, chunk_size))
            .add_entity(id);
    }

    /// Remove `id` from the chunk that contains `world_pos`.
    pub fn remove_entity(&mut self, id: EntityId, world_pos: Vec3) {
        let coord = ChunkCoord::from_world(world_pos, self.chunk_size);
        if let Some(chunk) = self.chunks.get_mut(&coord) {
            chunk.remove_entity(id);
        }
    }

    // ── Activation ────────────────────────────────────────────────────────────

    /// Recompute the active set based on current camera positions.
    ///
    /// Chunks entering the activation radius transition `Unloaded → Active`.
    /// Chunks leaving it transition `Active → Unloaded`.
    /// All state-transition sites are marked `// STREAMING HOOK`.
    pub fn update_activation(&mut self, camera_positions: &[Vec3]) {
        let radius     = self.activation_radius;
        let chunk_size = self.chunk_size;

        // ── 1. Build the desired-active set ──────────────────────────────────
        let mut desired: HashSet<ChunkCoord> = HashSet::new();
        for &cam in camera_positions {
            let half_span = (radius / chunk_size).ceil() as i32 + 1;
            let center    = ChunkCoord::from_world(cam, chunk_size);
            for dx in -half_span..=half_span {
                for dy in -half_span..=half_span {
                    for dz in -half_span..=half_span {
                        let coord = ChunkCoord::new(
                            center.x + dx,
                            center.y + dy,
                            center.z + dz,
                        );
                        let chunk_center = Vec3::new(
                            (coord.x as f32 + 0.5) * chunk_size,
                            (coord.y as f32 + 0.5) * chunk_size,
                            (coord.z as f32 + 0.5) * chunk_size,
                        );
                        if cam.distance(chunk_center) <= radius {
                            desired.insert(coord);
                        }
                    }
                }
            }
        }

        // ── 2. Activate newly in-range chunks ────────────────────────────────
        for &coord in &desired {
            let chunk_size_local = chunk_size;
            let chunk = self.chunks
                .entry(coord)
                .or_insert_with(|| Chunk::new(coord, chunk_size_local));
            if chunk.state == ChunkState::Unloaded {
                // STREAMING HOOK: initiate async asset load here.
                chunk.state = ChunkState::Active;
                log::debug!("Chunk {:?} activated", coord);
            }
        }

        // ── 3. Deactivate out-of-range chunks ────────────────────────────────
        for chunk in self.chunks.values_mut() {
            if chunk.state == ChunkState::Active && !desired.contains(&chunk.coord) {
                // STREAMING HOOK: initiate async asset eviction here.
                chunk.state = ChunkState::Unloaded;
                log::debug!("Chunk {:?} deactivated", chunk.coord);
            }
        }
    }

    /// Force-activate all existing chunks (useful for small / demo levels).
    pub fn activate_all(&mut self) {
        for chunk in self.chunks.values_mut() {
            chunk.activate();
        }
    }

    // ── Queries ───────────────────────────────────────────────────────────────

    /// Collect every entity ID from currently active chunks.
    pub fn active_entities(&self) -> Vec<EntityId> {
        self.chunks
            .values()
            .filter(|c| c.is_active())
            .flat_map(|c| c.entities.iter().copied())
            .collect()
    }

    /// Collect every entity ID from **all** chunks regardless of streaming state.
    ///
    /// Used to ensure lights (and other entities with influence that extends
    /// beyond their containing chunk) are never silently dropped from a
    /// render view just because their chunk is outside the activation radius.
    /// Frustum culling handles the final visibility decision.
    pub fn all_entities(&self) -> Vec<EntityId> {
        self.chunks
            .values()
            .flat_map(|c| c.entities.iter().copied())
            .collect()
    }

    pub fn chunks(&self) -> impl Iterator<Item = &Chunk> {
        self.chunks.values()
    }

    pub fn chunks_mut(&mut self) -> impl Iterator<Item = &mut Chunk> {
        self.chunks.values_mut()
    }

    pub fn active_chunks(&self) -> impl Iterator<Item = &Chunk> {
        self.chunks.values().filter(|c| c.is_active())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use crate::chunk::ChunkState;
    use crate::entity::EntityId;

    fn make_partition() -> WorldPartition {
        WorldPartition::new(16.0, 32.0)
    }

    // ── get_or_create ─────────────────────────────────────────────────────────

    #[test]
    fn get_or_create_inserts_new_chunk() {
        let mut wp = make_partition();
        let coord  = ChunkCoord::new(1, 0, 0);
        let chunk  = wp.get_or_create(coord);
        assert_eq!(chunk.coord, coord);
        assert_eq!(chunk.state, ChunkState::Unloaded);
    }

    #[test]
    fn get_or_create_returns_existing() {
        let mut wp    = make_partition();
        let coord     = ChunkCoord::new(0, 0, 0);
        wp.get_or_create(coord).activate();
        // Second call should see the activated chunk
        assert_eq!(wp.get_or_create(coord).state, ChunkState::Active);
    }

    // ── coord_for ─────────────────────────────────────────────────────────────

    #[test]
    fn coord_for_matches_manual() {
        let wp = make_partition();
        let c  = wp.coord_for(Vec3::new(17.0, 0.0, 0.0));
        assert_eq!(c, ChunkCoord::new(1, 0, 0));
    }

    // ── place_entity / remove_entity ─────────────────────────────────────────

    #[test]
    fn place_entity_creates_chunk() {
        let mut wp = make_partition();
        wp.place_entity(EntityId::new(1), Vec3::new(5.0, 0.0, 5.0));
        let coord = wp.coord_for(Vec3::new(5.0, 0.0, 5.0));
        assert!(wp.get_or_create(coord).entities.contains(&EntityId::new(1)));
    }

    #[test]
    fn remove_entity_cleans_chunk() {
        let mut wp  = make_partition();
        let id      = EntityId::new(99);
        let pos     = Vec3::new(5.0, 0.0, 5.0);
        wp.place_entity(id, pos);
        wp.remove_entity(id, pos);
        let coord   = wp.coord_for(pos);
        assert!(!wp.get_or_create(coord).entities.contains(&id));
    }

    // ── update_activation ─────────────────────────────────────────────────────

    #[test]
    fn activation_brings_nearby_chunks_active() {
        let mut wp = WorldPartition::new(16.0, 20.0);
        // Seed a chunk at origin so it exists before activation
        wp.get_or_create(ChunkCoord::new(0, 0, 0));
        wp.update_activation(&[Vec3::ZERO]);
        // The chunk containing the origin should now be active
        assert_eq!(
            wp.get_or_create(ChunkCoord::new(0, 0, 0)).state,
            ChunkState::Active
        );
    }

    #[test]
    fn activation_deactivates_far_chunks() {
        let mut wp = WorldPartition::new(16.0, 20.0);
        // Manually insert a chunk far away and force it active
        wp.get_or_create(ChunkCoord::new(100, 0, 100)).activate();
        // Activate around origin — far chunk should be evicted
        wp.update_activation(&[Vec3::ZERO]);
        assert_eq!(
            wp.chunks()
                .find(|c| c.coord == ChunkCoord::new(100, 0, 100))
                .unwrap()
                .state,
            ChunkState::Unloaded
        );
    }

    #[test]
    fn activate_all_sets_all_active() {
        let mut wp = make_partition();
        for i in 0..4 {
            wp.get_or_create(ChunkCoord::new(i, 0, 0));
        }
        wp.activate_all();
        assert!(wp.chunks().all(|c| c.state == ChunkState::Active));
    }

    // ── active_entities ───────────────────────────────────────────────────────

    #[test]
    fn active_entities_only_from_active_chunks() {
        let mut wp = make_partition();
        // Place two entities in two chunks
        let id_active   = EntityId::new(1);
        let id_inactive = EntityId::new(2);
        wp.place_entity(id_active,   Vec3::new(0.0, 0.0,  0.0));
        wp.place_entity(id_inactive, Vec3::new(500.0, 0.0, 0.0));
        // Activate only the first chunk
        wp.get_or_create(ChunkCoord::new(0, 0, 0)).activate();
        let active = wp.active_entities();
        assert!(active.contains(&id_active));
        assert!(!active.contains(&id_inactive));
    }

    #[test]
    fn all_entities_includes_inactive_chunks() {
        let mut wp = make_partition();
        let id_active   = EntityId::new(1);
        let id_inactive = EntityId::new(2);
        wp.place_entity(id_active,   Vec3::new(0.0, 0.0, 0.0));
        wp.place_entity(id_inactive, Vec3::new(500.0, 0.0, 0.0));
        // Only activate the first chunk
        wp.get_or_create(ChunkCoord::new(0, 0, 0)).activate();
        // active_entities skips the unloaded chunk
        assert!(!wp.active_entities().contains(&id_inactive));
        // all_entities includes both
        let all = wp.all_entities();
        assert!(all.contains(&id_active));
        assert!(all.contains(&id_inactive));
    }

    #[test]
    fn active_entities_empty_when_no_active_chunks() {
        let mut wp = make_partition();
        wp.place_entity(EntityId::new(5), Vec3::ZERO);
        // Do not activate
        assert!(wp.active_entities().is_empty());
    }
}
