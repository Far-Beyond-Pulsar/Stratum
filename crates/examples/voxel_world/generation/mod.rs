//! Chunk data generation — orchestrates terrain + structures into `ChunkFile`.

pub mod structures;
pub mod villages;

use stratum::{
    ChunkCoord,
    PlacementContext,
    level_fs::format::{ChunkFile, EntityRecord, TransformRecord, FORMAT_VERSION},
};

use crate::biomes::{self, Biome};
use crate::noise::hash;
use crate::prefabs::PrefabLibrary;
use crate::terrain::*;

/// Build the complete chunk data for `coord`, populating terrain voxels
/// and placing structures (trees, villages, towers, waterways).
pub fn build_chunk_file(coord: ChunkCoord, lib: &PrefabLibrary) -> ChunkFile {
    let empty = || ChunkFile {
        version:          FORMAT_VERSION,
        coord:            [coord.x, coord.y, coord.z],
        entities:         vec![],
        prefab_instances: vec![],
    };

    if coord.y < 0 || coord.y >= MAX_Y_CHUNKS { return empty(); }

    let wy_min = coord.y * VOXELS_PER_CHUNK;
    let wy_max = wy_min + VOXELS_PER_CHUNK;
    let ox     = coord.x * VOXELS_PER_CHUNK;
    let oz     = coord.z * VOXELS_PER_CHUNK;

    let mut entities = Vec::new();
    let mut eid: u64 = 0;

    // ── Terrain voxels ──────────────────────────────────────────────────
    for lx in 0..VOXELS_PER_CHUNK {
        for lz in 0..VOXELS_PER_CHUNK {
            let wx = ox + lx;
            let wz = oz + lz;
            let surface_h = terrain_height(wx, wz);
            let mf = mountain_factor(wx as f32, wz as f32);
            let biome = biomes::biome_at(wx, wz, mf);

            // Determine how high to fill (includes water above terrain)
            let fill_top = if surface_h < WATER_LEVEL && biome != Biome::Desert {
                WATER_LEVEL
            } else {
                surface_h
            };

            let y_lo = wy_min;
            let y_hi = wy_max.min(fill_top + 1);

            for wy in y_lo..y_hi {
                let block = if wy <= surface_h {
                    surface_block(wy, surface_h, biome)
                } else if biome == Biome::Taiga && wy == WATER_LEVEL {
                    crate::blocks::Block::Ice   // frozen surface in taiga
                } else {
                    crate::blocks::Block::Water
                };

                eid += 1;
                entities.push(EntityRecord {
                    id: eid,
                    transform: Some(TransformRecord {
                        position: [wx as f32 + 0.5, wy as f32 + 0.5, wz as f32 + 0.5],
                        rotation: [0.0, 0.0, 0.0, 1.0],
                        scale:    [1.0, 1.0, 1.0],
                    }),
                    mesh:            None,
                    material:        Some(block.mat_index()),
                    light:           None,
                    billboard:       None,
                    bounding_radius: 0.866,
                    tags:            vec![],
                });
            }
        }
    }

    // ── Structures (only placed from y=0 chunks) ────────────────────────
    if coord.y == 0 {
        let mut ctx = PlacementContext::new();

        if let Some(vc) = villages::village_center_near(coord.x, coord.z) {
            villages::place_village_structures(
                coord, vc, &mut entities, &mut eid, lib, &mut ctx,
            );
        } else if hash(coord.x, coord.z, 88) < TOWER_PROBABILITY {
            structures::place_tower_in_chunk(
                ox, oz, coord.x, coord.z,
                &mut entities, &mut eid, lib, &mut ctx,
            );
        } else if hash(coord.x, coord.z, 93) < WATERWAY_PROBABILITY {
            structures::place_waterway_in_chunk(
                ox, oz, coord.x, coord.z,
                &mut entities, &mut eid, lib, &mut ctx,
            );
        } else {
            structures::place_trees_in_chunk(
                ox, oz, &mut entities, &mut eid, lib, &mut ctx,
            );
        }
    }

    ChunkFile {
        version:          FORMAT_VERSION,
        coord:            [coord.x, coord.y, coord.z],
        entities,
        prefab_instances: vec![],
    }
}
