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

    // Multiple octaves of noise
    let mut height = 0.0;
    height += smooth_noise_2d(x * 0.01, z * 0.01) * 20.0; // Large features
    height += smooth_noise_2d(x * 0.05, z * 0.05) * 8.0;  // Medium features
    height += smooth_noise_2d(x * 0.1, z * 0.1) * 3.0;    // Small details

    // Base height at y=32
    let base_height = 32;
    (base_height as f32 + height) as i32
}

/// Generate a voxel chunk at the given chunk coordinates.
pub fn generate_chunk(chunk_x: i32, chunk_y: i32, chunk_z: i32) -> VoxelChunk {
    let mut chunk = VoxelChunk::new();

    // World coordinates for this chunk
    let world_x_base = chunk_x * CHUNK_SIZE as i32;
    let world_y_base = chunk_y * CHUNK_SIZE as i32;
    let world_z_base = chunk_z * CHUNK_SIZE as i32;

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
                    // Stone below surface
                    if world_y < 20 && hash(world_x, world_y, world_z) % 100 < 2 {
                        // Rare diamond ore
                        Block::DiamondOre
                    } else if world_y < 30 && hash(world_x, world_y, world_z) % 100 < 5 {
                        // Gold ore
                        Block::GoldOre
                    } else if world_y < 40 && hash(world_x, world_y, world_z) % 100 < 10 {
                        // Iron ore
                        Block::IronOre
                    } else if hash(world_x, world_y, world_z) % 100 < 15 {
                        // Coal ore
                        Block::CoalOre
                    } else {
                        Block::Stone
                    }
                } else if world_y < height - 1 {
                    // Dirt layer
                    Block::Dirt
                } else if world_y == height - 1 {
                    // Top layer - grass or snow depending on height
                    if height > 50 {
                        Block::Snow
                    } else {
                        Block::Grass
                    }
                } else if world_y < 32 {
                    // Water below sea level
                    Block::Water
                } else {
                    // Air above surface
                    Block::Air
                };

                chunk.set(x, y, z, block);
            }
        }
    }

    chunk
}
