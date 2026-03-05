//! `voxel_world` — Infinite procedurally-generated Minecraft-style voxel world.
//!
//! Chunks are generated lazily as the camera moves.  Each Stratum world-
//! partition chunk maps 1:1 to one on-disk file under `levels/voxel_world/`.
//!
//! ## Mesh strategy
//! A single `GpuMesh` is built per material per chunk.
//! Only exposed faces are emitted — interior faces between two solid blocks are
//! culled. This keeps draw-call count proportional to material count per chunk.
//!
//! ## Features
//! * Procedural terrain (grass / dirt / stone)
//! * Oak trees — solid log trunk + hollow leaf canopy
//! * Villages — clusters of stone-brick cottages with plank roofs and glass windows
//!
//! ## Controls
//! WASD fly | Space/Shift up/down | Mouse drag look (click to grab) | Tab mode | Esc exit

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use glam::Vec3;
use winit::{
    application::ApplicationHandler,
    event::*,
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowId},
};

use helio_render_v2::{
    GpuMesh, PackedVertex, Renderer, RendererConfig,
    features::{BloomFeature, FeatureRegistry, LightingFeature, ShadowsFeature},
};

use stratum::{
    chunk_on_disk,
    Aabb, CameraId, CameraKind, ChunkCoord, Components, EntityId, LightData,
    MaterialHandle, PlacementContext, Prefab, PrefabVolume, Projection,
    RenderTargetHandle, SimulationMode, Stratum, StratumCamera, Transform,
    Viewport, Level, StreamEvent, LevelStreamer, MeshHandle,
    level_fs::format::{ChunkFile, EntityRecord, TransformRecord, FORMAT_VERSION},
};
use stratum_helio::{AssetRegistry, HelioIntegration, Material};

// ── Level directory ───────────────────────────────────────────────────────────

fn level_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("levels")
        .join("voxel_world")
}

/// Bump this key to wipe stale on-disk chunks after format changes.
const CACHE_KEY: &str = "voxel_world_v6_mountains_forests";

// ── World constants ───────────────────────────────────────────────────────────

/// World-space metres per chunk edge.  1 voxel = 1 m so this equals VOXELS_PER_CHUNK.
const CHUNK_SIZE: f32 = 16.0;
const VOXELS_PER_CHUNK: i32 = 16;
const ACTIVATION_RADIUS: f32 = CHUNK_SIZE * 8.0;
/// Half-side of the square load area in chunk coords.
const LOAD_RADIUS: i32 = 5;
/// How many Y-chunk layers to load (covers 0..MAX_Y_CHUNKS*16 metres).
const MAX_Y_CHUNKS: i32 = 3; // 0..47 m covers mountains

// Terrain shaping
const BASE_HEIGHT: i32 = 4;
const GENTLE_RANGE: i32 = 10;  // gentle hills add up to 10 m above base
const MOUNTAIN_BONUS: i32 = 38; // mountains can add up to 38 m on top of gentle hills

/// Surface height above which no trees or structures spawn (stone-only zone).
const STRUCTURE_HEIGHT_CUTOFF: i32 = 12;

// Forest biome
/// Low-freq noise threshold; above this = forest zone.
const FOREST_THRESHOLD: f32 = 0.50;
/// Within the forest zone, individual spot noise must exceed this to plant a tree.
const FOREST_TREE_SPOT_MIN: f32 = 0.58;
/// Minimum world-block separation between tree grid sample points.
const TREE_GRID_STEP: i32 = 5;

// Village macro-grid
/// Side length in chunk-coordinates of one village macro-cell.
const VILLAGE_GRID: i32 = 22;
/// Fraction of macro-cells that contain a village.
const VILLAGE_PROBABILITY: f32 = 0.28;
/// Village radius in chunk-coordinates (houses cluster within this from center).
const VILLAGE_CHUNK_RADIUS: i32 = 3;
/// Min/max house count per village.
const VILLAGE_HOUSE_MIN: usize = 4;
const VILLAGE_HOUSE_MAX: usize = 8;

// Ruined towers
const TOWER_PROBABILITY: f32 = 0.022; // ~2% of chunks may have a tower

const CAM_SPEED: f32 = 28.0;
const LOOK_SENS: f32 = 0.002;

// ── Block type ────────────────────────────────────────────────────────────────

/// Block discriminant stored in chunk JSON as `material` field index.
///   0 = air (not stored)
///   1 = grass   2 = dirt    3 = stone
///   4 = wood    5 = leaves  6 = stone_brick  7 = plank  8 = glass
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Block { Grass, Dirt, Stone, Wood, Leaves, StoneBrick, Plank, Glass }

impl Block {
    fn mat_index(self) -> u64 {
        match self {
            Block::Grass      => 1,
            Block::Dirt       => 2,
            Block::Stone      => 3,
            Block::Wood       => 4,
            Block::Leaves     => 5,
            Block::StoneBrick => 6,
            Block::Plank      => 7,
            Block::Glass      => 8,
        }
    }
    fn from_mat_index(n: u64) -> Option<Self> {
        match n {
            1 => Some(Block::Grass),
            2 => Some(Block::Dirt),
            3 => Some(Block::Stone),
            4 => Some(Block::Wood),
            5 => Some(Block::Leaves),
            6 => Some(Block::StoneBrick),
            7 => Some(Block::Plank),
            8 => Some(Block::Glass),
            _ => None,
        }
    }
}

// ── Heightmap (no external dep) ───────────────────────────────────────────────

fn hash(x: i32, z: i32, seed: u64) -> f32 {
    let mut v = (x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
              ^ (z as u64).wrapping_mul(0x6c62_272e_07bb_0142)
              ^ seed;
    v ^= v >> 30; v = v.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    v ^= v >> 27; v = v.wrapping_mul(0x94d0_49bb_1331_11eb);
    v ^= v >> 31;
    (v as f32) / (u64::MAX as f32)
}

fn smooth_noise(x: f32, z: f32, seed: u64) -> f32 {
    let xi = x.floor() as i32; let zi = z.floor() as i32;
    let ux = { let f = x - xi as f32; f * f * (3.0 - 2.0 * f) };
    let uz = { let f = z - zi as f32; f * f * (3.0 - 2.0 * f) };
    let a = hash(xi, zi, seed);     let b = hash(xi+1, zi,   seed);
    let c = hash(xi, zi+1, seed);   let d = hash(xi+1, zi+1, seed);
    (a + ux*(b-a)) + uz * ((c + ux*(d-c)) - (a + ux*(b-a)))
}

// ── Terrain ───────────────────────────────────────────────────────────────────

/// Mountain influence 0..1 at world position (wx, wz).
/// Returns > 0 only in mountain zones; peaks at 1.0.
fn mountain_factor(wx: f32, wz: f32) -> f32 {
    let raw = smooth_noise(wx / 88.0, wz / 88.0, 20)
            + smooth_noise(wx / 44.0, wz / 44.0, 21) * 0.5;
    let raw = (raw / 1.5).clamp(0.0, 1.0);
    // Smooth threshold — only the top portion becomes mountain.
    ((raw - 0.52) / 0.48).max(0.0).clamp(0.0, 1.0)
}

fn terrain_height(wx: i32, wz: i32) -> i32 {
    let (x, z) = (wx as f32, wz as f32);

    // Gentle rolling hills (2-3 octaves)
    let hills = smooth_noise(x / 24.0, z / 24.0, 10)
              + smooth_noise(x / 12.0, z / 12.0, 11) * 0.5
              + smooth_noise(x /  6.0, z /  6.0, 12) * 0.25;
    let hills = (hills / 1.75).clamp(0.0, 1.0);
    let gentle_h = BASE_HEIGHT + (hills * GENTLE_RANGE as f32) as i32;

    // Mountain boost
    let mf = mountain_factor(x, z);
    gentle_h + (mf * MOUNTAIN_BONUS as f32) as i32
}

fn block_type(wy: i32, surface_h: i32) -> Block {
    // High-altitude bare stone (mountainside)
    if wy > 22 && wy >= surface_h - 1 { return Block::Stone; }
    if wy == surface_h      { Block::Grass }
    else if wy >= surface_h - 2 { Block::Dirt }
    else                    { Block::Stone }
}

// ── Biome queries ─────────────────────────────────────────────────────────────

/// Low-frequency noise that defines forest blobs.  Returns 0..1.
/// Values > FOREST_THRESHOLD → forest zone.
fn forest_noise_at(wx: f32, wz: f32) -> f32 {
    let n = smooth_noise(wx / 62.0, wz / 62.0, 30)
          + smooth_noise(wx / 31.0, wz / 31.0, 31) * 0.5;
    (n / 1.5).clamp(0.0, 1.0)
}

/// High-frequency noise that controls individual tree spots within a forest.
fn tree_spot_noise(wx: f32, wz: f32) -> f32 {
    smooth_noise(wx / 9.0, wz / 9.0, 40)
        + smooth_noise(wx / 4.5, wz / 4.5, 41) * 0.4
}

/// Returns true if (wx, wz) is inside a forest zone (no mountains, high forest noise).
fn is_forest_zone(wx: i32, wz: i32) -> bool {
    let mf = mountain_factor(wx as f32, wz as f32);
    if mf > 0.15 { return false; } // mountains suppress forests
    forest_noise_at(wx as f32, wz as f32) > FOREST_THRESHOLD
}

// ── Village macro-grid ────────────────────────────────────────────────────────

/// Returns the chunk-coordinate of the village center inside macro-cell (cx, cz),
/// or `None` if this cell has no village.
fn village_center_for_cell(cell_x: i32, cell_z: i32) -> Option<(i32, i32)> {
    if hash(cell_x, cell_z, 77) > VILLAGE_PROBABILITY { return None; }
    let margin = 3.0;
    let range  = (VILLAGE_GRID as f32) - margin * 2.0;
    let fx = margin + hash(cell_x, cell_z, 78) * range;
    let fz = margin + hash(cell_x, cell_z, 79) * range;
    Some((cell_x * VILLAGE_GRID + fx as i32,
          cell_z * VILLAGE_GRID + fz as i32))
}

/// If chunk `(chunk_x, chunk_z)` is within any village's radius, return that
/// village's center chunk-coord.
fn village_center_near(chunk_x: i32, chunk_z: i32) -> Option<(i32, i32)> {
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

// ── Prefab definitions ────────────────────────────────────────────────────────

fn voxel_entity(lx: f32, ly: f32, lz: f32, block: Block) -> Components {
    Components::new()
        .with_transform(Transform::from_position(Vec3::new(lx + 0.5, ly + 0.5, lz + 0.5)))
        .with_material(stratum::MaterialHandle(block.mat_index()))
        .with_bounding_radius(0.866)
}

/// Standard oak tree: 5-block trunk + rounded 5×4×5 canopy.
fn make_tree_prefab() -> Prefab {
    let mut b = Prefab::builder("tree_oak");
    for y in 0..5_i32 {
        b = b.with_entity(voxel_entity(-0.5, y as f32, -0.5, Block::Wood));
    }
    // Canopy — 4 layers, widest in the middle
    for dy in 0..4_i32 {
        let r: i32 = match dy { 1 | 2 => 2, _ => 1 };
        for dx in -r..=r {
            for dz in -r..=r {
                if r == 2 && dx.abs() == 2 && dz.abs() == 2 && (dy == 0 || dy == 3) { continue; }
                b = b.with_entity(voxel_entity(dx as f32 - 0.5, (dy + 3) as f32, dz as f32 - 0.5, Block::Leaves));
            }
        }
    }
    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::new(-0.5, 0.0, -0.5), Vec3::new(0.5, 5.0, 0.5),
    )));
    b = b.with_volume(PrefabVolume::hollow(Aabb::new(
        Vec3::new(-2.5, 3.0, -2.5), Vec3::new(2.5, 7.0, 2.5),
    )));
    b.build()
}

/// Taller pine-style tree: 7-block trunk, narrow 3×4×3 pointed canopy.
fn make_tall_tree_prefab() -> Prefab {
    let mut b = Prefab::builder("tree_pine");
    for y in 0..7_i32 {
        b = b.with_entity(voxel_entity(-0.5, y as f32, -0.5, Block::Wood));
    }
    // Pointed canopy — narrow at top, wider at base
    for dy in 0..4_i32 {
        let r: i32 = 2 - (dy / 2);
        for dx in -r..=r {
            for dz in -r..=r {
                b = b.with_entity(voxel_entity(dx as f32 - 0.5, (dy + 4) as f32, dz as f32 - 0.5, Block::Leaves));
            }
        }
    }
    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::new(-0.5, 0.0, -0.5), Vec3::new(0.5, 7.0, 0.5),
    )));
    b = b.with_volume(PrefabVolume::hollow(Aabb::new(
        Vec3::new(-2.5, 4.0, -2.5), Vec3::new(2.5, 8.0, 2.5),
    )));
    b.build()
}

/// Cottage: 7×7 footprint, 4 blocks of wall, stone-brick + plank roof + glass windows.
fn make_house_prefab() -> Prefab {
    let mut b = Prefab::builder("house_cottage");
    let w = 7_i32;
    let wall_h = 4_i32;

    for x in 0..w {
        for z in 0..w {
            b = b.with_entity(voxel_entity(x as f32, 0.0, z as f32, Block::Plank));
            let on_edge = x == 0 || x == w-1 || z == 0 || z == w-1;
            if on_edge {
                for y in 1..wall_h {
                    let is_window = y == 2
                        && ((x == 0 || x == w-1) && z > 1 && z < w-2
                            || (z == 0 || z == w-1) && x > 1 && x < w-2);
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
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(w as f32, (wall_h + 1) as f32, w as f32),
    )));
    b = b.with_volume(PrefabVolume::hollow(Aabb::new(
        Vec3::new(1.0, 1.0, 1.0),
        Vec3::new((w-1) as f32, wall_h as f32, (w-1) as f32),
    )));
    b.build()
}

/// Larger house variant: 9×9 footprint, adds a chimney.
fn make_large_house_prefab() -> Prefab {
    let mut b = Prefab::builder("house_large");
    let w = 9_i32;
    let wall_h = 5_i32;

    for x in 0..w {
        for z in 0..w {
            b = b.with_entity(voxel_entity(x as f32, 0.0, z as f32, Block::Plank));
            let on_edge = x == 0 || x == w-1 || z == 0 || z == w-1;
            if on_edge {
                for y in 1..wall_h {
                    let is_window = y == 2
                        && ((x == 0 || x == w-1) && z > 1 && z < w-2
                            || (z == 0 || z == w-1) && x > 1 && x < w-2);
                    b = b.with_entity(voxel_entity(
                        x as f32, y as f32, z as f32,
                        if is_window { Block::Glass } else { Block::StoneBrick },
                    ));
                }
            }
            b = b.with_entity(voxel_entity(x as f32, wall_h as f32, z as f32, Block::Plank));
        }
    }
    // Chimney: 1×3 tall stone-brick column in back-right corner
    for y in 1..wall_h+3 {
        b = b.with_entity(voxel_entity((w-2) as f32, y as f32, (w-2) as f32, Block::StoneBrick));
    }
    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(w as f32, (wall_h + 1) as f32, w as f32),
    )));
    b = b.with_volume(PrefabVolume::hollow(Aabb::new(
        Vec3::new(1.0, 1.0, 1.0),
        Vec3::new((w-1) as f32, wall_h as f32, (w-1) as f32),
    )));
    b.build()
}

/// Village well: 3×3 stone-brick rim, hollow centre.
fn make_well_prefab() -> Prefab {
    let mut b = Prefab::builder("well");
    for x in 0..3_i32 {
        for z in 0..3_i32 {
            if x == 1 && z == 1 {
                // hollow centre — just put a stone floor 1 below ground
                b = b.with_entity(voxel_entity(x as f32, -1.0, z as f32, Block::StoneBrick));
            } else {
                b = b.with_entity(voxel_entity(x as f32, 0.0, z as f32, Block::StoneBrick));
                b = b.with_entity(voxel_entity(x as f32, 1.0, z as f32, Block::StoneBrick));
            }
        }
    }
    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(3.0, 2.0, 3.0),
    )));
    b.build()
}

/// Ruined tower: 3×3 base, 8 blocks tall, blocks randomly missing (more gaps higher up).
fn make_ruined_tower_prefab() -> Prefab {
    let mut b = Prefab::builder("ruined_tower");
    let size   = 3_i32;
    let height = 8_i32;

    for y in 0..height {
        for x in 0..size {
            for z in 0..size {
                let is_wall = x == 0 || x == size-1 || z == 0 || z == size-1;
                let is_base = y == 0;
                if !is_wall && !is_base { continue; }

                // Ruin decay: probability of a block being absent increases with height
                let decay_prob = ((y as f32 - 1.0) / height as f32 * 0.7).max(0.0);
                if y > 1 && hash(x * 31 + y * 7, z * 13 + y * 11, 88) < decay_prob { continue; }

                b = b.with_entity(voxel_entity(x as f32, y as f32, z as f32, Block::StoneBrick));
            }
        }
    }
    b = b.with_volume(PrefabVolume::solid(Aabb::new(
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(size as f32, height as f32, size as f32),
    )));
    b.build()
}

// ── Placement helpers ─────────────────────────────────────────────────────────

/// Append world-space `EntityRecord`s for all entities in `prefab` placed at `world_pos`.
fn emit_prefab_entities(
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

// ── Structure placement ───────────────────────────────────────────────────────

/// Place trees across a chunk using a regular grid + biome noise filter.
///
/// Trees only appear in forest zones (forest_noise > FOREST_THRESHOLD) and
/// where individual spot noise exceeds a density-dependent threshold — creating
/// dense interiors and sparse forest edges, with wide open plains in between.
fn place_trees_in_chunk(
    ox: i32, oz: i32,
    entities: &mut Vec<EntityRecord>, eid: &mut u64,
    tree: &Prefab, tall_tree: &Prefab,
    ctx: &mut PlacementContext,
) {
    use stratum::{Level, LevelId};
    let mut scratch = Level::new(LevelId::new(0), "scratch", 32.0, 10000.0);

    let mut gx = ox;
    while gx < ox + VOXELS_PER_CHUNK {
        let mut gz = oz;
        while gz < oz + VOXELS_PER_CHUNK {
            let forest = forest_noise_at(gx as f32, gz as f32);
            if forest > FOREST_THRESHOLD {
                let spot = tree_spot_noise(gx as f32, gz as f32);
                // In denser forest zones the threshold is lower → more trees.
                let threshold = FOREST_TREE_SPOT_MIN - (forest - FOREST_THRESHOLD) * 0.35;
                if spot > threshold {
                    let h = terrain_height(gx, gz);
                    if h >= BASE_HEIGHT && h <= STRUCTURE_HEIGHT_CUTOFF {
                        let world_pos = Vec3::new(gx as f32, h as f32 + 1.0, gz as f32);
                        // Pine trees appear occasionally for variety
                        let use_pine = hash(gx, gz, 55) < 0.25;
                        let chosen   = if use_pine { tall_tree } else { tree };
                        if ctx.place(chosen, world_pos, &mut scratch).is_ok() {
                            emit_prefab_entities(chosen, world_pos, entities, eid);
                        }
                    }
                }
            }
            gz += TREE_GRID_STEP;
        }
        gx += TREE_GRID_STEP;
    }
}

/// Place a ruined tower at a deterministic position within the chunk, if conditions allow.
fn place_tower_in_chunk(
    ox: i32, oz: i32,
    chunk_x: i32, chunk_z: i32,
    entities: &mut Vec<EntityRecord>, eid: &mut u64,
    tower: &Prefab,
    ctx: &mut PlacementContext,
) {
    use stratum::{Level, LevelId};
    let mut scratch = Level::new(LevelId::new(0), "scratch", 32.0, 10000.0);

    // Don't put towers in forests or near villages
    if is_forest_zone(ox + VOXELS_PER_CHUNK / 2, oz + VOXELS_PER_CHUNK / 2) { return; }
    if village_center_near(chunk_x, chunk_z).is_some() { return; }

    let fx = 3.0 + hash(chunk_x, chunk_z, 81) * (VOXELS_PER_CHUNK as f32 - 6.0);
    let fz = 3.0 + hash(chunk_x, chunk_z, 82) * (VOXELS_PER_CHUNK as f32 - 6.0);
    let wx = ox + fx as i32;
    let wz = oz + fz as i32;
    let h  = terrain_height(wx, wz);
    if h < BASE_HEIGHT + 1 || h > STRUCTURE_HEIGHT_CUTOFF { return; }

    let world_pos = Vec3::new(wx as f32, h as f32 + 1.0, wz as f32);
    if ctx.place(tower, world_pos, &mut scratch).is_ok() {
        emit_prefab_entities(tower, world_pos, entities, eid);
    }
}

/// Place village structures for a chunk that is within a village's radius.
///
/// The full set of house positions is deterministically computed from the village
/// center; only those whose base falls inside this chunk's XZ bounds are emitted.
/// The well is placed by the chunk that contains the village center.
fn place_village_structures(
    chunk_coord:   ChunkCoord,
    village_center: (i32, i32),
    entities:      &mut Vec<EntityRecord>,
    eid:           &mut u64,
    house:         &Prefab,
    large_house:   &Prefab,
    well:          &Prefab,
    ctx:           &mut PlacementContext,
) {
    use stratum::{Level, LevelId};
    let mut scratch = Level::new(LevelId::new(0), "scratch", 32.0, 10000.0);

    let (vcx, vcz) = village_center;
    // Village world-space center (middle of the center chunk)
    let vcwx = vcx * VOXELS_PER_CHUNK + VOXELS_PER_CHUNK / 2;
    let vcwz = vcz * VOXELS_PER_CHUNK + VOXELS_PER_CHUNK / 2;

    // Reject village placement on mountainous or very uneven terrain
    let center_h = terrain_height(vcwx, vcwz);
    if center_h > STRUCTURE_HEIGHT_CUTOFF { return; }

    let ox = chunk_coord.x * VOXELS_PER_CHUNK;
    let oz = chunk_coord.z * VOXELS_PER_CHUNK;

    // Well — placed only from the center chunk
    if chunk_coord.x == vcx && chunk_coord.z == vcz {
        let wp = Vec3::new(vcwx as f32 - 1.0, center_h as f32 + 1.0, vcwz as f32 - 1.0);
        if ctx.place(well, wp, &mut scratch).is_ok() {
            emit_prefab_entities(well, wp, entities, eid);
        }
    }

    // Houses — deterministic set from village center, emitted per-chunk
    let house_count = VILLAGE_HOUSE_MIN
        + (hash(vcx, vcz, 55) * (VILLAGE_HOUSE_MAX - VILLAGE_HOUSE_MIN) as f32) as usize;

    for i in 0..house_count {
        let angle  = hash(vcx + i as i32, vcz,          60 + i as u64) * std::f32::consts::TAU;
        let radius = 8.0 + hash(vcx, vcz + i as i32,    70 + i as u64) * 22.0;
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

        // Alternate between house sizes based on index
        let chosen = if i % 5 == 0 { large_house } else { house };
        if ctx.place(chosen, world_pos, &mut scratch).is_ok() {
            emit_prefab_entities(chosen, world_pos, entities, eid);
        }
    }
}

// ── Chunk data generation ─────────────────────────────────────────────────────
//
// The ChunkFile stores one EntityRecord per voxel. Each record contains:
//   - transform.position  → world-space voxel centre
//   - material            → Block discriminant index
// Mesh handles are NOT stored — they are built at load time from geometry.
//
// Multi-Y support: terrain heights can exceed 16 m (mountains), so chunks at
// coord.y = 1 and coord.y = 2 also receive terrain blocks.
// Structures (trees, buildings) are only emitted from coord.y == 0 chunks,
// since their bases are always on ground level (≤ STRUCTURE_HEIGHT_CUTOFF).

fn build_chunk_file(coord: ChunkCoord) -> ChunkFile {
    let empty = || ChunkFile {
        version:          FORMAT_VERSION,
        coord:            [coord.x, coord.y, coord.z],
        entities:         vec![],
        prefab_instances: vec![],
    };

    if coord.y < 0 || coord.y >= MAX_Y_CHUNKS { return empty(); }

    let wy_min = coord.y * VOXELS_PER_CHUNK;      // inclusive
    let wy_max = wy_min + VOXELS_PER_CHUNK;        // exclusive
    let ox     = coord.x * VOXELS_PER_CHUNK;
    let oz     = coord.z * VOXELS_PER_CHUNK;

    let mut entities = Vec::new();
    let mut eid: u64 = 0;

    // ── Terrain voxels ──────────────────────────────────────────────────────
    for lx in 0..VOXELS_PER_CHUNK {
        for lz in 0..VOXELS_PER_CHUNK {
            let wx = ox + lx;
            let wz = oz + lz;
            let surface_h = terrain_height(wx, wz);

            // Emit only the blocks that land in this chunk's Y range
            let y_lo = wy_min;
            let y_hi = wy_max.min(surface_h + 1); // exclusive upper bound
            for wy in y_lo..y_hi {
                let block = block_type(wy, surface_h);
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

    // ── Structures (only placed from y=0 chunks) ─────────────────────────────
    if coord.y == 0 {
        let tree       = make_tree_prefab();
        let tall_tree  = make_tall_tree_prefab();
        let house      = make_house_prefab();
        let large_house = make_large_house_prefab();
        let well       = make_well_prefab();
        let tower      = make_ruined_tower_prefab();
        let mut ctx    = PlacementContext::new();

        if let Some(vc) = village_center_near(coord.x, coord.z) {
            place_village_structures(
                coord, vc,
                &mut entities, &mut eid,
                &house, &large_house, &well,
                &mut ctx,
            );
        } else if hash(coord.x, coord.z, 88) < TOWER_PROBABILITY {
            place_tower_in_chunk(
                ox, oz, coord.x, coord.z,
                &mut entities, &mut eid,
                &tower, &mut ctx,
            );
        } else {
            place_trees_in_chunk(
                ox, oz,
                &mut entities, &mut eid,
                &tree, &tall_tree,
                &mut ctx,
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

// ── Chunk mesh builder ────────────────────────────────────────────────────────
//
// Given a voxel set (world-space integer positions), build face-culled geometry.
// Only faces with NO solid neighbour in that direction are emitted.
// Returns one Vec<(vertices, indices)> per distinct Block type.

fn build_chunk_mesh(
    device:          &wgpu::Device,
    chunk:           ChunkFile,
    mat_grass:       MaterialHandle,
    mat_dirt:        MaterialHandle,
    mat_stone:       MaterialHandle,
    mat_wood:        MaterialHandle,
    mat_leaves:      MaterialHandle,
    mat_stone_brick: MaterialHandle,
    mat_plank:       MaterialHandle,
    mat_glass:       MaterialHandle,
) -> Vec<(GpuMesh, MaterialHandle)> {
    // Collect solid voxels keyed by their integer min-corner (floor of centre).
    let mut solid: HashMap<(i32,i32,i32), Block> = HashMap::new();
    for rec in &chunk.entities {
        if let (Some(t), Some(m)) = (&rec.transform, rec.material) {
            if let Some(block) = Block::from_mat_index(m) {
                solid.insert((
                    t.position[0].floor() as i32,
                    t.position[1].floor() as i32,
                    t.position[2].floor() as i32,
                ), block);
            }
        }
    }
    if solid.is_empty() { return vec![]; }

    // Face table derived directly from GpuMesh::cube (same winding / UVs).
    // Each entry: (normal, neighbour-delta, [4 corner offsets from voxel min])
    // Winding: CCW when viewed from outside (matches the renderer's convention).
    #[rustfmt::skip]
    const FACES: &[([f32;3], [i32;3], [[f32;3];4])] = &[
        // +Z
        ([0.,0., 1.], [0,0, 1], [[0.,0.,1.],[1.,0.,1.],[1.,1.,1.],[0.,1.,1.]]),
        // -Z
        ([0.,0.,-1.], [0,0,-1], [[1.,0.,0.],[0.,0.,0.],[0.,1.,0.],[1.,1.,0.]]),
        // +X
        ([ 1.,0.,0.], [ 1,0,0], [[1.,0.,1.],[1.,0.,0.],[1.,1.,0.],[1.,1.,1.]]),
        // -X
        ([-1.,0.,0.], [-1,0,0], [[0.,0.,0.],[0.,0.,1.],[0.,1.,1.],[0.,1.,0.]]),
        // +Y
        ([0., 1.,0.], [0, 1,0], [[0.,1.,1.],[1.,1.,1.],[1.,1.,0.],[0.,1.,0.]]),
        // -Y
        ([0.,-1.,0.], [0,-1,0], [[0.,0.,0.],[1.,0.,0.],[1.,0.,1.],[0.,0.,1.]]),
    ];
    const UVS: [[f32;2]; 4] = [[0.,0.],[1.,0.],[1.,1.],[0.,1.]];

    let mut verts: HashMap<Block, Vec<PackedVertex>> = HashMap::new();
    let mut idxs:  HashMap<Block, Vec<u32>>          = HashMap::new();

    for (&(bx,by,bz), &block) in &solid {
        let v  = verts.entry(block).or_default();
        let ix = idxs.entry(block).or_default();

        for (normal, nb, corners) in FACES {
            if solid.contains_key(&(bx+nb[0], by+nb[1], bz+nb[2])) { continue; }

            let base_vert = v.len() as u32;
            // Tangent = direction from corner[0] to corner[1] (matches GpuMesh::cube).
            let c0 = corners[0]; let c1 = corners[1];
            let td = [c1[0]-c0[0], c1[1]-c0[1], c1[2]-c0[2]];
            let tl = (td[0]*td[0] + td[1]*td[1] + td[2]*td[2]).sqrt().max(1e-8);
            let tangent = [td[0]/tl, td[1]/tl, td[2]/tl];

            for (ci, corner) in corners.iter().enumerate() {
                v.push(PackedVertex::new_with_tangent(
                    [bx as f32 + corner[0], by as f32 + corner[1], bz as f32 + corner[2]],
                    *normal, UVS[ci], tangent,
                ));
            }
            ix.extend_from_slice(&[base_vert, base_vert+1, base_vert+2, base_vert, base_vert+2, base_vert+3]);
        }
    }

    let mat_of = |b: Block| match b {
        Block::Grass      => mat_grass,
        Block::Dirt       => mat_dirt,
        Block::Stone      => mat_stone,
        Block::Wood       => mat_wood,
        Block::Leaves     => mat_leaves,
        Block::StoneBrick => mat_stone_brick,
        Block::Plank      => mat_plank,
        Block::Glass      => mat_glass,
    };
    let mut result = Vec::new();
    for (block, v) in verts {
        if v.is_empty() { continue; }
        let ix = idxs.remove(&block).unwrap_or_default();
        result.push((GpuMesh::new(device, &v, &ix), mat_of(block)));
    }
    result
}

// ── VoxelChunkManager ─────────────────────────────────────────────────────────

/// Max new chunk requests sent to the streamer per frame.
const MAX_REQUESTS_PER_FRAME: usize = 6;
/// Max completed chunk events processed (mesh uploads) per frame.
const MAX_UPLOADS_PER_FRAME: usize = 4;

struct VoxelChunkManager {
    dir:        PathBuf,
    /// Chunks resident in the live `Level`: coord → (entity IDs, mesh handles to free).
    pub loaded: HashMap<ChunkCoord, (Vec<EntityId>, Vec<MeshHandle>)>,
    in_flight:       HashSet<ChunkCoord>,
    grass_mat:       MaterialHandle,
    dirt_mat:        MaterialHandle,
    stone_mat:       MaterialHandle,
    wood_mat:        MaterialHandle,
    leaves_mat:      MaterialHandle,
    stone_brick_mat: MaterialHandle,
    plank_mat:       MaterialHandle,
    glass_mat:       MaterialHandle,
    pending_ready: Vec<StreamEvent>,
}

impl VoxelChunkManager {
    #[allow(clippy::too_many_arguments)]
    fn new(
        dir:             PathBuf,
        grass_mat:       MaterialHandle,
        dirt_mat:        MaterialHandle,
        stone_mat:       MaterialHandle,
        wood_mat:        MaterialHandle,
        leaves_mat:      MaterialHandle,
        stone_brick_mat: MaterialHandle,
        plank_mat:       MaterialHandle,
        glass_mat:       MaterialHandle,
    ) -> Self {
        Self {
            dir, grass_mat, dirt_mat, stone_mat,
            wood_mat, leaves_mat, stone_brick_mat, plank_mat, glass_mat,
            loaded: HashMap::new(), in_flight: HashSet::new(), pending_ready: Vec::new(),
        }
    }

    fn desired_set(&self, cam: Vec3) -> HashSet<ChunkCoord> {
        let cx = (cam.x / CHUNK_SIZE).floor() as i32;
        let cz = (cam.z / CHUNK_SIZE).floor() as i32;
        let mut set = HashSet::new();
        for dx in -LOAD_RADIUS..=LOAD_RADIUS {
            for dz in -LOAD_RADIUS..=LOAD_RADIUS {
                for y in 0..MAX_Y_CHUNKS {
                    set.insert(ChunkCoord::new(cx + dx, y, cz + dz));
                }
            }
        }
        set
    }

    fn update(&mut self, cam: Vec3, level: &mut Level, streamer: &LevelStreamer, assets: &mut AssetRegistry) {
        let desired = self.desired_set(cam);

        let evict: Vec<ChunkCoord> = self.loaded.keys()
            .filter(|c| !desired.contains(c))
            .copied()
            .collect();
        for coord in evict {
            if let Some((ids, mesh_handles)) = self.loaded.remove(&coord) {
                for id in ids { level.despawn_entity(id); }
                for mh in mesh_handles { assets.remove(mh); }
            }
            level.partition_mut().remove_chunk(coord);
        }

        let mut to_request: Vec<ChunkCoord> = desired.into_iter()
            .filter(|c| !self.loaded.contains_key(c) && !self.in_flight.contains(c))
            .collect();
        let cx = cam.x; let cz = cam.z;
        to_request.sort_unstable_by(|a, b| {
            let da = (a.x as f32 * CHUNK_SIZE - cx).powi(2) + (a.z as f32 * CHUNK_SIZE - cz).powi(2);
            let db = (b.x as f32 * CHUNK_SIZE - cx).powi(2) + (b.z as f32 * CHUNK_SIZE - cz).powi(2);
            da.partial_cmp(&db).unwrap()
        });

        for coord in to_request.into_iter().take(MAX_REQUESTS_PER_FRAME) {
            self.in_flight.insert(coord);
            if chunk_on_disk(&self.dir, coord) {
                streamer.request_chunk(self.dir.clone(), coord);
            } else {
                streamer.request_generate_and_load(self.dir.clone(), coord, build_chunk_file(coord));
            }
        }
    }

    /// Accept newly arrived stream events into the pending queue.
    fn collect_events(&mut self, new_events: Vec<StreamEvent>) {
        self.pending_ready.extend(new_events);
    }

    /// Process up to MAX_UPLOADS_PER_FRAME pending events (GPU mesh uploads).
    fn flush_events(
        &mut self,
        level:  &mut Level,
        device: &wgpu::Device,
        assets: &mut AssetRegistry,
    ) {
        let take = self.pending_ready.len().min(MAX_UPLOADS_PER_FRAME);
        for event in self.pending_ready.drain(..take).collect::<Vec<_>>() {
            match event {
                StreamEvent::ChunkReady { coord, data } => {
                    self.in_flight.remove(&coord);
                    let submeshes = build_chunk_mesh(
                        device, data,
                        self.grass_mat, self.dirt_mat, self.stone_mat,
                        self.wood_mat, self.leaves_mat, self.stone_brick_mat,
                        self.plank_mat, self.glass_mat,
                    );
                    let cx = coord.x as f32 * CHUNK_SIZE + CHUNK_SIZE * 0.5;
                    let cz = coord.z as f32 * CHUNK_SIZE + CHUNK_SIZE * 0.5;
                    let chunk_centre = Vec3::new(cx, CHUNK_SIZE * 0.5, cz);

                    let mut ids = Vec::new();
                    let mut mesh_handles = Vec::new();

                    for (gpu_mesh, mat) in submeshes {
                        let mesh_h = assets.add(gpu_mesh);
                        mesh_handles.push(mesh_h);
                        ids.push(level.spawn_entity(
                            Components::new()
                                .with_transform(Transform::from_position(chunk_centre))
                                .with_mesh(mesh_h)
                                .with_material(mat)
                                .with_bounding_radius(CHUNK_SIZE * 2.5),
                        ));
                    }

                    level.partition_mut().get_or_create(coord).activate();
                    self.loaded.insert(coord, (ids, mesh_handles));
                }
                StreamEvent::ChunkError { coord, error } => {
                    self.in_flight.remove(&coord);
                    log::warn!("Chunk {:?}: {}", coord, error);
                }
            }
        }
    }
}

// ── Cache invalidation ────────────────────────────────────────────────────────

fn ensure_cache_valid(dir: &std::path::Path) {
    let key_path = dir.join(".cache_key");
    let current_ok = std::fs::read_to_string(&key_path)
        .map(|s| s.trim() == CACHE_KEY)
        .unwrap_or(false);
    if !current_ok {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).expect("create level dir");
        std::fs::write(&key_path, CACHE_KEY).expect("write cache key");
        log::info!("Level cache invalidated — regenerating chunks");
    }
}

// ── App ───────────────────────────────────────────────────────────────────────

struct App { state: Option<AppState> }
impl App { fn new() -> Self { Self { state: None } } }

struct AppState {
    window:         Arc<Window>,
    surface:        wgpu::Surface<'static>,
    device:         Arc<wgpu::Device>,
    queue:          Arc<wgpu::Queue>,
    surface_format: wgpu::TextureFormat,
    stratum:        Stratum,
    integration:    HelioIntegration,
    main_cam_id:    CameraId,
    chunks:         VoxelChunkManager,
    streamer:       LevelStreamer,
    last_frame:     std::time::Instant,
    keys:           HashSet<KeyCode>,
    cursor_grabbed: bool,
    mouse_delta:    (f32, f32),
    time:           f32,
    frame_count:    u32,
    fps_acc:        f32,
}

// ── ApplicationHandler ────────────────────────────────────────────────────────

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() { return; }

        ensure_cache_valid(&level_dir());

        let window = Arc::new(
            event_loop.create_window(
                Window::default_attributes()
                    .with_title("Stratum — Infinite Voxel World")
                    .with_inner_size(winit::dpi::LogicalSize::new(1280u32, 720u32)),
            ).expect("window"),
        );

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(), ..Default::default()
        });
        let surface = instance.create_surface(window.clone()).expect("surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference:       wgpu::PowerPreference::HighPerformance,
            compatible_surface:     Some(&surface),
            force_fallback_adapter: false,
        })).expect("adapter");

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label:                 Some("voxel device"),
                required_features:     wgpu::Features::EXPERIMENTAL_RAY_QUERY,
                required_limits:       wgpu::Limits::default()
                    .using_minimum_supported_acceleration_structure_values(),
                memory_hints:          wgpu::MemoryHints::default(),
                experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
                trace:                 wgpu::Trace::Off,
            },
        )).expect("device");

        let device = Arc::new(device);
        let queue  = Arc::new(queue);

        let caps = surface.get_capabilities(&adapter);
        let fmt  = caps.formats.iter().find(|f| f.is_srgb()).copied().unwrap_or(caps.formats[0]);
        let size = window.inner_size();

        surface.configure(&device, &wgpu::SurfaceConfiguration {
            usage:                         wgpu::TextureUsages::RENDER_ATTACHMENT,
            format:                        fmt,
            width:                         size.width,
            height:                        size.height,
            present_mode:                  wgpu::PresentMode::Fifo,
            alpha_mode:                    caps.alpha_modes[0],
            view_formats:                  vec![],
            desired_maximum_frame_latency: 2,
        });

        let features = FeatureRegistry::builder()
            .with_feature(LightingFeature::new())
            .with_feature(ShadowsFeature::new().with_atlas_size(2048).with_max_lights(4))
            .with_feature(BloomFeature::new().with_intensity(0.1).with_threshold(2.0))
            .build();

        let renderer = Renderer::new(
            device.clone(), queue.clone(),
            RendererConfig { width: size.width, height: size.height, surface_format: fmt, features },
        ).expect("renderer");

        let mut integration = HelioIntegration::new(renderer, AssetRegistry::new());

        // PBR materials — one per block type.
        // Chunk meshes are built at load time; only material handles are stored here.
        let make_mat = |integration: &mut HelioIntegration, color: [f32; 4], roughness: f32| {
            let g = integration.create_material(
                &Material::new().with_base_color(color).with_roughness(roughness),
            );
            integration.assets_mut().add_material(g)
        };

        let grass_mat       = make_mat(&mut integration, [0.24, 0.55, 0.16, 1.0], 0.90);
        let dirt_mat        = make_mat(&mut integration, [0.47, 0.30, 0.14, 1.0], 1.00);
        let stone_mat       = make_mat(&mut integration, [0.50, 0.50, 0.50, 1.0], 0.85);
        let wood_mat        = make_mat(&mut integration, [0.40, 0.25, 0.10, 1.0], 0.80);
        let leaves_mat      = make_mat(&mut integration, [0.15, 0.45, 0.10, 1.0], 0.95);
        let stone_brick_mat = make_mat(&mut integration, [0.55, 0.52, 0.48, 1.0], 0.80);
        let plank_mat       = make_mat(&mut integration, [0.62, 0.45, 0.22, 1.0], 0.75);
        let glass_mat       = make_mat(&mut integration, [0.60, 0.80, 0.90, 0.5], 0.10);

        let mut stratum  = Stratum::new(SimulationMode::Editor);
        let level_id     = stratum.create_level("voxel_world", CHUNK_SIZE, ACTIVATION_RADIUS);
        stratum.level_mut(level_id).unwrap().activate_all_chunks();

        // Single directional sun light.
        {
            let level = stratum.level_mut(level_id).unwrap();
            level.spawn_entity(
                Components::new()
                    .with_transform(Transform::from_position(Vec3::new(0.0, 100.0, 0.0)))
                    .with_light(LightData::Directional {
                        direction: Vec3::new(-0.4, -1.0, -0.3).normalize().to_array(),
                        color:     [1.0, 0.97, 0.88],
                        intensity: 8.0,
                    }),
            );
        }

        // Camera — start elevated, looking at terrain.
        let main_cam_id = stratum.register_camera(StratumCamera {
            id:            CameraId::PLACEHOLDER,
            kind:          CameraKind::EditorPerspective,
            position:      Vec3::new(4.0, 50.0, -12.0),
            yaw:           0.0,
            pitch:         -0.35,
            projection:    Projection::perspective(std::f32::consts::FRAC_PI_3, 0.1, 1000.0),
            render_target: RenderTargetHandle::PrimarySurface,
            viewport:      Viewport::full(),
            priority:      0,
            active:        true,
        });

        let streamer = LevelStreamer::new();
        let chunks   = VoxelChunkManager::new(
            level_dir(),
            grass_mat, dirt_mat, stone_mat,
            wood_mat, leaves_mat, stone_brick_mat, plank_mat, glass_mat,
        );

        self.state = Some(AppState {
            window, surface, device, queue, surface_format: fmt,
            stratum, integration, main_cam_id,
            chunks, streamer,
            last_frame:     std::time::Instant::now(),
            keys:           HashSet::new(),
            cursor_grabbed: false,
            mouse_delta:    (0.0, 0.0),
            time:           0.0,
            frame_count:    0,
            fps_acc:        0.0,
        });
    }

    fn window_event(
        &mut self, event_loop: &ActiveEventLoop,
        _id: WindowId, event: WindowEvent,
    ) {
        let Some(state) = &mut self.state else { return };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::KeyboardInput { event: KeyEvent {
                state: ElementState::Pressed,
                physical_key: PhysicalKey::Code(KeyCode::Escape), ..
            }, .. } => {
                if state.cursor_grabbed {
                    state.cursor_grabbed = false;
                    let _ = state.window.set_cursor_grab(CursorGrabMode::None);
                    state.window.set_cursor_visible(true);
                } else {
                    event_loop.exit();
                }
            }

            WindowEvent::KeyboardInput { event: KeyEvent {
                state: ElementState::Pressed,
                physical_key: PhysicalKey::Code(KeyCode::Tab), ..
            }, .. } => {
                state.stratum.toggle_mode();
                log::info!("Mode → {:?}", state.stratum.mode());
            }

            WindowEvent::KeyboardInput { event: KeyEvent {
                state: ks, physical_key: PhysicalKey::Code(key), ..
            }, .. } => {
                match ks {
                    ElementState::Pressed  => { state.keys.insert(key); }
                    ElementState::Released => { state.keys.remove(&key); }
                }
            }

            WindowEvent::MouseInput {
                state: ElementState::Pressed, button: MouseButton::Left, ..
            } => {
                if !state.cursor_grabbed {
                    let ok = state.window
                        .set_cursor_grab(CursorGrabMode::Confined)
                        .or_else(|_| state.window.set_cursor_grab(CursorGrabMode::Locked))
                        .is_ok();
                    if ok {
                        state.window.set_cursor_visible(false);
                        state.cursor_grabbed = true;
                    }
                }
            }

            WindowEvent::Resized(s) if s.width > 0 && s.height > 0 => {
                state.surface.configure(&state.device, &wgpu::SurfaceConfiguration {
                    usage:                         wgpu::TextureUsages::RENDER_ATTACHMENT,
                    format:                        state.surface_format,
                    width:                         s.width,
                    height:                        s.height,
                    present_mode:                  wgpu::PresentMode::Fifo,
                    alpha_mode:                    wgpu::CompositeAlphaMode::Auto,
                    view_formats:                  vec![],
                    desired_maximum_frame_latency: 2,
                });
                state.integration.resize(s.width, s.height);
            }

            WindowEvent::RedrawRequested => {
                let now = std::time::Instant::now();
                let dt  = (now - state.last_frame).as_secs_f32().min(0.1);
                state.last_frame = now;
                state.render(dt);
                state.window.request_redraw();
            }

            _ => {}
        }
    }

    fn device_event(
        &mut self, _: &ActiveEventLoop, _: winit::event::DeviceId, event: DeviceEvent,
    ) {
        let Some(state) = &mut self.state else { return };
        if let DeviceEvent::MouseMotion { delta: (dx, dy) } = event {
            if state.cursor_grabbed {
                state.mouse_delta.0 += dx as f32;
                state.mouse_delta.1 += dy as f32;
            }
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(s) = &self.state { s.window.request_redraw(); }
    }
}

// ── Per-frame logic ───────────────────────────────────────────────────────────

impl AppState {
    fn render(&mut self, dt: f32) {
        self.time       += dt;
        self.frame_count += 1;
        self.fps_acc    += dt;

        // Camera movement.
        {
            let cam = self.stratum.cameras_mut()
                .get_mut(self.main_cam_id).expect("camera");
            cam.yaw   += self.mouse_delta.0 * LOOK_SENS;
            cam.pitch  = (cam.pitch - self.mouse_delta.1 * LOOK_SENS).clamp(-1.5, 1.5);
            let fwd   = cam.forward();
            let right = cam.right();
            if self.keys.contains(&KeyCode::KeyW)      { cam.position += fwd   * CAM_SPEED * dt; }
            if self.keys.contains(&KeyCode::KeyS)      { cam.position -= fwd   * CAM_SPEED * dt; }
            if self.keys.contains(&KeyCode::KeyA)      { cam.position -= right * CAM_SPEED * dt; }
            if self.keys.contains(&KeyCode::KeyD)      { cam.position += right * CAM_SPEED * dt; }
            if self.keys.contains(&KeyCode::Space)     { cam.position += Vec3::Y * CAM_SPEED * dt; }
            if self.keys.contains(&KeyCode::ShiftLeft) { cam.position -= Vec3::Y * CAM_SPEED * dt; }
        }
        self.mouse_delta = (0.0, 0.0);

        let cam_pos = self.stratum.cameras_mut()
            .get_mut(self.main_cam_id)
            .map(|c| c.position)
            .unwrap_or(Vec3::ZERO);

        // Drain stream events into pending queue, then flush up to the per-frame cap.
        let new_events: Vec<_> = self.streamer.poll_loaded().into_iter().collect();
        self.chunks.collect_events(new_events);
        {
            let level  = self.stratum.active_level_mut().expect("level");
            let assets = self.integration.assets_mut();
            self.chunks.flush_events(level, &self.device, assets);
            self.chunks.update(cam_pos, level, &self.streamer, assets);
        }

        self.stratum.tick(dt);

        // Re-activate all manager-loaded chunks (tick's activation update can fight us).
        {
            let level = self.stratum.active_level_mut().expect("level");
            for &coord in self.chunks.loaded.keys() {
                level.partition_mut().get_or_create(coord).activate();
            }
        }

        // ── Stats every 10 frames ────────────────────────────────────────────
        if self.frame_count % 10 == 0 {
            let fps        = if self.fps_acc > 0.0 { 10.0 / self.fps_acc } else { 0.0 };
            let loaded     = self.chunks.loaded.len();
            let in_flight  = self.chunks.in_flight.len();
            let pending    = self.chunks.pending_ready.len();
            let meshes     = self.integration.assets_mut().mesh_count();
            eprintln!(
                "[frame {:5}] fps={:.1}  chunks loaded={} in_flight={} pending={}  meshes={}",
                self.frame_count, fps, loaded, in_flight, pending, meshes
            );
            self.fps_acc = 0.0;
        }

        let size  = self.window.inner_size();
        let views = self.stratum.build_views(size.width, size.height, self.time);
        if views.is_empty() { return; }

        let output = match self.surface.get_current_texture() {
            Ok(t)  => t,
            Err(e) => { log::warn!("Surface error: {e:?}"); return; }
        };
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let level = self.stratum.active_level().expect("level");


        if let Err(e) = self.integration.submit_frame(&views, level, &view, dt) {
            log::error!("Render error: {e:?}");
        }
        output.present();
    }
}

// ── main ──────────────────────────────────────────────────────────────────────

fn main() {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    log::info!(
        "Infinite voxel world — chunk {}m, load radius {} → {} chunks max",
        CHUNK_SIZE as i32, LOAD_RADIUS, (LOAD_RADIUS * 2 + 1).pow(2),
    );
    log::info!("WASD fly | Space/Shift up/down | Mouse look (click) | Tab mode | Esc exit");

    EventLoop::new().expect("event loop")
        .run_app(&mut App::new())
        .expect("run_app failed");
}
