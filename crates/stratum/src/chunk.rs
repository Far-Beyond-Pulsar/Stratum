//! Spatial chunk primitives — atoms of world partitioning.
//!
//! A chunk is a uniformly-sized cubic cell in the world partition grid.
//! It tracks which entities reside within it and its current streaming state.

use glam::Vec3;
use crate::entity::EntityId;

// ── ChunkCoord ────────────────────────────────────────────────────────────────

/// Integer 3-D grid coordinate identifying one chunk cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkCoord {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl ChunkCoord {
    #[inline]
    pub fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// Map a world position to its containing chunk given a uniform `chunk_size`.
    #[inline]
    pub fn from_world(pos: Vec3, chunk_size: f32) -> Self {
        Self {
            x: (pos.x / chunk_size).floor() as i32,
            y: (pos.y / chunk_size).floor() as i32,
            z: (pos.z / chunk_size).floor() as i32,
        }
    }
}

// ── Aabb ─────────────────────────────────────────────────────────────────────

/// Axis-aligned bounding box in world space.
#[derive(Debug, Clone, Copy)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    #[inline]
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    #[inline]
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    #[inline]
    pub fn half_extents(&self) -> Vec3 {
        (self.max - self.min) * 0.5
    }

    #[inline]
    pub fn contains_point(&self, p: Vec3) -> bool {
        p.cmpge(self.min).all() && p.cmple(self.max).all()
    }
}

// ── ChunkState ────────────────────────────────────────────────────────────────

/// Streaming lifecycle state of a partition chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkState {
    /// Not resident — no entity data in memory.
    Unloaded,
    /// Async streaming IO is in progress.
    Loading,
    /// Fully resident and renderable.
    Active,
    /// Eviction pending; still resident until the operation completes.
    Unloading,
}

// ── Chunk ─────────────────────────────────────────────────────────────────────

/// One spatial cell within a world partition grid.
pub struct Chunk {
    pub coord:    ChunkCoord,
    pub bounds:   Aabb,
    pub entities: Vec<EntityId>,
    pub state:    ChunkState,
}

impl Chunk {
    /// Create a new empty, unloaded chunk at `coord` with uniform `chunk_size`.
    pub fn new(coord: ChunkCoord, chunk_size: f32) -> Self {
        let min = Vec3::new(
            coord.x as f32 * chunk_size,
            coord.y as f32 * chunk_size,
            coord.z as f32 * chunk_size,
        );
        Self {
            coord,
            bounds:   Aabb::new(min, min + Vec3::splat(chunk_size)),
            entities: Vec::new(),
            state:    ChunkState::Unloaded,
        }
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        self.state == ChunkState::Active
    }

    /// Immediately transition to `Active` (no async IO required).
    #[inline]
    pub fn activate(&mut self) {
        self.state = ChunkState::Active;
    }

    /// Immediately transition to `Unloaded`.
    #[inline]
    pub fn deactivate(&mut self) {
        self.state = ChunkState::Unloaded;
    }

    /// Register `id` in this chunk (idempotent).
    pub fn add_entity(&mut self, id: EntityId) {
        if !self.entities.contains(&id) {
            self.entities.push(id);
        }
    }

    pub fn remove_entity(&mut self, id: EntityId) {
        self.entities.retain(|&e| e != id);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use crate::entity::EntityId;

    // ── ChunkCoord ────────────────────────────────────────────────────────────

    #[test]
    fn coord_from_world_origin() {
        let c = ChunkCoord::from_world(Vec3::ZERO, 16.0);
        assert_eq!(c, ChunkCoord::new(0, 0, 0));
    }

    #[test]
    fn coord_from_world_positive() {
        // (17, 0, 33) with chunk_size=16 → (1, 0, 2)
        let c = ChunkCoord::from_world(Vec3::new(17.0, 0.0, 33.0), 16.0);
        assert_eq!(c, ChunkCoord::new(1, 0, 2));
    }

    #[test]
    fn coord_from_world_negative() {
        // (-1, 0, 0) with chunk_size=16 → (-1, 0, 0)
        let c = ChunkCoord::from_world(Vec3::new(-1.0, 0.0, 0.0), 16.0);
        assert_eq!(c, ChunkCoord::new(-1, 0, 0));
    }

    #[test]
    fn coord_from_world_exact_boundary() {
        // Exactly on the boundary (16.0, 0, 0) → chunk (1, 0, 0)
        let c = ChunkCoord::from_world(Vec3::new(16.0, 0.0, 0.0), 16.0);
        assert_eq!(c, ChunkCoord::new(1, 0, 0));
    }

    #[test]
    fn coord_hash_equality() {
        let a = ChunkCoord::new(3, -1, 7);
        let b = ChunkCoord::new(3, -1, 7);
        let c = ChunkCoord::new(3, -1, 8);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    // ── Aabb ─────────────────────────────────────────────────────────────────

    #[test]
    fn aabb_center() {
        let aabb = Aabb::new(Vec3::ZERO, Vec3::new(4.0, 4.0, 4.0));
        assert_eq!(aabb.center(), Vec3::new(2.0, 2.0, 2.0));
    }

    #[test]
    fn aabb_half_extents() {
        let aabb = Aabb::new(Vec3::new(-1.0, -2.0, -3.0), Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(aabb.half_extents(), Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn aabb_contains_point_inside() {
        let aabb = Aabb::new(Vec3::ZERO, Vec3::splat(10.0));
        assert!(aabb.contains_point(Vec3::new(5.0, 5.0, 5.0)));
    }

    #[test]
    fn aabb_contains_point_on_boundary() {
        let aabb = Aabb::new(Vec3::ZERO, Vec3::splat(10.0));
        assert!(aabb.contains_point(Vec3::new(10.0, 0.0, 0.0)));
    }

    #[test]
    fn aabb_contains_point_outside() {
        let aabb = Aabb::new(Vec3::ZERO, Vec3::splat(10.0));
        assert!(!aabb.contains_point(Vec3::new(10.1, 0.0, 0.0)));
    }

    // ── ChunkState ────────────────────────────────────────────────────────────

    #[test]
    fn chunk_state_transitions() {
        let mut chunk = Chunk::new(ChunkCoord::new(0, 0, 0), 16.0);
        assert_eq!(chunk.state, ChunkState::Unloaded);
        assert!(!chunk.is_active());

        chunk.activate();
        assert_eq!(chunk.state, ChunkState::Active);
        assert!(chunk.is_active());

        chunk.deactivate();
        assert_eq!(chunk.state, ChunkState::Unloaded);
        assert!(!chunk.is_active());
    }

    // ── Chunk entity management ───────────────────────────────────────────────

    #[test]
    fn chunk_add_entity_idempotent() {
        let mut chunk = Chunk::new(ChunkCoord::new(0, 0, 0), 16.0);
        let id = EntityId::new(42);
        chunk.add_entity(id);
        chunk.add_entity(id); // duplicate — must not double-insert
        assert_eq!(chunk.entities.len(), 1);
    }

    #[test]
    fn chunk_remove_entity() {
        let mut chunk = Chunk::new(ChunkCoord::new(0, 0, 0), 16.0);
        let id = EntityId::new(1);
        chunk.add_entity(id);
        chunk.remove_entity(id);
        assert!(chunk.entities.is_empty());
    }

    #[test]
    fn chunk_bounds_match_coord() {
        let coord = ChunkCoord::new(2, 0, -1);
        let chunk = Chunk::new(coord, 16.0);
        // min should be (32, 0, -16)
        assert_eq!(chunk.bounds.min, Vec3::new(32.0, 0.0, -16.0));
        assert_eq!(chunk.bounds.max, Vec3::new(48.0, 16.0, 0.0));
    }
}
