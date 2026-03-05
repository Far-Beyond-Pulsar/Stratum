//! House prefab definitions — 7 biome-appropriate house types.

use glam::Vec3;
use stratum::{Aabb, Prefab, PrefabVolume};

use crate::blocks::Block;
use super::voxel_entity;

// ── Cottage (7×7, 4-wall) ───────────────────────────────────────────────────

/// Stone-brick cottage with plank roof and glass windows.
pub fn make_cottage() -> Prefab {
    let mut b = Prefab::builder("house_cottage");
    let w = 7_i32;
    let wall_h = 4_i32;

    for x in 0..w {
        for z in 0..w {
            b = b.with_entity(voxel_entity(x as f32, 0.0, z as f32, Block::Plank));
            let on_edge = x == 0 || x == w - 1 || z == 0 || z == w - 1;
            if on_edge {
                for y in 1..wall_h {
                    let is_window = y == 2
                        && ((x == 0 || x == w - 1) && z > 1 && z < w - 2
                            || (z == 0 || z == w - 1) && x > 1 && x < w - 2);
                    b = b.with_entity(voxel_entity(
                        x as f32, y as f32, z as f32,
                        if is_window { Block::Glass } else { Block::StoneBrick },
                    ));
                }
            }
            b = b.with_entity(voxel_entity(x as f32, wall_h as f32, z as f32, Block::Plank));
        }
    }
    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::ZERO, Vec3::new(w as f32, (wall_h + 1) as f32, w as f32),
    )));
    b = b.with_volume(PrefabVolume::hollow(Aabb::new(
        Vec3::ONE, Vec3::new((w - 1) as f32, wall_h as f32, (w - 1) as f32),
    )));
    b.build()
}

// ── Manor (9×9, chimney) ────────────────────────────────────────────────────

/// Large stone-brick manor with chimney.
pub fn make_manor() -> Prefab {
    let mut b = Prefab::builder("house_manor");
    let w = 9_i32;
    let wall_h = 5_i32;

    for x in 0..w {
        for z in 0..w {
            b = b.with_entity(voxel_entity(x as f32, 0.0, z as f32, Block::Plank));
            let on_edge = x == 0 || x == w - 1 || z == 0 || z == w - 1;
            if on_edge {
                for y in 1..wall_h {
                    let is_window = y == 2
                        && ((x == 0 || x == w - 1) && z > 1 && z < w - 2
                            || (z == 0 || z == w - 1) && x > 1 && x < w - 2);
                    b = b.with_entity(voxel_entity(
                        x as f32, y as f32, z as f32,
                        if is_window { Block::Glass } else { Block::StoneBrick },
                    ));
                }
            }
            b = b.with_entity(voxel_entity(x as f32, wall_h as f32, z as f32, Block::Plank));
        }
    }
    // Chimney — stone-brick column at back-right corner
    for y in 1..wall_h + 3 {
        b = b.with_entity(voxel_entity((w - 2) as f32, y as f32, (w - 2) as f32, Block::StoneBrick));
    }
    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::ZERO, Vec3::new(w as f32, (wall_h + 1) as f32, w as f32),
    )));
    b = b.with_volume(PrefabVolume::hollow(Aabb::new(
        Vec3::ONE, Vec3::new((w - 1) as f32, wall_h as f32, (w - 1) as f32),
    )));
    b.build()
}

// ── Tower house (5×5, 3 stories) ────────────────────────────────────────────

/// Narrow tower house: 5×5 footprint, 10 blocks tall, 3 stories with glass.
pub fn make_tower_house() -> Prefab {
    let mut b = Prefab::builder("house_tower");
    let w = 5_i32;
    let total_h = 10_i32;

    for x in 0..w {
        for z in 0..w {
            // Floor
            b = b.with_entity(voxel_entity(x as f32, 0.0, z as f32, Block::Plank));
            let on_edge = x == 0 || x == w - 1 || z == 0 || z == w - 1;
            if on_edge {
                for y in 1..total_h {
                    // Windows at y=2, y=5, y=8 (each story)
                    let is_window = (y == 2 || y == 5 || y == 8)
                        && ((x == 0 || x == w - 1) && z > 0 && z < w - 1
                            || (z == 0 || z == w - 1) && x > 0 && x < w - 1);
                    b = b.with_entity(voxel_entity(
                        x as f32, y as f32, z as f32,
                        if is_window { Block::Glass } else { Block::StoneBrick },
                    ));
                }
            } else {
                // Interior floors at y=3 and y=6
                for &floor_y in &[3, 6] {
                    b = b.with_entity(voxel_entity(x as f32, floor_y as f32, z as f32, Block::Plank));
                }
            }
            // Roof
            b = b.with_entity(voxel_entity(x as f32, total_h as f32, z as f32, Block::Plank));
        }
    }
    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::ZERO, Vec3::new(w as f32, (total_h + 1) as f32, w as f32),
    )));
    b = b.with_volume(PrefabVolume::hollow(Aabb::new(
        Vec3::ONE, Vec3::new((w - 1) as f32, total_h as f32, (w - 1) as f32),
    )));
    b.build()
}

// ── Market stall (5×3, open front) ──────────────────────────────────────────

/// Open-air market stall: plank canopy on posts, 5×3 footprint.
pub fn make_market_stall() -> Prefab {
    let mut b = Prefab::builder("market_stall");
    let w = 5_i32;
    let d = 3_i32;
    let h = 3_i32;

    // Four corner posts
    for &(px, pz) in &[(0, 0), (w - 1, 0), (0, d - 1), (w - 1, d - 1)] {
        for y in 0..h {
            b = b.with_entity(voxel_entity(px as f32, y as f32, pz as f32, Block::Wood));
        }
    }
    // Plank canopy roof
    for x in 0..w {
        for z in 0..d {
            b = b.with_entity(voxel_entity(x as f32, h as f32, z as f32, Block::Plank));
        }
    }
    // Counter — cobblestone shelf at height 1, front edge
    for x in 1..w - 1 {
        b = b.with_entity(voxel_entity(x as f32, 1.0, 0.0, Block::Cobblestone));
    }
    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::ZERO, Vec3::new(w as f32, (h + 1) as f32, d as f32),
    )));
    b.build()
}

// ── Barn (11×7, wide door) ──────────────────────────────────────────────────

/// Large barn: plank walls, thatch roof, wide door opening.
pub fn make_barn() -> Prefab {
    let mut b = Prefab::builder("barn");
    let w = 11_i32;
    let d = 7_i32;
    let wall_h = 5_i32;

    for x in 0..w {
        for z in 0..d {
            // Floor
            b = b.with_entity(voxel_entity(x as f32, 0.0, z as f32, Block::Cobblestone));
            let on_edge = x == 0 || x == w - 1 || z == 0 || z == d - 1;
            if on_edge {
                for y in 1..wall_h {
                    // Wide door opening on front face (z=0), center 3 blocks, 3 tall
                    let is_door = z == 0 && x >= 4 && x <= 6 && y <= 3;
                    if is_door { continue; }
                    b = b.with_entity(voxel_entity(x as f32, y as f32, z as f32, Block::Plank));
                }
            }
            // Thatch roof
            b = b.with_entity(voxel_entity(x as f32, wall_h as f32, z as f32, Block::Thatch));
        }
    }
    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::ZERO, Vec3::new(w as f32, (wall_h + 1) as f32, d as f32),
    )));
    b = b.with_volume(PrefabVolume::hollow(Aabb::new(
        Vec3::ONE, Vec3::new((w - 1) as f32, wall_h as f32, (d - 1) as f32),
    )));
    b.build()
}

// ── Desert house (7×7, flat sandstone roof) ─────────────────────────────────

/// Sandstone desert house with flat roof and small windows.
pub fn make_desert_house() -> Prefab {
    let mut b = Prefab::builder("house_desert");
    let w = 7_i32;
    let wall_h = 4_i32;

    for x in 0..w {
        for z in 0..w {
            b = b.with_entity(voxel_entity(x as f32, 0.0, z as f32, Block::Sandstone));
            let on_edge = x == 0 || x == w - 1 || z == 0 || z == w - 1;
            if on_edge {
                for y in 1..wall_h {
                    // Small windows — only one per wall face, at height 2
                    let is_window = y == 2
                        && ((x == 0 || x == w - 1) && z == w / 2
                            || (z == 0 || z == w - 1) && x == w / 2);
                    b = b.with_entity(voxel_entity(
                        x as f32, y as f32, z as f32,
                        if is_window { Block::Glass } else { Block::Sandstone },
                    ));
                }
            }
            // Flat sandstone roof
            b = b.with_entity(voxel_entity(x as f32, wall_h as f32, z as f32, Block::Sandstone));
        }
    }
    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::ZERO, Vec3::new(w as f32, (wall_h + 1) as f32, w as f32),
    )));
    b = b.with_volume(PrefabVolume::hollow(Aabb::new(
        Vec3::ONE, Vec3::new((w - 1) as f32, wall_h as f32, (w - 1) as f32),
    )));
    b.build()
}

// ── Taiga cabin (7×7, thick plank walls) ────────────────────────────────────

/// Sturdy wooden cabin with double-thick plank walls.
pub fn make_taiga_cabin() -> Prefab {
    let mut b = Prefab::builder("house_taiga");
    let w = 7_i32;
    let wall_h = 4_i32;

    for x in 0..w {
        for z in 0..w {
            b = b.with_entity(voxel_entity(x as f32, 0.0, z as f32, Block::Plank));
            // Double-thick walls (outer 2 layers are wall)
            let outer_edge = x == 0 || x == w - 1 || z == 0 || z == w - 1;
            let inner_edge = x == 1 || x == w - 2 || z == 1 || z == w - 2;
            let is_interior = x >= 2 && x <= w - 3 && z >= 2 && z <= w - 3;
            if outer_edge || (inner_edge && !is_interior) {
                for y in 1..wall_h {
                    // Windows only on outer wall, height 2, center of each face
                    let is_window = y == 2 && outer_edge
                        && ((x == 0 || x == w - 1) && z == w / 2
                            || (z == 0 || z == w - 1) && x == w / 2);
                    b = b.with_entity(voxel_entity(
                        x as f32, y as f32, z as f32,
                        if is_window { Block::Glass } else { Block::Plank },
                    ));
                }
            }
            // Plank roof
            b = b.with_entity(voxel_entity(x as f32, wall_h as f32, z as f32, Block::Plank));
        }
    }
    // Chimney — stone-brick
    for y in 1..wall_h + 2 {
        b = b.with_entity(voxel_entity(1.0, y as f32, 1.0, Block::StoneBrick));
    }
    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::ZERO, Vec3::new(w as f32, (wall_h + 1) as f32, w as f32),
    )));
    b = b.with_volume(PrefabVolume::hollow(Aabb::new(
        Vec3::new(2.0, 1.0, 2.0), Vec3::new((w - 2) as f32, wall_h as f32, (w - 2) as f32),
    )));
    b.build()
}
