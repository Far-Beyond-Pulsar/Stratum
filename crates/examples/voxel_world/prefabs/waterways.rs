//! Waterway structure prefabs — aqueduct pillar, aqueduct span, dock.

use glam::Vec3;
use stratum::{Aabb, Prefab, PrefabVolume};

use crate::blocks::Block;
use super::voxel_entity;

// ── Aqueduct pillar (3×3, 6 tall) ───────────────────────────────────────────

/// Stone-brick support pillar for raised water channels.
pub fn make_aqueduct_pillar() -> Prefab {
    let mut b = Prefab::builder("aqueduct_pillar");
    let size = 3_i32;
    let height = 6_i32;

    for y in 0..height {
        for x in 0..size {
            for z in 0..size {
                let on_edge = x == 0 || x == size - 1 || z == 0 || z == size - 1;
                // Hollow interior above base
                if !on_edge && y > 0 { continue; }
                b = b.with_entity(voxel_entity(
                    x as f32, y as f32, z as f32,
                    if y == 0 { Block::Cobblestone } else { Block::StoneBrick },
                ));
            }
        }
    }
    // Channel on top — 3-wide with water
    for x in 0..size {
        for z in 0..size {
            let is_rim = x == 0 || x == size - 1;
            b = b.with_entity(voxel_entity(
                x as f32, height as f32, z as f32,
                if is_rim { Block::StoneBrick } else { Block::Water },
            ));
        }
    }

    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::ZERO, Vec3::new(size as f32, (height + 1) as f32, size as f32),
    )));
    b.build()
}

// ── Aqueduct span (5×1×3) ───────────────────────────────────────────────────

/// Bridge section — 5 blocks long, stone-brick channel with water on top.
pub fn make_aqueduct_span() -> Prefab {
    let mut b = Prefab::builder("aqueduct_span");
    let length = 5_i32;
    let width  = 3_i32;

    for z in 0..length {
        for x in 0..width {
            let is_rim = x == 0 || x == width - 1;
            // Channel base
            b = b.with_entity(voxel_entity(x as f32, 0.0, z as f32, Block::StoneBrick));
            // Rim walls + water
            b = b.with_entity(voxel_entity(
                x as f32, 1.0, z as f32,
                if is_rim { Block::StoneBrick } else { Block::Water },
            ));
        }
    }
    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::ZERO, Vec3::new(width as f32, 2.0, length as f32),
    )));
    b.build()
}

// ── Dock (5×3, plank platform) ──────────────────────────────────────────────

/// Wooden dock platform extending over water, with support posts.
pub fn make_dock() -> Prefab {
    let mut b = Prefab::builder("dock");
    let w = 5_i32;
    let d = 3_i32;

    // Support posts at corners and middle
    for &(px, pz) in &[(0, 0), (w - 1, 0), (0, d - 1), (w - 1, d - 1), (w / 2, d - 1)] {
        for y in -2..0_i32 {
            b = b.with_entity(voxel_entity(px as f32, y as f32, pz as f32, Block::Wood));
        }
    }
    // Plank deck
    for x in 0..w {
        for z in 0..d {
            b = b.with_entity(voxel_entity(x as f32, 0.0, z as f32, Block::Plank));
        }
    }
    // Railing on two long sides
    for x in 0..w {
        b = b.with_entity(voxel_entity(x as f32, 1.0, 0.0, Block::Wood));
        b = b.with_entity(voxel_entity(x as f32, 1.0, (d - 1) as f32, Block::Wood));
    }
    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::new(0.0, -2.0, 0.0), Vec3::new(w as f32, 2.0, d as f32),
    )));
    b.build()
}
