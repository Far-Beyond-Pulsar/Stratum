//! Village infrastructure prefabs — well, fountain, lamppost.

use glam::Vec3;
use stratum::{Aabb, Prefab, PrefabVolume};

use crate::blocks::Block;
use super::voxel_entity;

// ── Well (3×3) ──────────────────────────────────────────────────────────────

/// Stone-brick well: 3×3 rim, hollow centre.
pub fn make_well() -> Prefab {
    let mut b = Prefab::builder("well");
    for x in 0..3_i32 {
        for z in 0..3_i32 {
            if x == 1 && z == 1 {
                // Hollow centre — floor 1 below ground
                b = b.with_entity(voxel_entity(x as f32, -1.0, z as f32, Block::StoneBrick));
            } else {
                b = b.with_entity(voxel_entity(x as f32, 0.0, z as f32, Block::StoneBrick));
                b = b.with_entity(voxel_entity(x as f32, 1.0, z as f32, Block::StoneBrick));
            }
        }
    }
    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::new(0.0, -1.0, 0.0), Vec3::new(3.0, 2.0, 3.0),
    )));
    b.build()
}

// ── Fountain (5×5) ──────────────────────────────────────────────────────────

/// Stone-brick fountain with water centre column.
pub fn make_fountain() -> Prefab {
    let mut b = Prefab::builder("fountain");
    for x in 0..5_i32 {
        for z in 0..5_i32 {
            let on_edge = x == 0 || x == 4 || z == 0 || z == 4;
            let is_center = x == 2 && z == 2;
            if on_edge {
                // Rim — 2 blocks high
                b = b.with_entity(voxel_entity(x as f32, 0.0, z as f32, Block::StoneBrick));
                b = b.with_entity(voxel_entity(x as f32, 1.0, z as f32, Block::StoneBrick));
            } else if is_center {
                // Centre column with water
                b = b.with_entity(voxel_entity(x as f32, 0.0, z as f32, Block::StoneBrick));
                b = b.with_entity(voxel_entity(x as f32, 1.0, z as f32, Block::StoneBrick));
                b = b.with_entity(voxel_entity(x as f32, 2.0, z as f32, Block::Water));
            } else {
                // Basin floor + water
                b = b.with_entity(voxel_entity(x as f32, 0.0, z as f32, Block::StoneBrick));
                b = b.with_entity(voxel_entity(x as f32, 1.0, z as f32, Block::Water));
            }
        }
    }
    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::ZERO, Vec3::new(5.0, 3.0, 5.0),
    )));
    b.build()
}

// ── Lamppost ────────────────────────────────────────────────────────────────

/// Cobblestone base, wood post, glass lantern top.
pub fn make_lamppost() -> Prefab {
    let mut b = Prefab::builder("lamppost");
    // Base
    b = b.with_entity(voxel_entity(0.0, 0.0, 0.0, Block::Cobblestone));
    // Post
    b = b.with_entity(voxel_entity(0.0, 1.0, 0.0, Block::Wood));
    b = b.with_entity(voxel_entity(0.0, 2.0, 0.0, Block::Wood));
    b = b.with_entity(voxel_entity(0.0, 3.0, 0.0, Block::Wood));
    // Lantern
    b = b.with_entity(voxel_entity(0.0, 4.0, 0.0, Block::Glass));
    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::ZERO, Vec3::new(1.0, 5.0, 1.0),
    )));
    b.build()
}
