//! Tree prefab definitions — 7 biome-appropriate tree types.

use glam::Vec3;
use stratum::{Aabb, Prefab, PrefabVolume};

use crate::blocks::Block;
use super::voxel_entity;

// ── Oak ─────────────────────────────────────────────────────────────────────

/// Standard oak tree: 5-block trunk + rounded 5×4×5 canopy.
pub fn make_oak() -> Prefab {
    let mut b = Prefab::builder("tree_oak");
    for y in 0..5_i32 {
        b = b.with_entity(voxel_entity(-0.5, y as f32, -0.5, Block::Wood));
    }
    for dy in 0..4_i32 {
        let r: i32 = match dy { 1 | 2 => 2, _ => 1 };
        for dx in -r..=r {
            for dz in -r..=r {
                if r == 2 && dx.abs() == 2 && dz.abs() == 2 && (dy == 0 || dy == 3) { continue; }
                b = b.with_entity(voxel_entity(
                    dx as f32 - 0.5, (dy + 3) as f32, dz as f32 - 0.5, Block::Leaves,
                ));
            }
        }
    }
    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::new(-0.5, 0.0, -0.5), Vec3::new(0.5, 5.0, 0.5),
    )));
    b = b.with_volume(PrefabVolume::hollow(Aabb::new(
        Vec3::new(-2.5, 3.0, -2.5), Vec3::new(2.5, 7.0, 2.5),
    )));
    b.build()
}

// ── Pine ────────────────────────────────────────────────────────────────────

/// Tall conifer: 7-block trunk, narrow pointed canopy.
pub fn make_pine() -> Prefab {
    let mut b = Prefab::builder("tree_pine");
    for y in 0..7_i32 {
        b = b.with_entity(voxel_entity(-0.5, y as f32, -0.5, Block::Wood));
    }
    for dy in 0..5_i32 {
        let r: i32 = match dy { 0 => 2, 1 | 2 => 2, 3 => 1, _ => 0 };
        for dx in -r..=r {
            for dz in -r..=r {
                // Round off corners for the widest layers
                if r == 2 && dx.abs() == 2 && dz.abs() == 2 { continue; }
                b = b.with_entity(voxel_entity(
                    dx as f32 - 0.5, (dy + 4) as f32, dz as f32 - 0.5, Block::Leaves,
                ));
            }
        }
    }
    // Top spire
    b = b.with_entity(voxel_entity(-0.5, 9.0, -0.5, Block::Leaves));
    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::new(-0.5, 0.0, -0.5), Vec3::new(0.5, 7.0, 0.5),
    )));
    b = b.with_volume(PrefabVolume::hollow(Aabb::new(
        Vec3::new(-2.5, 4.0, -2.5), Vec3::new(2.5, 10.0, 2.5),
    )));
    b.build()
}

// ── Birch ───────────────────────────────────────────────────────────────────

/// Birch tree: 6-block pale trunk, 3-layer narrow canopy.
pub fn make_birch() -> Prefab {
    let mut b = Prefab::builder("tree_birch");
    // Tall thin trunk
    for y in 0..6_i32 {
        b = b.with_entity(voxel_entity(-0.5, y as f32, -0.5, Block::Wood));
    }
    // Compact canopy — 3 layers, radius 1-2
    for dy in 0..3_i32 {
        let r: i32 = if dy == 1 { 2 } else { 1 };
        for dx in -r..=r {
            for dz in -r..=r {
                if r == 2 && dx.abs() == 2 && dz.abs() == 2 { continue; }
                b = b.with_entity(voxel_entity(
                    dx as f32 - 0.5, (dy + 5) as f32, dz as f32 - 0.5, Block::Leaves,
                ));
            }
        }
    }
    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::new(-0.5, 0.0, -0.5), Vec3::new(0.5, 6.0, 0.5),
    )));
    b = b.with_volume(PrefabVolume::hollow(Aabb::new(
        Vec3::new(-2.5, 5.0, -2.5), Vec3::new(2.5, 8.0, 2.5),
    )));
    b.build()
}

// ── Willow ──────────────────────────────────────────────────────────────────

/// Willow tree: 5-block trunk, wide canopy with hanging leaf curtains.
pub fn make_willow() -> Prefab {
    let mut b = Prefab::builder("tree_willow");
    for y in 0..5_i32 {
        b = b.with_entity(voxel_entity(-0.5, y as f32, -0.5, Block::Wood));
    }
    // Wide canopy — radius 3, 2 layers
    for dy in 0..2_i32 {
        let r: i32 = 3;
        for dx in -r..=r {
            for dz in -r..=r {
                if dx.abs() == 3 && dz.abs() == 3 { continue; } // round corners
                b = b.with_entity(voxel_entity(
                    dx as f32 - 0.5, (dy + 5) as f32, dz as f32 - 0.5, Block::Leaves,
                ));
            }
        }
    }
    // Hanging curtains — leaves draping down around perimeter
    let r = 3_i32;
    for dx in -r..=r {
        for dz in -r..=r {
            let on_edge = dx.abs() >= 2 || dz.abs() >= 2;
            if !on_edge { continue; }
            if dx.abs() == 3 && dz.abs() == 3 { continue; }
            // Hang 2–3 blocks down from canopy base
            let hang = if (dx + dz) % 2 == 0 { 3 } else { 2 };
            for dy in 1..=hang {
                b = b.with_entity(voxel_entity(
                    dx as f32 - 0.5, (5 - dy) as f32, dz as f32 - 0.5, Block::Leaves,
                ));
            }
        }
    }
    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::new(-0.5, 0.0, -0.5), Vec3::new(0.5, 5.0, 0.5),
    )));
    b = b.with_volume(PrefabVolume::hollow(Aabb::new(
        Vec3::new(-3.5, 2.0, -3.5), Vec3::new(3.5, 7.0, 3.5),
    )));
    b.build()
}

// ── Jungle ──────────────────────────────────────────────────────────────────

/// Massive jungle tree: 10-block DarkWood trunk, wide DarkLeaves canopy, buttress roots.
pub fn make_jungle_tree() -> Prefab {
    let mut b = Prefab::builder("tree_jungle");
    // Tall trunk
    for y in 0..10_i32 {
        b = b.with_entity(voxel_entity(-0.5, y as f32, -0.5, Block::DarkWood));
    }
    // Buttress roots — flared base
    for &(dx, dz) in &[(1, 0), (-1, 0), (0, 1), (0, -1)] {
        for y in 0..2_i32 {
            b = b.with_entity(voxel_entity(
                dx as f32 - 0.5, y as f32, dz as f32 - 0.5, Block::DarkWood,
            ));
        }
    }
    // Massive canopy — 5 layers
    for dy in 0..5_i32 {
        let r: i32 = match dy { 0 => 2, 1 | 2 => 3, 3 => 2, _ => 1 };
        for dx in -r..=r {
            for dz in -r..=r {
                if r == 3 && dx.abs() == 3 && dz.abs() == 3 { continue; }
                b = b.with_entity(voxel_entity(
                    dx as f32 - 0.5, (dy + 8) as f32, dz as f32 - 0.5, Block::DarkLeaves,
                ));
            }
        }
    }
    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::new(-1.5, 0.0, -1.5), Vec3::new(1.5, 10.0, 1.5),
    )));
    b = b.with_volume(PrefabVolume::hollow(Aabb::new(
        Vec3::new(-3.5, 8.0, -3.5), Vec3::new(3.5, 13.0, 3.5),
    )));
    b.build()
}

// ── Cactus ──────────────────────────────────────────────────────────────────

/// Desert cactus: 3–4 blocks tall with one arm.
pub fn make_cactus() -> Prefab {
    let mut b = Prefab::builder("cactus");
    // Main stem — 4 tall
    for y in 0..4_i32 {
        b = b.with_entity(voxel_entity(-0.5, y as f32, -0.5, Block::Cactus));
    }
    // One arm: extends +X at height 2, then up 1
    b = b.with_entity(voxel_entity(0.5, 2.0, -0.5, Block::Cactus));
    b = b.with_entity(voxel_entity(0.5, 3.0, -0.5, Block::Cactus));

    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::new(-0.5, 0.0, -0.5), Vec3::new(1.5, 4.0, 0.5),
    )));
    b.build()
}

// ── Dead tree ───────────────────────────────────────────────────────────────

/// Leafless dead tree: 4-block trunk + 2 bare branches.
pub fn make_dead_tree() -> Prefab {
    let mut b = Prefab::builder("tree_dead");
    for y in 0..4_i32 {
        b = b.with_entity(voxel_entity(-0.5, y as f32, -0.5, Block::Wood));
    }
    // Branch 1: extends +X at height 3
    b = b.with_entity(voxel_entity(0.5, 3.0, -0.5, Block::Wood));
    b = b.with_entity(voxel_entity(1.5, 3.0, -0.5, Block::Wood));
    // Branch 2: extends -Z at height 2
    b = b.with_entity(voxel_entity(-0.5, 2.0, -1.5, Block::Wood));
    b = b.with_entity(voxel_entity(-0.5, 2.0, -2.5, Block::Wood));

    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::new(-0.5, 0.0, -2.5), Vec3::new(2.0, 4.0, 0.5),
    )));
    b.build()
}
