//! Village macro-grid and biome-aware house placement.

use glam::Vec3;
use stratum::{
    ChunkCoord, PlacementContext, Prefab,
    level_fs::format::EntityRecord,
};

use crate::biomes::{self, Biome};
use crate::noise::hash;
use crate::prefabs::PrefabLibrary;
use crate::terrain::*;

use super::structures::emit_prefab_entities;

// ── Village grid ────────────────────────────────────────────────────────────

/// Returns the chunk-coordinate of the village center inside macro-cell (cx, cz),
/// or `None` if this cell has no village.
pub fn village_center_for_cell(cell_x: i32, cell_z: i32) -> Option<(i32, i32)> {
    if hash(cell_x, cell_z, 77) > VILLAGE_PROBABILITY { return None; }
    let margin = 3.0;
    let range  = (VILLAGE_GRID as f32) - margin * 2.0;
    let fx = margin + hash(cell_x, cell_z, 78) * range;
    let fz = margin + hash(cell_x, cell_z, 79) * range;
    Some((
        cell_x * VILLAGE_GRID + fx as i32,
        cell_z * VILLAGE_GRID + fz as i32,
    ))
}

/// If chunk `(chunk_x, chunk_z)` is within any village's radius, return that
/// village's center chunk-coord.
pub fn village_center_near(chunk_x: i32, chunk_z: i32) -> Option<(i32, i32)> {
    let cell_x = chunk_x.div_euclid(VILLAGE_GRID);
    let cell_z = chunk_z.div_euclid(VILLAGE_GRID);
    for dcx in -1..=1 {
        for dcz in -1..=1 {
            if let Some((vcx, vcz)) = village_center_for_cell(cell_x + dcx, cell_z + dcz) {
                if (chunk_x - vcx).abs() <= VILLAGE_CHUNK_RADIUS
                && (chunk_z - vcz).abs() <= VILLAGE_CHUNK_RADIUS {
                    return Some((vcx, vcz));
                }
            }
        }
    }
    None
}

// ── Biome-aware house selection ─────────────────────────────────────────────

/// Select a house prefab for a village in the given biome.
fn select_house<'a>(
    index: usize, biome: Biome, lib: &'a PrefabLibrary,
) -> &'a Prefab {
    match biome {
        Biome::Desert => {
            if index % 4 == 0 { &lib.market_stall } else { &lib.desert_house }
        }
        Biome::Taiga => {
            match index % 4 {
                0 => &lib.barn,
                1 => &lib.cottage,
                _ => &lib.taiga_cabin,
            }
        }
        Biome::Swamp => &lib.cottage,
        Biome::Jungle => {
            if index % 3 == 0 { &lib.market_stall } else { &lib.cottage }
        }
        Biome::Mountains => {
            if index % 3 == 0 { &lib.tower_house } else { &lib.cottage }
        }
        // Plains / Forest — full variety
        _ => match index % 6 {
            0 => &lib.manor,
            1 => &lib.tower_house,
            2 => &lib.barn,
            3 => &lib.market_stall,
            _ => &lib.cottage,
        },
    }
}

/// Select village centre-piece based on biome.
fn select_centerpiece<'a>(biome: Biome, lib: &'a PrefabLibrary) -> &'a Prefab {
    match biome {
        Biome::Plains | Biome::Forest => &lib.fountain,
        _ => &lib.well,
    }
}

// ── Village structure placement ─────────────────────────────────────────────

/// Place village structures for a chunk within a village's radius.
pub fn place_village_structures(
    chunk_coord:    ChunkCoord,
    village_center: (i32, i32),
    entities:       &mut Vec<EntityRecord>,
    eid:            &mut u64,
    lib:            &PrefabLibrary,
    ctx:            &mut PlacementContext,
) {
    use stratum::{Level, LevelId};
    let mut scratch = Level::new(LevelId::new(0), "scratch", 32.0, 10000.0);

    let (vcx, vcz) = village_center;
    let vcwx = vcx * VOXELS_PER_CHUNK + VOXELS_PER_CHUNK / 2;
    let vcwz = vcz * VOXELS_PER_CHUNK + VOXELS_PER_CHUNK / 2;

    // Reject village on mountainous terrain
    let center_h = terrain_height(vcwx, vcwz);
    if center_h > STRUCTURE_HEIGHT_CUTOFF { return; }

    let mf = mountain_factor(vcwx as f32, vcwz as f32);
    let biome = biomes::biome_at(vcwx, vcwz, mf);

    let ox = chunk_coord.x * VOXELS_PER_CHUNK;
    let oz = chunk_coord.z * VOXELS_PER_CHUNK;

    // Centre-piece — placed only from the centre chunk
    if chunk_coord.x == vcx && chunk_coord.z == vcz {
        let piece = select_centerpiece(biome, lib);
        let wp = Vec3::new(vcwx as f32 - 1.0, center_h as f32 + 1.0, vcwz as f32 - 1.0);
        if ctx.place(piece, wp, &mut scratch).is_ok() {
            emit_prefab_entities(piece, wp, entities, eid);
        }

        // Lampposts around centre (4 cardinal directions)
        for &(dx, dz) in &[(3, 0), (-3, 0), (0, 3), (0, -3)] {
            let lx = vcwx + dx;
            let lz = vcwz + dz;
            let lh = terrain_height(lx, lz);
            let lp = Vec3::new(lx as f32, lh as f32 + 1.0, lz as f32);
            if ctx.place(&lib.lamppost, lp, &mut scratch).is_ok() {
                emit_prefab_entities(&lib.lamppost, lp, entities, eid);
            }
        }

        // Dock if village is near water
        if center_h <= WATER_LEVEL + 3 {
            let dock_z = vcwz + VOXELS_PER_CHUNK / 2 + 2;
            let dock_h = terrain_height(vcwx, dock_z);
            if dock_h <= WATER_LEVEL {
                let dp = Vec3::new(vcwx as f32 - 2.0, WATER_LEVEL as f32 + 1.0, dock_z as f32);
                if ctx.place(&lib.dock, dp, &mut scratch).is_ok() {
                    emit_prefab_entities(&lib.dock, dp, entities, eid);
                }
            }
        }
    }

    // Houses — deterministic set from village center
    let house_count = VILLAGE_HOUSE_MIN
        + (hash(vcx, vcz, 55) * (VILLAGE_HOUSE_MAX - VILLAGE_HOUSE_MIN) as f32) as usize;

    for i in 0..house_count {
        let angle  = hash(vcx + i as i32, vcz,       60 + i as u64) * std::f32::consts::TAU;
        let radius = 8.0 + hash(vcx, vcz + i as i32, 70 + i as u64) * 22.0;
        let hwx_f  = vcwx as f32 + angle.cos() * radius;
        let hwz_f  = vcwz as f32 + angle.sin() * radius;
        let hwx    = hwx_f as i32;
        let hwz    = hwz_f as i32;

        // Only emit from the chunk that contains this house's origin
        if hwx < ox || hwx >= ox + VOXELS_PER_CHUNK { continue; }
        if hwz < oz || hwz >= oz + VOXELS_PER_CHUNK { continue; }

        let h = terrain_height(hwx + 3, hwz + 3);
        if h > STRUCTURE_HEIGHT_CUTOFF { continue; }
        let world_pos = Vec3::new(hwx_f, h as f32 + 1.0, hwz_f);

        let house = select_house(i, biome, lib);
        if ctx.place(house, world_pos, &mut scratch).is_ok() {
            emit_prefab_entities(house, world_pos, entities, eid);
        }
    }
}
