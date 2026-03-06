//! Synchronous file I/O for the level file system.
//!
//! All public functions operate on a `level_dir: &Path` that points to the
//! root of the level directory (the folder that contains `level.json`).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use glam::{Quat, Vec3};

use crate::chunk::ChunkCoord;
use crate::entity::{
    BillboardData, Components, EntityId, LightData, MaterialHandle, MeshHandle,
    SkyAtmosphereData, SkylightData, Transform,
};
use crate::level::Level;

use super::format::{
    BillboardRecord, ChunkEntry, ChunkFile, EntityRecord, LightRecord, SectorIndex,
    SkyAtmosphereRecord, SkylightRecord, TransformRecord, FORMAT_VERSION,
};
use super::{LevelFsError, LevelManifest};

// ── Path helpers ──────────────────────────────────────────────────────────────

/// Default index bucket size in chunk-coordinate units.
pub const DEFAULT_BUCKET_SIZE: i32 = 64;

/// Return the sector coordinate that contains `coord` for the given bucket size.
///
/// Uses Euclidean (floor) division so negative coordinates are bucketed
/// correctly: coord `(-1, 0, 0)` with `bucket = 64` → sector `(-1, 0, 0)`.
#[inline]
pub fn sector_for(coord: ChunkCoord, bucket_size: i32) -> [i32; 3] {
    [
        coord.x.div_euclid(bucket_size),
        coord.y.div_euclid(bucket_size),
        coord.z.div_euclid(bucket_size),
    ]
}

/// `{level_dir}/level.json`
#[inline]
pub fn manifest_path(level_dir: &Path) -> PathBuf {
    level_dir.join("level.json")
}

/// `{level_dir}/indexes/{sx}/{sy}/{sz}/index.json`
///
/// Each path segment is the signed decimal integer for that axis, so the tree
/// is naturally ordered and spans arbitrarily large coordinate spaces without
/// special encoding.
#[inline]
pub fn sector_index_path(level_dir: &Path, sector: [i32; 3]) -> PathBuf {
    level_dir
        .join("indexes")
        .join(sector[0].to_string())
        .join(sector[1].to_string())
        .join(sector[2].to_string())
        .join("index.json")
}

/// `{level_dir}/chunks/{x}_{y}_{z}.chunk.json`
#[inline]
pub fn chunk_file_path(level_dir: &Path, coord: ChunkCoord) -> PathBuf {
    level_dir
        .join("chunks")
        .join(format!("{}_{}_{}.chunk.json", coord.x, coord.y, coord.z))
}

// ── Save ──────────────────────────────────────────────────────────────────────

/// Serialise a `Level` to `level_dir`, creating the full directory tree.
///
/// # Layout produced
/// ```text
/// {level_dir}/
///   level.json
///   indexes/{sx}/{sy}/{sz}/index.json   (one per occupied sector)
///   chunks/{x}_{y}_{z}.chunk.json       (one per occupied chunk)
/// ```
pub fn save_level(level: &Level, level_dir: &Path) -> Result<(), LevelFsError> {
    let bucket_size = DEFAULT_BUCKET_SIZE;


    println!("Saving level '{}' to '{}'", level.name, level_dir.display());
    fs::create_dir_all(level_dir.join("chunks"))?;

    let mut sector_map: HashMap<[i32; 3], Vec<ChunkEntry>> = HashMap::new();
    let mut chunk_count = 0usize;

    for chunk in level.partition().chunks() {
        let coord = chunk.coord;

        // Build entity records only for entities that still live in the store.
        let entity_records: Vec<EntityRecord> = chunk
            .entities
            .iter()
            .filter_map(|&id| level.entities().get(id).map(|c| entity_to_record(id, c)))
            .collect();

        let chunk_file = ChunkFile {
            version:          FORMAT_VERSION,
            coord:            [coord.x, coord.y, coord.z],
            entities:         entity_records.clone(),
            prefab_instances: Vec::new(),
        };

        write_json(&chunk_file_path(level_dir, coord), &chunk_file)?;

        // Register in the appropriate sector index.
        let sector = sector_for(coord, bucket_size);
        sector_map.entry(sector).or_default().push(ChunkEntry {
            coord:        [coord.x, coord.y, coord.z],
            entity_count: entity_records.len(),
        });

        chunk_count += 1;
    }

    // Write one sector index file per occupied sector.
    for (sector, entries) in &sector_map {
        let path = sector_index_path(level_dir, *sector);
        fs::create_dir_all(path.parent().expect("sector index path has parent"))?;
        write_json(&path, &SectorIndex {
            version: FORMAT_VERSION,
            sector:  *sector,
            entries: entries.clone(),
        })?;
    }

    // Write the root manifest last so a partial write leaves it absent.
    write_json(&manifest_path(level_dir), &LevelManifest {
        version:           FORMAT_VERSION,
        name:              level.name.clone(),
        id:                level.id().raw(),
        chunk_size:        level.partition().chunk_size,
        activation_radius: level.partition().activation_radius,
        index_bucket_size: bucket_size,
        chunk_count,
    })?;

    log::info!(
        "Saved level '{}' → {} chunks, {} sectors",
        level.name,
        chunk_count,
        sector_map.len()
    );
    Ok(())
}

// ── Load ──────────────────────────────────────────────────────────────────────

/// Read the root manifest without loading any chunk data.
///
/// This is cheap (a single JSON file) and should be called to validate the
/// level before beginning a streaming session.
pub fn load_manifest(level_dir: &Path) -> Result<LevelManifest, LevelFsError> {
    println!("Loading level manifest from '{}'", level_dir.display());
    read_json(&manifest_path(level_dir))
}

/// Load the raw `ChunkFile` for the chunk at `coord`.
///
/// Returns `LevelFsError::ChunkNotFound` if the file does not exist.
pub fn load_chunk(level_dir: &Path, coord: ChunkCoord) -> Result<ChunkFile, LevelFsError> {
    println!("Loading chunk at '{:?}' from '{}'", coord, level_dir.display());
    let path = chunk_file_path(level_dir, coord);
    if !path.exists() {
        return Err(LevelFsError::ChunkNotFound(format!("{:?}", coord)));
    }
    read_json(&path)
}

/// Load the sector index that covers `coord`.
///
/// The `bucket_size` should come from `LevelManifest::index_bucket_size`.
pub fn load_sector_index(
    level_dir:   &Path,
    coord:       ChunkCoord,
    bucket_size: i32,
) -> Result<SectorIndex, LevelFsError> {
    println!("Loading sector index for chunk at '{:?}' from '{}'", coord, level_dir.display());
    let sector = sector_for(coord, bucket_size);
    read_json(&sector_index_path(level_dir, sector))
}

// ── Single-chunk write ────────────────────────────────────────────────────────

/// Returns `true` if the chunk file for `coord` already exists on disk.
///
/// Cheap filesystem stat — safe to call every frame per candidate chunk.
pub fn chunk_on_disk(level_dir: &Path, coord: ChunkCoord) -> bool {
    println!("Checking if chunk at '{:?}' exists on disk in '{}'", coord, level_dir.display());
    chunk_file_path(level_dir, coord).exists()
}

/// Save (or overwrite) a single chunk to disk and upsert its entry in the
/// appropriate sector index.
///
/// Does **not** update the level manifest; use [`save_level`] when serialising
/// a complete level for the first time.  This function is for the streaming
/// write path where chunks are generated lazily on a background thread.
pub fn save_chunk(
    level_dir:  &Path,
    coord:      ChunkCoord,
    chunk_file: &ChunkFile,
    bucket_size: i32,
) -> Result<(), LevelFsError> {
    println!("Saving chunk at '{:?}' to '{}'", coord, level_dir.display());
    fs::create_dir_all(level_dir.join("chunks"))?;
    write_json(&chunk_file_path(level_dir, coord), chunk_file)?;

    // Upsert the sector index entry for this chunk.
    let sector   = sector_for(coord, bucket_size);
    let idx_path = sector_index_path(level_dir, sector);
    fs::create_dir_all(idx_path.parent().expect("sector path has parent"))?;

    let mut idx: SectorIndex = if idx_path.exists() {
        read_json(&idx_path)?
    } else {
        SectorIndex { version: FORMAT_VERSION, sector, entries: Vec::new() }
    };

    // Remove stale entry (if re-saving) then insert fresh one.
    idx.entries.retain(|e| e.coord != [coord.x, coord.y, coord.z]);
    idx.entries.push(ChunkEntry {
        coord:        [coord.x, coord.y, coord.z],
        entity_count: chunk_file.entities.len(),
    });

    write_json(&idx_path, &idx)
}

// ── Conversion helpers ────────────────────────────────────────────────────────

/// Deserialise a `ChunkFile` into `(EntityId, Components)` pairs ready to
/// be re-spawned into a `Level`.
pub fn chunk_to_components(chunk: ChunkFile) -> Vec<(EntityId, Components)> {
    chunk
        .entities
        .into_iter()
        .map(|rec| {
            let id = EntityId::new(rec.id);
            (id, record_to_components(rec))
        })
        .collect()
}

/// Walk `{level_dir}/indexes/` recursively and collect every `ChunkCoord`
/// that has a written index entry — without loading individual chunk files.
///
/// Useful at startup to know which coords to pre-request from the streamer.
pub fn discover_chunk_coords(level_dir: &Path) -> Result<Vec<ChunkCoord>, LevelFsError> {
    println!("Discovering chunk coordinates from indexes in '{}'", level_dir.display());

    let mut coords = Vec::new();
    let index_root = level_dir.join("indexes");
    if !index_root.exists() {
        return Ok(coords);
    }
    collect_index_entries(&index_root, &mut coords)?;
    Ok(coords)
}

fn collect_index_entries(dir: &Path, out: &mut Vec<ChunkCoord>) -> Result<(), LevelFsError> {
    println!("Collecting chunk coordinates from '{}'", dir.display());

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path  = entry.path();
        if path.is_dir() {
            collect_index_entries(&path, out)?;
        } else if path.file_name().map(|n| n == "index.json").unwrap_or(false) {
            let idx: SectorIndex = read_json(&path)?;
            for e in idx.entries {
                out.push(ChunkCoord::new(e.coord[0], e.coord[1], e.coord[2]));
            }
        }
    }
    Ok(())
}


fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), LevelFsError> {
    println!("Writing JSON to '{}'", path.display());

    let json = serde_json::to_string_pretty(value)?;
    fs::write(path, json)?;
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, LevelFsError> {
    println!("Reading JSON from '{}'", path.display());

    let text  = fs::read_to_string(path)?;
    let value = serde_json::from_str(&text)?;
    Ok(value)
}

fn entity_to_record(id: EntityId, c: &Components) -> EntityRecord {
    entity_to_record_pub(id.raw(), c)
}

/// Public-to-the-module version of `entity_to_record` used by `prefab_io`.
pub(super) fn entity_to_record_pub(id: u64, c: &Components) -> EntityRecord {
    EntityRecord {
        id,
        transform:       c.transform.as_ref().map(transform_to_record),
        mesh:            c.mesh.map(|m| m.0),
        material:        c.material.map(|m| m.0),
        light:           c.light.as_ref().map(light_to_record),
        skylight:        c.skylight.as_ref().map(skylight_to_record),
        sky_atmosphere:  c.sky_atmosphere.as_ref().map(sky_atmosphere_to_record),
        billboard:       c.billboard.as_ref().map(billboard_to_record),
        bounding_radius: c.bounding_radius,
        tags:            c.tags.clone(),
    }
}

/// Deserialise a single [`EntityRecord`] into a [`Components`] bag.
///
/// Used by `prefab_io` to convert stored prefab entities back to runtime data.
pub(super) fn record_to_components(rec: EntityRecord) -> Components {
    let mut c = Components::new();
    if let Some(t) = rec.transform {
        c = c.with_transform(Transform {
            position: Vec3::from_array(t.position),
            rotation: Quat::from_array(t.rotation),
            scale:    Vec3::from_array(t.scale),
        });
    }
    if let Some(m) = rec.mesh     { c = c.with_mesh(MeshHandle(m)); }
    if let Some(m) = rec.material { c = c.with_material(MaterialHandle(m)); }
    if let Some(l) = rec.light          { c = c.with_light(light_from_record(l)); }
    if let Some(s) = rec.skylight       { c = c.with_skylight(skylight_from_record(s)); }
    if let Some(a) = rec.sky_atmosphere { c = c.with_sky_atmosphere(sky_atmosphere_from_record(a)); }
    if let Some(b) = rec.billboard {
        c = c.with_billboard(BillboardData {
            size:         b.size,
            color:        b.color,
            screen_scale: b.screen_scale,
        });
    }
    c.bounding_radius = rec.bounding_radius;
    for tag in rec.tags { c = c.with_tag(tag); }
    c
}

fn transform_to_record(t: &Transform) -> TransformRecord {
    TransformRecord {
        position: t.position.to_array(),
        rotation: t.rotation.to_array(),
        scale:    t.scale.to_array(),
    }
}

fn light_to_record(l: &LightData) -> LightRecord {
    match *l {
        LightData::Point { color, intensity, range } =>
            LightRecord::Point { color, intensity, range },
        LightData::Directional { direction, color, intensity } =>
            LightRecord::Directional { direction, color, intensity },
        LightData::Spot { direction, color, intensity, range, inner_angle, outer_angle } =>
            LightRecord::Spot { direction, color, intensity, range, inner_angle, outer_angle },
    }
}

fn billboard_to_record(b: &BillboardData) -> BillboardRecord {
    BillboardRecord { size: b.size, color: b.color, screen_scale: b.screen_scale }
}

fn skylight_to_record(s: &SkylightData) -> SkylightRecord {
    SkylightRecord { intensity: s.intensity, color_tint: s.color_tint }
}

fn skylight_from_record(r: SkylightRecord) -> SkylightData {
    SkylightData { intensity: r.intensity, color_tint: r.color_tint }
}

fn sky_atmosphere_to_record(a: &SkyAtmosphereData) -> SkyAtmosphereRecord {
    SkyAtmosphereRecord {
        rayleigh_scatter: a.rayleigh_scatter,
        rayleigh_h_scale: a.rayleigh_h_scale,
        mie_scatter:      a.mie_scatter,
        mie_h_scale:      a.mie_h_scale,
        mie_g:            a.mie_g,
        sun_intensity:    a.sun_intensity,
        sun_disk_angle:   a.sun_disk_angle,
        earth_radius:     a.earth_radius,
        atm_radius:       a.atm_radius,
        exposure:         a.exposure,
    }
}

fn sky_atmosphere_from_record(r: SkyAtmosphereRecord) -> SkyAtmosphereData {
    SkyAtmosphereData {
        rayleigh_scatter: r.rayleigh_scatter,
        rayleigh_h_scale: r.rayleigh_h_scale,
        mie_scatter:      r.mie_scatter,
        mie_h_scale:      r.mie_h_scale,
        mie_g:            r.mie_g,
        sun_intensity:    r.sun_intensity,
        sun_disk_angle:   r.sun_disk_angle,
        earth_radius:     r.earth_radius,
        atm_radius:       r.atm_radius,
        exposure:         r.exposure,
    }
}

fn light_from_record(l: LightRecord) -> LightData {
    match l {
        LightRecord::Point { color, intensity, range } =>
            LightData::Point { color, intensity, range },
        LightRecord::Directional { direction, color, intensity } =>
            LightData::Directional { direction, color, intensity },
        LightRecord::Spot { direction, color, intensity, range, inner_angle, outer_angle } =>
            LightData::Spot { direction, color, intensity, range, inner_angle, outer_angle },
    }
}

// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::{Level, LevelId};
    use crate::entity::{Components, MeshHandle, Transform};
    use glam::Vec3;

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("stratum_level_fs_test_{}", name));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn sector_for_positive() {
        let c = ChunkCoord::new(65, 0, 0);
        assert_eq!(sector_for(c, 64), [1, 0, 0]);
    }

    #[test]
    fn sector_for_negative() {
        // -1 div_euclid 64 == -1  (bucket -1 covers [-64..-1])
        let c = ChunkCoord::new(-1, 0, 0);
        assert_eq!(sector_for(c, 64), [-1, 0, 0]);
    }

    #[test]
    fn save_and_reload_manifest() {
        let dir   = scratch_dir("manifest");
        let mut level = Level::new(LevelId::new(42), "test_level", 16.0, 32.0);
        level.spawn_entity(
            Components::new()
                .with_transform(Transform::from_position(Vec3::ZERO))
                .with_mesh(MeshHandle(1)),
        );
        save_level(&level, &dir).expect("save failed");

        let manifest = load_manifest(&dir).expect("load failed");
        assert_eq!(manifest.name, "test_level");
        assert_eq!(manifest.id, 42);
        assert_eq!(manifest.chunk_size, 16.0);
        assert_eq!(manifest.chunk_count, 1);
    }

    #[test]
    fn chunk_round_trip() {
        let dir   = scratch_dir("chunk_rt");
        let mut level = Level::new(LevelId::new(1), "rt", 16.0, 64.0);
        let pos   = Vec3::new(5.0, 0.0, 5.0);
        let id    = level.spawn_entity(
            Components::new()
                .with_transform(Transform::from_position(pos))
                .with_mesh(MeshHandle(7))
                .with_tag("static"),
        );
        save_level(&level, &dir).expect("save failed");

        let coord = level.partition().coord_for(pos);
        let chunk = load_chunk(&dir, coord).expect("load chunk failed");
        assert_eq!(chunk.entities.len(), 1);
        assert_eq!(chunk.entities[0].id, id.raw());
        assert_eq!(chunk.entities[0].mesh, Some(7));
        assert_eq!(chunk.entities[0].tags, vec!["static"]);
    }

    #[test]
    fn sector_index_written() {
        let dir   = scratch_dir("sector_idx");
        let mut level = Level::new(LevelId::new(1), "si", 16.0, 64.0);
        level.spawn_entity(
            Components::new().with_transform(Transform::from_position(Vec3::ZERO)),
        );
        save_level(&level, &dir).expect("save failed");

        let coord = ChunkCoord::new(0, 0, 0);
        let idx   = load_sector_index(&dir, coord, DEFAULT_BUCKET_SIZE).expect("no sector index");
        assert!(!idx.entries.is_empty());
    }

    #[test]
    fn chunk_not_found_error() {
        let dir = scratch_dir("not_found");
        fs::create_dir_all(&dir).unwrap();
        let res = load_chunk(&dir, ChunkCoord::new(99, 99, 99));
        assert!(matches!(res, Err(LevelFsError::ChunkNotFound(_))));
    }
}
