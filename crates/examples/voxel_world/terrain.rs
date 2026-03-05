//! Terrain generation — heightmap, surface blocks, biome-aware queries.

use crate::biomes::Biome;
use crate::blocks::Block;
use crate::noise::smooth_noise;

// ── World constants ─────────────────────────────────────────────────────────

pub const CHUNK_SIZE: f32       = 16.0;
pub const VOXELS_PER_CHUNK: i32 = 16;
pub const ACTIVATION_RADIUS: f32 = CHUNK_SIZE * 8.0;
pub const LOAD_RADIUS: i32      = 32;
pub const MAX_Y_CHUNKS: i32     = 3;

pub const BASE_HEIGHT: i32      = 4;
pub const GENTLE_RANGE: i32     = 10;
pub const MOUNTAIN_BONUS: i32   = 38;
pub const WATER_LEVEL: i32      = 6;
pub const STRUCTURE_HEIGHT_CUTOFF: i32 = 12;

// Forest biome noise thresholds
pub const FOREST_THRESHOLD: f32       = 0.50;
pub const FOREST_TREE_SPOT_MIN: f32   = 0.58;
pub const TREE_GRID_STEP: i32         = 5;

// Village macro-grid
pub const VILLAGE_GRID: i32           = 22;
pub const VILLAGE_PROBABILITY: f32    = 0.28;
pub const VILLAGE_CHUNK_RADIUS: i32   = 3;
pub const VILLAGE_HOUSE_MIN: usize    = 4;
pub const VILLAGE_HOUSE_MAX: usize    = 8;

// Towers & waterways
pub const TOWER_PROBABILITY: f32      = 0.022;
pub const WATERWAY_PROBABILITY: f32   = 0.015;

// ── Terrain shaping ─────────────────────────────────────────────────────────

/// Mountain influence 0..1 at world position.
pub fn mountain_factor(wx: f32, wz: f32) -> f32 {
    let raw = smooth_noise(wx / 88.0, wz / 88.0, 20)
            + smooth_noise(wx / 44.0, wz / 44.0, 21) * 0.5;
    let raw = (raw / 1.5).clamp(0.0, 1.0);
    ((raw - 0.52) / 0.48).max(0.0).clamp(0.0, 1.0)
}

/// Absolute terrain surface height at (wx, wz).
pub fn terrain_height(wx: i32, wz: i32) -> i32 {
    let (x, z) = (wx as f32, wz as f32);
    let hills = smooth_noise(x / 24.0, z / 24.0, 10)
              + smooth_noise(x / 12.0, z / 12.0, 11) * 0.5
              + smooth_noise(x /  6.0, z /  6.0, 12) * 0.25;
    let hills = (hills / 1.75).clamp(0.0, 1.0);
    let gentle_h = BASE_HEIGHT + (hills * GENTLE_RANGE as f32) as i32;
    let mf = mountain_factor(x, z);
    gentle_h + (mf * MOUNTAIN_BONUS as f32) as i32
}

/// Choose the terrain block at height `wy` given surface height and biome.
pub fn surface_block(wy: i32, surface_h: i32, biome: Biome) -> Block {
    // High-altitude bare stone / snow
    if wy > 22 && wy >= surface_h - 1 {
        return if biome == Biome::Mountains && wy > 30 { Block::Snow } else { Block::Stone };
    }

    if wy == surface_h {
        match biome {
            Biome::Plains | Biome::Forest | Biome::Jungle => Block::Grass,
            Biome::Taiga      => Block::Snow,
            Biome::Desert     => Block::Sand,
            Biome::Swamp      => Block::Clay,
            Biome::Mountains  => if wy > 25 { Block::Snow } else { Block::Stone },
        }
    } else if wy >= surface_h - 2 {
        match biome {
            Biome::Desert => Block::Sandstone,
            Biome::Swamp  => Block::Clay,
            _             => Block::Dirt,
        }
    } else {
        Block::Stone
    }
}

// ── Biome-layer noise queries ───────────────────────────────────────────────

/// Low-frequency noise defining forest blobs (0..1).
pub fn forest_noise_at(wx: f32, wz: f32) -> f32 {
    let n = smooth_noise(wx / 62.0, wz / 62.0, 30)
          + smooth_noise(wx / 31.0, wz / 31.0, 31) * 0.5;
    (n / 1.5).clamp(0.0, 1.0)
}

/// High-frequency noise controlling individual tree spots within a forest.
pub fn tree_spot_noise(wx: f32, wz: f32) -> f32 {
    smooth_noise(wx / 9.0, wz / 9.0, 40)
        + smooth_noise(wx / 4.5, wz / 4.5, 41) * 0.4
}

/// Returns true if (wx, wz) is inside a forested zone (no mountains, high forest noise).
pub fn is_forest_zone(wx: i32, wz: i32) -> bool {
    let mf = mountain_factor(wx as f32, wz as f32);
    if mf > 0.15 { return false; }
    forest_noise_at(wx as f32, wz as f32) > FOREST_THRESHOLD
}
