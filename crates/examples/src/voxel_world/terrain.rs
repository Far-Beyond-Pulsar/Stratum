//! Procedural terrain generation for voxel chunks.

use super::blocks::Block;
use super::voxel_chunk::{VoxelChunk, CHUNK_SIZE};

/// Simple hash function for noise generation.
fn hash(x: i32, y: i32, z: i32) -> u32 {
    let mut h = (x as u32)
        .wrapping_mul(374761393)
        .wrapping_add(y as u32)
        .wrapping_mul(668265263)
        .wrapping_add(z as u32);
    h ^= h >> 13;
    h = h.wrapping_mul(1274126177);
    h ^= h >> 16;
    h
}

/// Simple 2D noise function (0.0 to 1.0).
fn noise_2d(x: i32, z: i32) -> f32 {
    (hash(x, 0, z) as f32 / u32::MAX as f32)
}

/// Smooth 2D noise with interpolation.
fn smooth_noise_2d(x: f32, z: f32) -> f32 {
    let x0 = x.floor() as i32;
    let z0 = z.floor() as i32;
    let x1 = x0 + 1;
    let z1 = z0 + 1;

    let fx = x - x0 as f32;
    let fz = z - z0 as f32;

    // Smooth interpolation
    let u = fx * fx * (3.0 - 2.0 * fx);
    let v = fz * fz * (3.0 - 2.0 * fz);

    let n00 = noise_2d(x0, z0);
    let n10 = noise_2d(x1, z0);
    let n01 = noise_2d(x0, z1);
    let n11 = noise_2d(x1, z1);

    let nx0 = n00 * (1.0 - u) + n10 * u;
    let nx1 = n01 * (1.0 - u) + n11 * u;

    nx0 * (1.0 - v) + nx1 * v
}

/// Multi-octave noise for terrain height.
fn terrain_height(world_x: i32, world_z: i32) -> i32 {
    let x = world_x as f32;
    let z = world_z as f32;

    // Hills: multi-octave noise for gentle rolling terrain
    let hills = smooth_noise_2d(x / 24.0, z / 24.0)
              + smooth_noise_2d(x / 12.0, z / 12.0) * 0.5
              + smooth_noise_2d(x / 6.0, z / 6.0) * 0.25;
    let hills = (hills / 1.75).max(0.0).min(1.0);

    // Mountains: large-scale features
    let mountain_raw = smooth_noise_2d(x / 88.0, z / 88.0)
                     + smooth_noise_2d(x / 44.0, z / 44.0) * 0.5;
    let mountain_raw = (mountain_raw / 1.5).max(0.0).min(1.0);
    let mountain_factor = ((mountain_raw - 0.52) / 0.48).max(0.0).min(1.0);

    // Base height: 8 + gentle hills (0-10) + mountains (0-30)
    let base_height = 8;
    let gentle_height = base_height + (hills * 10.0) as i32;
    gentle_height + (mountain_factor * 30.0) as i32
}

/// Generate a voxel chunk at the given chunk coordinates.
pub fn generate_chunk(chunk_x: i32, chunk_y: i32, chunk_z: i32) -> VoxelChunk {
    let mut chunk = VoxelChunk::new();

    // World coordinates for this chunk
    let world_x_base = chunk_x * CHUNK_SIZE as i32;
    let world_y_base = chunk_y * CHUNK_SIZE as i32;
    let world_z_base = chunk_z * CHUNK_SIZE as i32;

    // Sample terrain height at chunk center for debugging
    let center_height = terrain_height(world_x_base + 8, world_z_base + 8);
    let chunk_max_y = world_y_base + CHUNK_SIZE as i32 - 1;

    // Debug log for y=0 chunks only
    if chunk_y == 0 && (chunk_x.abs() <= 2 && chunk_z.abs() <= 2) {
        log::info!("Generating chunk ({}, {}, {}) - terrain height at center: {}, chunk y-range: {}-{}",
            chunk_x, chunk_y, chunk_z, center_height, world_y_base, chunk_max_y);
    }

    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            let world_x = world_x_base + x as i32;
            let world_z = world_z_base + z as i32;

            // Get terrain height at this x, z position
            let height = terrain_height(world_x, world_z);

            for y in 0..CHUNK_SIZE {
                let world_y = world_y_base + y as i32;

                let block = if world_y == 0 {
                    // Bedrock at the bottom
                    Block::Bedrock
                } else if world_y < height - 3 {
                    // Stone below surface with ore veins
                    if world_y <= 5 && hash(world_x, world_y, world_z) % 1000 < 3 {
                        Block::DiamondOre
                    } else if world_y <= 8 && hash(world_x, world_y, world_z) % 1000 < 6 {
                        Block::GoldOre
                    } else if world_y <= 15 && hash(world_x, world_y, world_z) % 1000 < 18 {
                        Block::IronOre
                    } else if world_y <= 20 && hash(world_x, world_y, world_z) % 1000 < 30 {
                        Block::CoalOre
                    } else if hash(world_x, world_y, world_z) % 1000 < 20 {
                        Block::Gravel
                    } else {
                        Block::Stone
                    }
                } else if world_y < height - 1 {
                    // Dirt layer (2-3 blocks below surface)
                    Block::Dirt
                } else if world_y == height - 1 {
                    // Surface layer
                    if height > 35 {
                        Block::Snow
                    } else {
                        Block::Grass
                    }
                } else if world_y < 10 {
                    // Water level at y=10
                    Block::Water
                } else {
                    // Air above surface
                    Block::Air
                };

                chunk.set(x, y, z, block);
            }
        }
    }

    // Debug log block count for y=0 chunks near spawn
    if chunk_y == 0 && (chunk_x.abs() <= 2 && chunk_z.abs() <= 2) {
        let solid_count = chunk.count_solid_blocks();
        log::info!("Chunk ({}, {}, {}) has {} solid blocks", chunk_x, chunk_y, chunk_z, solid_count);
    }

    chunk
}
