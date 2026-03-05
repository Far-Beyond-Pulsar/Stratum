//! On-disk record types for the level file system.
//!
//! Every type here is cheaply serializable to / from JSON and carries no
//! runtime-only data (GPU handles, pointers, etc.).  The live Stratum types
//! (`Level`, `Chunk`, `Components`, …) are converted to/from these records
//! inside `io.rs`; the rest of the crate never sees `serde`.

use serde::{Deserialize, Serialize};

// ── Version sentinel ──────────────────────────────────────────────────────────

pub const FORMAT_VERSION: u32 = 1;

// ── LevelManifest ─────────────────────────────────────────────────────────────

/// Root manifest — stored at `{level_dir}/level.json`.
///
/// Contains all metadata needed to reconstruct a `Level` (minus entity data,
/// which lives in individual chunk files).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LevelManifest {
    pub version:           u32,
    pub name:              String,
    pub id:                u64,
    pub chunk_size:        f32,
    pub activation_radius: f32,
    /// Side length of each index sector in chunk-coordinate units.
    ///
    /// Chunk `(x, y, z)` is indexed in sector
    /// `(x.div_euclid(bucket), y.div_euclid(bucket), z.div_euclid(bucket))`.
    /// Larger values → fewer index files but each one is larger.
    pub index_bucket_size: i32,
    /// Total number of chunk files present on disk (informational).
    pub chunk_count:       usize,
}

// ── SectorIndex ───────────────────────────────────────────────────────────────

/// Sector index — stored at
/// `{level_dir}/indexes/{sector_x}/{sector_y}/{sector_z}/index.json`.
///
/// Lists every chunk that falls within the sector's spatial region, together
/// with a cheap pre-computed entity count so callers can prioritise loads
/// without opening chunk files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectorIndex {
    pub version: u32,
    /// Sector grid coordinate `[x, y, z]`.
    pub sector:  [i32; 3],
    pub entries: Vec<ChunkEntry>,
}

/// Lightweight descriptor for one chunk within a `SectorIndex`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkEntry {
    /// Grid coordinate of the chunk `[x, y, z]`.
    pub coord:        [i32; 3],
    pub entity_count: usize,
}

// ── ChunkFile ─────────────────────────────────────────────────────────────────

/// Full chunk payload — stored at
/// `{level_dir}/chunks/{x}_{y}_{z}.chunk.json`.
///
/// Contains every entity whose `Transform.position` maps into this chunk cell,
/// plus any prefab instances that have been placed in this chunk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkFile {
    pub version:  u32,
    /// Grid coordinate of the chunk `[x, y, z]`.
    pub coord:    [i32; 3],
    pub entities: Vec<EntityRecord>,
    /// Prefab placements recorded in this chunk.
    ///
    /// Each entry either stays as a lightweight reference (`mode = "ref"`) or
    /// has already been expanded into `entities` above (`mode = "unpacked"`).
    /// Absent when empty (omitted from JSON).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prefab_instances: Vec<PrefabInstanceRecord>,
}

// ── Prefab records ────────────────────────────────────────────────────────────

/// One placed instance of a prefab recorded in a chunk file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrefabInstanceRecord {
    /// Name of the source prefab (used to look it up in the registry or on disk).
    pub prefab: String,
    /// World-space placement transform.
    pub transform: TransformRecord,
    /// `"ref"` — entities are NOT in the chunk's `entities` list; they must be
    /// resolved from the prefab definition at load time.
    /// `"unpacked"` — entities have already been expanded into `entities`.
    #[serde(default = "default_mode")]
    pub mode: String,
}

fn default_mode() -> String { "ref".into() }

/// On-disk representation of a full prefab definition.
///
/// Stored at `{level_dir}/prefabs/{name}.prefab.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrefabFile {
    pub version:  u32,
    pub name:     String,
    pub entities: Vec<EntityRecord>,
    pub volumes:  Vec<PrefabVolumeRecord>,
}

/// Serializable representation of one [`PrefabVolume`](crate::prefab::PrefabVolume).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrefabVolumeRecord {
    /// AABB minimum corner `[x, y, z]` in local (prefab) space.
    pub min:    [f32; 3],
    /// AABB maximum corner `[x, y, z]` in local (prefab) space.
    pub max:    [f32; 3],
    pub hollow: bool,
}

// ── EntityRecord ──────────────────────────────────────────────────────────────

/// Serializable snapshot of one entity's `Components`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityRecord {
    pub id:              u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transform:       Option<TransformRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mesh:            Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material:        Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub light:           Option<LightRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billboard:       Option<BillboardRecord>,
    pub bounding_radius: f32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags:            Vec<String>,
}

// ── Component records ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformRecord {
    /// `[x, y, z]`
    pub position: [f32; 3],
    /// Quaternion `[x, y, z, w]`
    pub rotation: [f32; 4],
    /// `[x, y, z]`
    pub scale:    [f32; 3],
}

/// Serializable mirror of `LightData`.  Uses a `"type"` discriminant tag so
/// the JSON is human-readable: `{ "type": "Point", "color": […], … }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum LightRecord {
    Point {
        color:     [f32; 3],
        intensity: f32,
        range:     f32,
    },
    Directional {
        direction: [f32; 3],
        color:     [f32; 3],
        intensity: f32,
    },
    Spot {
        direction:   [f32; 3],
        color:       [f32; 3],
        intensity:   f32,
        range:       f32,
        inner_angle: f32,
        outer_angle: f32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillboardRecord {
    /// `[width, height]` in world-space metres.
    pub size:         [f32; 2],
    /// RGBA linear-colour tint.
    pub color:        [f32; 4],
    pub screen_scale: bool,
}
