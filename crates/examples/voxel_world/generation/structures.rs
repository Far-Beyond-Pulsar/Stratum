//! Structure placement — trees, towers, waterway features.

use glam::Vec3;
use stratum::{
    ChunkCoord, PlacementContext, Prefab,
    level_fs::format::{EntityRecord, TransformRecord},
};

use crate::biomes::{self, Biome};
use crate::noise::hash;
use crate::prefabs::PrefabLibrary;
use crate::terrain::*;

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Append world-space `EntityRecord`s for all entities in `prefab` placed at `world_pos`.
pub fn emit_prefab_entities(
    prefab:    &Prefab,
    world_pos: Vec3,
    entities:  &mut Vec<EntityRecord>,
    eid:       &mut u64,
) {
    for template in &prefab.entities {
        let Some(ref lt) = template.transform else { continue };
        let wp = lt.position + world_pos;
        *eid += 1;
        entities.push(EntityRecord {
            id:              *eid,
            transform:       Some(TransformRecord {
                position: wp.to_array(),
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale:    [1.0, 1.0, 1.0],
            }),
            mesh:            None,
            material:        template.material.map(|m| m.0),
            light:           None,
            billboard:       None,
            bounding_radius: 0.866,
            tags:            vec![],
        });
    }
}

// ── Tree placement ──────────────────────────────────────────────────────────

/// Select tree prefabs appropriate for the given biome.
fn select_tree<'a>(
    wx: i32, wz: i32, biome: Biome, lib: &'a PrefabLibrary,
) -> Option<&'a Prefab> {
    match biome {
        Biome::Plains => {
            if hash(wx, wz, 55) < 0.15 { Some(&lib.birch) } else { Some(&lib.oak) }
        }
        Biome::Forest => {
            let r = hash(wx, wz, 55);
            if r < 0.20 { Some(&lib.pine) }
            else if r < 0.40 { Some(&lib.birch) }
            else { Some(&lib.oak) }
        }
        Biome::Taiga => Some(&lib.pine),
        Biome::Jungle => {
            if hash(wx, wz, 55) < 0.35 { Some(&lib.jungle_tree) } else { Some(&lib.oak) }
        }
        Biome::Swamp => {
            if hash(wx, wz, 55) < 0.40 { Some(&lib.dead_tree) } else { Some(&lib.willow) }
        }
        Biome::Desert => Some(&lib.cactus),
        Biome::Mountains => {
            if hash(wx, wz, 55) < 0.70 { Some(&lib.pine) } else { None }
        }
    }
}

/// Should trees be placed at this world position, given the biome?
fn should_place_tree(wx: i32, wz: i32, biome: Biome) -> bool {
    let forest = forest_noise_at(wx as f32, wz as f32);
    match biome {
        Biome::Forest  => forest > 0.35,
        Biome::Jungle  => forest > 0.30,
        Biome::Taiga   => forest > 0.40,
        Biome::Swamp   => forest > 0.50,
        Biome::Plains  => forest > FOREST_THRESHOLD,
        Biome::Desert  => hash(wx / 7, wz / 7, 56) < 0.12,
        Biome::Mountains => forest > 0.55 && terrain_height(wx, wz) < 20,
    }
}

/// Place trees across a chunk using a regular grid + biome noise filter.
pub fn place_trees_in_chunk(
    ox: i32, oz: i32,
    entities: &mut Vec<EntityRecord>, eid: &mut u64,
    lib: &PrefabLibrary,
    ctx: &mut PlacementContext,
) {
    use stratum::{Level, LevelId};
    let mut scratch = Level::new(LevelId::new(0), "scratch", 32.0, 10000.0);

    let mut gx = ox;
    while gx < ox + VOXELS_PER_CHUNK {
        let mut gz = oz;
        while gz < oz + VOXELS_PER_CHUNK {
            let mf = mountain_factor(gx as f32, gz as f32);
            let biome = biomes::biome_at(gx, gz, mf);

            if should_place_tree(gx, gz, biome) {
                let spot = tree_spot_noise(gx as f32, gz as f32);
                let threshold = FOREST_TREE_SPOT_MIN - (forest_noise_at(gx as f32, gz as f32) - 0.3) * 0.35;
                if spot > threshold || biome == Biome::Desert {
                    let h = terrain_height(gx, gz);
                    if h >= BASE_HEIGHT && h <= STRUCTURE_HEIGHT_CUTOFF {
                        if let Some(tree) = select_tree(gx, gz, biome, lib) {
                            let world_pos = Vec3::new(gx as f32, h as f32 + 1.0, gz as f32);
                            if ctx.place(tree, world_pos, &mut scratch).is_ok() {
                                emit_prefab_entities(tree, world_pos, entities, eid);
                            }
                        }
                    }
                }
            }
            gz += TREE_GRID_STEP;
        }
        gx += TREE_GRID_STEP;
    }
}

// ── Tower placement ─────────────────────────────────────────────────────────

/// Select tower type based on chunk noise.
fn select_tower<'a>(chunk_x: i32, chunk_z: i32, biome: Biome, lib: &'a PrefabLibrary) -> &'a Prefab {
    let r = hash(chunk_x * 3, chunk_z * 3, 90);
    match biome {
        // Near water → lighthouse
        _ if terrain_height(
            chunk_x * VOXELS_PER_CHUNK + VOXELS_PER_CHUNK / 2,
            chunk_z * VOXELS_PER_CHUNK + VOXELS_PER_CHUNK / 2,
        ) <= WATER_LEVEL + 3 => &lib.lighthouse,
        // Otherwise ruined or watchtower
        _ if r < 0.60 => &lib.ruined_tower,
        _ => &lib.watchtower,
    }
}

/// Place a tower at a deterministic position within the chunk.
pub fn place_tower_in_chunk(
    ox: i32, oz: i32,
    chunk_x: i32, chunk_z: i32,
    entities: &mut Vec<EntityRecord>, eid: &mut u64,
    lib: &PrefabLibrary,
    ctx: &mut PlacementContext,
) {
    use stratum::{Level, LevelId};
    let mut scratch = Level::new(LevelId::new(0), "scratch", 32.0, 10000.0);

    let mid_x = ox + VOXELS_PER_CHUNK / 2;
    let mid_z = oz + VOXELS_PER_CHUNK / 2;
    if is_forest_zone(mid_x, mid_z) { return; }

    let fx = 3.0 + hash(chunk_x, chunk_z, 81) * (VOXELS_PER_CHUNK as f32 - 6.0);
    let fz = 3.0 + hash(chunk_x, chunk_z, 82) * (VOXELS_PER_CHUNK as f32 - 6.0);
    let wx = ox + fx as i32;
    let wz = oz + fz as i32;
    let h = terrain_height(wx, wz);
    if h < BASE_HEIGHT + 1 || h > STRUCTURE_HEIGHT_CUTOFF { return; }

    let mf = mountain_factor(wx as f32, wz as f32);
    let biome = biomes::biome_at(wx, wz, mf);
    let tower = select_tower(chunk_x, chunk_z, biome, lib);
    let world_pos = Vec3::new(wx as f32, h as f32 + 1.0, wz as f32);
    if ctx.place(tower, world_pos, &mut scratch).is_ok() {
        emit_prefab_entities(tower, world_pos, entities, eid);
    }
}

// ── Waterway placement ──────────────────────────────────────────────────────

/// Place waterway structures (aqueduct + dock) in chunks near water.
pub fn place_waterway_in_chunk(
    ox: i32, oz: i32,
    chunk_x: i32, chunk_z: i32,
    entities: &mut Vec<EntityRecord>, eid: &mut u64,
    lib: &PrefabLibrary,
    ctx: &mut PlacementContext,
) {
    use stratum::{Level, LevelId};
    let mut scratch = Level::new(LevelId::new(0), "scratch", 32.0, 10000.0);

    // Check if chunk has terrain near water level
    let mid_h = terrain_height(ox + VOXELS_PER_CHUNK / 2, oz + VOXELS_PER_CHUNK / 2);
    let mf = mountain_factor((ox + VOXELS_PER_CHUNK / 2) as f32, (oz + VOXELS_PER_CHUNK / 2) as f32);
    let biome = biomes::biome_at(ox + VOXELS_PER_CHUNK / 2, oz + VOXELS_PER_CHUNK / 2, mf);

    if biome == Biome::Desert { return; }

    let near_water = mid_h <= WATER_LEVEL + 2;
    if !near_water { return; }

    let fx = 2.0 + hash(chunk_x, chunk_z, 91) * (VOXELS_PER_CHUNK as f32 - 4.0);
    let fz = 2.0 + hash(chunk_x, chunk_z, 92) * (VOXELS_PER_CHUNK as f32 - 4.0);
    let wx = ox + fx as i32;
    let wz = oz + fz as i32;
    let h = terrain_height(wx, wz);

    // Dock at water edge
    if h <= WATER_LEVEL && h >= WATER_LEVEL - 2 {
        let world_pos = Vec3::new(wx as f32, WATER_LEVEL as f32 + 1.0, wz as f32);
        if ctx.place(&lib.dock, world_pos, &mut scratch).is_ok() {
            emit_prefab_entities(&lib.dock, world_pos, entities, eid);
        }
    }
    // Aqueduct pillar on slightly higher ground
    else if h > WATER_LEVEL && h <= WATER_LEVEL + 3 {
        let world_pos = Vec3::new(wx as f32, h as f32 + 1.0, wz as f32);
        if ctx.place(&lib.aqueduct_pillar, world_pos, &mut scratch).is_ok() {
            emit_prefab_entities(&lib.aqueduct_pillar, world_pos, entities, eid);
        }
    }
}
