//! Tower prefabs — ruined tower, watchtower, lighthouse.

use glam::Vec3;
use stratum::{Aabb, Prefab, PrefabVolume};

use crate::blocks::Block;
use crate::noise::hash;
use super::voxel_entity;

// ── Ruined tower (3×3, 8 tall, decaying) ────────────────────────────────────

/// Ruined tower: 3×3 base, 8 blocks tall, blocks randomly missing higher up.
pub fn make_ruined_tower() -> Prefab {
    let mut b = Prefab::builder("ruined_tower");
    let size   = 3_i32;
    let height = 8_i32;

    for y in 0..height {
        for x in 0..size {
            for z in 0..size {
                let is_wall = x == 0 || x == size - 1 || z == 0 || z == size - 1;
                let is_base = y == 0;
                if !is_wall && !is_base { continue; }

                let decay_prob = ((y as f32 - 1.0) / height as f32 * 0.7).max(0.0);
                if y > 1 && hash(x * 31 + y * 7, z * 13 + y * 11, 88) < decay_prob { continue; }

                b = b.with_entity(voxel_entity(
                    x as f32, y as f32, z as f32,
                    if y < 2 { Block::MossyStone } else { Block::StoneBrick },
                ));
            }
        }
    }
    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::ZERO, Vec3::new(size as f32, height as f32, size as f32),
    )));
    b.build()
}

// ── Watchtower (5×5, 12 tall, intact) ───────────────────────────────────────

/// Intact watchtower: 5×5 base, 12 tall, stone-brick walls, plank observation platform.
pub fn make_watchtower() -> Prefab {
    let mut b = Prefab::builder("watchtower");
    let size   = 5_i32;
    let height = 12_i32;

    for x in 0..size {
        for z in 0..size {
            // Foundation
            b = b.with_entity(voxel_entity(x as f32, 0.0, z as f32, Block::Cobblestone));

            let on_edge = x == 0 || x == size - 1 || z == 0 || z == size - 1;
            if on_edge {
                for y in 1..height {
                    // Door opening at front
                    let is_door = z == 0 && x == size / 2 && y <= 2;
                    // Arrow slits every 3 levels
                    let is_slit = (y == 4 || y == 7 || y == 10)
                        && ((x == 0 || x == size - 1) && z == size / 2
                            || (z == 0 || z == size - 1) && x == size / 2);
                    if is_door { continue; }
                    b = b.with_entity(voxel_entity(
                        x as f32, y as f32, z as f32,
                        if is_slit { Block::Glass } else { Block::StoneBrick },
                    ));
                }
            }
        }
    }
    // Interior floors at y=4, y=8
    for &floor_y in &[4_i32, 8] {
        for x in 1..size - 1 {
            for z in 1..size - 1 {
                b = b.with_entity(voxel_entity(x as f32, floor_y as f32, z as f32, Block::Plank));
            }
        }
    }
    // Observation platform — extends 1 block past walls
    for x in -1..size + 1 {
        for z in -1..size + 1 {
            b = b.with_entity(voxel_entity(x as f32, height as f32, z as f32, Block::Plank));
        }
    }
    // Crenellations on platform edge
    for x in -1..size + 1 {
        for z in -1..size + 1 {
            let on_platform_edge = x == -1 || x == size || z == -1 || z == size;
            if on_platform_edge && (x + z) % 2 == 0 {
                b = b.with_entity(voxel_entity(
                    x as f32, (height + 1) as f32, z as f32, Block::StoneBrick,
                ));
            }
        }
    }
    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::new(-1.0, 0.0, -1.0), Vec3::new((size + 1) as f32, (height + 2) as f32, (size + 1) as f32),
    )));
    b = b.with_volume(PrefabVolume::hollow(Aabb::new(
        Vec3::ONE, Vec3::new((size - 1) as f32, height as f32, (size - 1) as f32),
    )));
    b.build()
}

// ── Lighthouse (3×3, 10 tall, glass top) ────────────────────────────────────

/// Coastal lighthouse: 3×3 stone base, glass lantern room at top.
pub fn make_lighthouse() -> Prefab {
    let mut b = Prefab::builder("lighthouse");
    let size   = 3_i32;
    let height = 10_i32;
    let glass_start = 7_i32;

    for x in 0..size {
        for z in 0..size {
            // Foundation
            b = b.with_entity(voxel_entity(x as f32, 0.0, z as f32, Block::Cobblestone));

            let on_edge = x == 0 || x == size - 1 || z == 0 || z == size - 1;
            if on_edge {
                for y in 1..height {
                    let block = if y >= glass_start {
                        Block::Glass
                    } else {
                        Block::StoneBrick
                    };
                    // Door
                    let is_door = z == 0 && x == 1 && y <= 2;
                    if is_door { continue; }
                    b = b.with_entity(voxel_entity(x as f32, y as f32, z as f32, block));
                }
            }
        }
    }
    // Interior floors at y=4, y=7
    for &floor_y in &[4_i32, glass_start] {
        for x in 1..size - 1 {
            for z in 1..size - 1 {
                b = b.with_entity(voxel_entity(x as f32, floor_y as f32, z as f32, Block::Plank));
            }
        }
    }
    // Roof cap
    for x in 0..size {
        for z in 0..size {
            b = b.with_entity(voxel_entity(x as f32, height as f32, z as f32, Block::StoneBrick));
        }
    }
    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::ZERO, Vec3::new(size as f32, (height + 1) as f32, size as f32),
    )));
    b = b.with_volume(PrefabVolume::hollow(Aabb::new(
        Vec3::ONE, Vec3::new((size - 1) as f32, height as f32, (size - 1) as f32),
    )));
    b.build()
}
