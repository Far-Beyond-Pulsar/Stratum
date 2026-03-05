//! Chunk data generation — orchestrates terrain + structures into `ChunkFile`.

pub mod structures;
pub mod villages;

use stratum::{
    ChunkCoord,
    PlacementContext,
    level_fs::format::{ChunkFile, FORMAT_VERSION},
};

use crate::noise::hash;
use crate::prefabs::PrefabLibrary;
use crate::terrain::*;

/// Build the complete chunk data for `coord`, populating terrain voxels
/// and placing structures (trees, villages, towers, waterways).
///
/// **Optimization**: Terrain voxels are NOT stored as entities. Instead, they
/// are regenerated procedurally on load from the noise functions. Only placed
/// structures (prefab instances) are serialized, saving ~95% of disk space.
pub fn build_chunk_file(coord: ChunkCoord, lib: &PrefabLibrary) -> ChunkFile {
    let empty = || ChunkFile {
        version:          FORMAT_VERSION,
        coord:            [coord.x, coord.y, coord.z],
        entities:         vec![],
        prefab_instances: vec![],
    };

    if coord.y < 0 || coord.y >= MAX_Y_CHUNKS { return empty(); }

    let ox = coord.x * VOXELS_PER_CHUNK;
    let oz = coord.z * VOXELS_PER_CHUNK;

    let mut entities = Vec::new();
    let mut eid: u64 = 0;

    // ── Skip terrain voxels — they will be regenerated on load ────────────────
    // This eliminates ~768 EntityRecords per chunk and reduces disk I/O by ~95%.

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
