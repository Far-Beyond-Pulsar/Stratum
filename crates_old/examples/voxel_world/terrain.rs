//! Terrain generation — heightmap, surface blocks, biome-aware queries.

use crate::biomes::Biome;
use crate::blocks::Block;
use crate::noise::{smooth_noise, hash};

// ── World constants ─────────────────────────────────────────────────────────

pub const CHUNK_SIZE: f32       = 32.0;
pub const VOXELS_PER_CHUNK: i32 = 32;
pub const LOAD_RADIUS: i32      = 8;
/// Partition activation radius — set large enough that `stratum.tick()`'s
/// automatic sphere-activation covers all corners of the LOAD_RADIUS square
/// (diagonal = CHUNK_SIZE * LOAD_RADIUS * sqrt(2) ≈ 362 for LOAD_RADIUS=16).
/// The voxel chunk manager drives actual visibility via the dormant-cache
/// enable/disable path; this radius just prevents stratum from fighting us.
pub const ACTIVATION_RADIUS: f32 = CHUNK_SIZE * (LOAD_RADIUS as f32 * 1.5);
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
/// Includes bedrock, ore veins, beach zones, and biome-specific surfaces.
pub fn surface_block(wy: i32, surface_h: i32, biome: Biome, wx: i32, wz: i32) -> Block {
    // ── Bedrock layer (y=0 always, y=1-2 mixed) ────────────────────────────
    if wy == 0 {
        return Block::Bedrock;
    }
    if wy <= 2 {
        let h = hash(wx, wz, 200 + wy as u64);
        if h < 0.5 - (wy as f32 * 0.15) {
            return Block::Bedrock;
        }
    }

    // ── High-altitude bare stone / snow ─────────────────────────────────────
    if wy > 22 && wy >= surface_h - 1 {
        return if biome == Biome::Mountains && wy > 30 { Block::Snow } else { Block::Stone };
    }

    // ── Beach zone: sand near water level ───────────────────────────────────
    let is_beach = is_beach_zone(wx, wz, surface_h, biome);

    if wy == surface_h {
        if is_beach {
            return Block::Sand;
        }
        match biome {
            Biome::Plains | Biome::Forest => Block::Grass,
            Biome::Jungle => Block::Grass,
            Biome::Taiga  => Block::Podzol,
            Biome::Desert => Block::Sand,
            Biome::Swamp  => Block::Clay,
            Biome::Mountains => if wy > 25 { Block::Snow } else { Block::Stone },
        }
    } else if wy >= surface_h - 2 {
        // ── Subsurface (2 blocks below surface) ────────────────────────────
        if is_beach {
            return if wy == surface_h - 1 { Block::Sand } else { Block::Sandstone };
        }
        match biome {
            Biome::Desert => Block::Sandstone,
            Biome::Swamp  => Block::Clay,
            _             => Block::Dirt,
        }
    } else {
        // ── Underground: stone with ore veins ──────────────────────────────
        ore_or_stone(wx, wy, wz)
    }
}

// ── Beach detection ─────────────────────────────────────────────────────────

/// True if this column should be a sand beach (near water, low elevation, not desert/taiga).
pub fn is_beach_zone(wx: i32, wz: i32, surface_h: i32, biome: Biome) -> bool {
    // Desert is already sand, taiga has snow shores, mountains are rocky
    if matches!(biome, Biome::Desert | Biome::Taiga | Biome::Mountains) {
        return false;
    }
    // Beach: surface within 2 blocks of water level
    if surface_h > WATER_LEVEL + 2 || surface_h < WATER_LEVEL - 1 {
        return false;
    }
    // Check if there's water nearby (any adjacent column below water level)
    for &(dx, dz) in &[(2, 0), (-2, 0), (0, 2), (0, -2), (3, 0), (-3, 0), (0, 3), (0, -3)] {
        let nh = terrain_height(wx + dx, wz + dz);
        if nh < WATER_LEVEL {
            return true;
        }
    }
    false
}

// ── Ore generation ──────────────────────────────────────────────────────────

/// Returns an ore block or stone based on position-seeded noise.
/// Mimics Minecraft ore distribution at compressed world heights.
fn ore_or_stone(wx: i32, wy: i32, wz: i32) -> Block {
    let h = hash(wx.wrapping_mul(13) ^ wy.wrapping_mul(7), wz.wrapping_mul(19) ^ wy, 300);

    // Gravel veins (any depth, ~2% chance)
    if h < 0.020 {
        return Block::Gravel;
    }

    // Coal ore (y=0-20, ~3% chance)
    if wy <= 20 && h < 0.050 {
        return Block::CoalOre;
    }

    // Iron ore (y=0-15, ~1.8% chance)
    if wy <= 15 && h < 0.068 {
        return Block::IronOre;
    }

    // Gold ore (y=0-8, ~0.6% chance)
    if wy <= 8 && h < 0.074 {
        return Block::GoldOre;
    }

    // Diamond ore (y=0-5, ~0.3% chance)
    if wy <= 5 && h < 0.077 {
        return Block::DiamondOre;
    }

    // Obsidian (y=0-3, very rare ~0.1%)
    if wy <= 3 && h < 0.078 {
        return Block::Obsidian;
    }

    Block::Stone
}

// ── Surface decoration queries ──────────────────────────────────────────────

/// Returns a decoration block to place on top of the surface, or None.
pub fn surface_decoration(wx: i32, wz: i32, surface_h: i32, biome: Biome) -> Option<Block> {
    if is_beach_zone(wx, wz, surface_h, biome) { return None; }
    if surface_h < WATER_LEVEL { return None; }
    if biome == Biome::Mountains && surface_h > 25 { return None; }

    let h = hash(wx, wz, 400);
    let h2 = hash(wx, wz, 401);

    match biome {
        Biome::Plains => {
            if h < 0.015 { return Some(Block::Poppy); }
            if h < 0.030 { return Some(Block::Dandelion); }
            if h < 0.005 && h2 > 0.5 { return Some(Block::Pumpkin); }
            None
        }
        Biome::Forest => {
            if h < 0.020 { return Some(Block::Poppy); }
            if h < 0.035 { return Some(Block::Dandelion); }
            if h < 0.050 && h2 < 0.5 { return Some(Block::BrownMushroom); }
            if h < 0.055 { return Some(Block::RedMushroom); }
            None
        }
        Biome::Jungle => {
            if h < 0.012 { return Some(Block::Melon); }
            if h < 0.025 { return Some(Block::BrownMushroom); }
            None
        }
        Biome::Swamp => {
            if h < 0.030 { return Some(Block::BrownMushroom); }
            if h < 0.045 { return Some(Block::RedMushroom); }
            None
        }
        Biome::Desert => {
            if h < 0.035 { return Some(Block::DeadBush); }
            None
        }
        Biome::Taiga => {
            if h < 0.020 { return Some(Block::BrownMushroom); }
            if h < 0.030 { return Some(Block::RedMushroom); }
            None
        }
        Biome::Mountains => None,
    }
}

/// Returns a water-surface decoration (lily pad), or None.
pub fn water_decoration(wx: i32, wz: i32, biome: Biome) -> Option<Block> {
    if biome != Biome::Swamp { return None; }
    let h = hash(wx, wz, 410);
    if h < 0.08 { Some(Block::LilyPad) } else { None }
}

/// Check if sugar cane should be placed at this position.
pub fn sugar_cane_spot(wx: i32, wz: i32, surface_h: i32, biome: Biome) -> bool {
    if matches!(biome, Biome::Desert | Biome::Mountains | Biome::Taiga) {
        return false;
    }
    if surface_h != WATER_LEVEL && surface_h != WATER_LEVEL + 1 {
        return false;
    }
    let has_water = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|&(dx, dz)| {
        terrain_height(wx + dx, wz + dz) < WATER_LEVEL
    });
    if !has_water { return false; }

    let h = hash(wx, wz, 420);
    h < 0.06
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
