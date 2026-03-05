//! File I/O for prefab definitions and instance unpacking.
//!
//! ## On-disk layout
//!
//! ```text
//! {level_dir}/
//!   prefabs/
//!     {name}.prefab.json   ← one file per named prefab
//! ```
//!
//! ## Unpacking
//!
//! [`unpack_instance`] expands a [`PrefabInstanceRecord`] (a lightweight
//! placement reference) into a set of [`EntityRecord`]s ready to be inserted
//! into a [`ChunkFile`]'s `entities` list.  The caller is responsible for
//! removing the `PrefabInstanceRecord` from the list and marking the instance
//! `mode = "unpacked"` if they want to persist the expansion.

use std::fs;
use std::path::{Path, PathBuf};

use glam::{Mat4, Quat, Vec3};

use crate::chunk::Aabb;
use crate::entity::Components;
use crate::prefab::{Prefab, PrefabVolume};

use super::format::{
    EntityRecord, PrefabFile, PrefabInstanceRecord, PrefabVolumeRecord,
    TransformRecord, FORMAT_VERSION,
};
use super::io::{entity_to_record_pub, record_to_components};
use super::LevelFsError;

// ── Path helpers ──────────────────────────────────────────────────────────────

/// `{level_dir}/prefabs/{name}.prefab.json`
pub fn prefab_file_path(level_dir: &Path, name: &str) -> PathBuf {
    level_dir.join("prefabs").join(format!("{}.prefab.json", name))
}

// ── Save ──────────────────────────────────────────────────────────────────────

/// Serialise a [`Prefab`] to `{level_dir}/prefabs/{name}.prefab.json`.
///
/// Creates the `prefabs/` subdirectory if needed.
pub fn save_prefab(level_dir: &Path, prefab: &Prefab) -> Result<(), LevelFsError> {
    let path = prefab_file_path(level_dir, &prefab.name);
    fs::create_dir_all(path.parent().expect("prefab path has parent"))?;

    let entities: Vec<EntityRecord> = prefab.entities.iter().enumerate()
        .map(|(i, c)| entity_to_record_pub(i as u64 + 1, c))
        .collect();

    let volumes: Vec<PrefabVolumeRecord> = prefab.volumes.iter()
        .map(|v| PrefabVolumeRecord {
            min:    v.bounds.min.to_array(),
            max:    v.bounds.max.to_array(),
            hollow: v.hollow,
        })
        .collect();

    let file = PrefabFile {
        version: FORMAT_VERSION,
        name:    prefab.name.clone(),
        entities,
        volumes,
    };

    write_json(&path, &file)
}

// ── Load ──────────────────────────────────────────────────────────────────────

/// Load a [`Prefab`] from `{level_dir}/prefabs/{name}.prefab.json`.
pub fn load_prefab(level_dir: &Path, name: &str) -> Result<Prefab, LevelFsError> {
    let path = prefab_file_path(level_dir, name);
    if !path.exists() {
        return Err(LevelFsError::PrefabNotFound(name.to_string()));
    }
    let file: PrefabFile = read_json(&path)?;
    Ok(prefab_from_file(file))
}

// ── Unpack ────────────────────────────────────────────────────────────────────

/// Expand a [`PrefabInstanceRecord`] into a flat list of [`EntityRecord`]s.
///
/// The `id_offset` is added to each entity's local numeric id to produce a
/// chunk-unique entity id.  Typical usage: pass the current `entities.len()`
/// before the call so ids don't collide.
///
/// Transforms in the returned records are **world-space** (the placement
/// transform has been applied).
pub fn unpack_instance(
    instance:  &PrefabInstanceRecord,
    prefab:    &Prefab,
    id_offset: u64,
) -> Vec<EntityRecord> {
    let t = &instance.transform;
    let place_mat = Mat4::from_scale_rotation_translation(
        Vec3::from_array(t.scale),
        Quat::from_array(t.rotation),
        Vec3::from_array(t.position),
    );

    prefab.entities.iter().enumerate().map(|(i, template)| {
        // Build local→world transform matrix.
        let local_mat = if let Some(ref lt) = template.transform {
            Mat4::from_scale_rotation_translation(
                lt.scale,
                lt.rotation,
                lt.position,
            )
        } else {
            Mat4::IDENTITY
        };
        let world_mat = place_mat * local_mat;
        let (scale, rot, pos) = world_mat.to_scale_rotation_translation();

        let mut rec = entity_to_record_pub(id_offset + i as u64 + 1, template);
        rec.transform = Some(TransformRecord {
            position: pos.to_array(),
            rotation: rot.to_array(),
            scale:    scale.to_array(),
        });
        rec
    }).collect()
}

// ── Conversions ───────────────────────────────────────────────────────────────

fn prefab_from_file(file: PrefabFile) -> Prefab {
    use crate::prefab::{Prefab as P, PrefabId};

    let entities: Vec<Components> = file.entities.into_iter()
        .map(|rec| record_to_components(rec))
        .collect();

    let volumes: Vec<PrefabVolume> = file.volumes.into_iter()
        .map(|v| PrefabVolume {
            bounds: Aabb::new(Vec3::from_array(v.min), Vec3::from_array(v.max)),
            hollow: v.hollow,
        })
        .collect();

    P {
        id:       PrefabId(0), // caller's registry will reassign
        name:     file.name,
        entities,
        volumes,
    }
}

// ── JSON helpers (private) ────────────────────────────────────────────────────

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), LevelFsError> {
    let json = serde_json::to_string_pretty(value)?;
    fs::write(path, json)?;
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, LevelFsError> {
    let text  = fs::read_to_string(path)?;
    let value = serde_json::from_str(&text)?;
    Ok(value)
}

// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::Aabb;
    use crate::entity::{Components, MeshHandle, Transform};
    use crate::prefab::{Prefab, PrefabVolume};
    use glam::Vec3;

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("stratum_prefab_io_test_{}", name));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn make_tree_prefab() -> Prefab {
        Prefab::builder("tree_oak")
            .with_entity(
                Components::new()
                    .with_transform(Transform::from_position(Vec3::new(0.0, 2.0, 0.0)))
                    .with_mesh(MeshHandle(4))
                    .with_bounding_radius(2.0),
            )
            .with_volume(PrefabVolume::solid(Aabb::new(
                Vec3::new(-0.5, 0.0, -0.5),
                Vec3::new( 0.5, 4.0,  0.5),
            )))
            .with_volume(PrefabVolume::hollow(Aabb::new(
                Vec3::new(-2.5, 2.0, -2.5),
                Vec3::new( 2.5, 7.0,  2.5),
            )))
            .build()
    }

    #[test]
    fn save_and_load_prefab_round_trip() {
        let dir    = scratch_dir("rt");
        let prefab = make_tree_prefab();
        save_prefab(&dir, &prefab).expect("save failed");
        let loaded = load_prefab(&dir, "tree_oak").expect("load failed");
        assert_eq!(loaded.name, "tree_oak");
        assert_eq!(loaded.entities.len(), 1);
        assert_eq!(loaded.volumes.len(), 2);
        assert!(!loaded.volumes[0].hollow);
        assert!( loaded.volumes[1].hollow);
    }

    #[test]
    fn load_prefab_missing_returns_error() {
        let dir = scratch_dir("missing");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(matches!(load_prefab(&dir, "no_such"), Err(LevelFsError::PrefabNotFound(_))));
    }

    #[test]
    fn unpack_instance_applies_world_offset() {
        let prefab = make_tree_prefab();
        let instance = PrefabInstanceRecord {
            prefab:    "tree_oak".into(),
            transform: TransformRecord {
                position: [10.0, 5.0, 10.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale:    [1.0, 1.0, 1.0],
            },
            mode: "ref".into(),
        };
        let records = unpack_instance(&instance, &prefab, 0);
        assert_eq!(records.len(), 1);
        let pos = records[0].transform.as_ref().unwrap().position;
        // Local entity is at (0,2,0); placement at (10,5,10) → world (10,7,10)
        assert!((pos[0] - 10.0).abs() < 0.01, "x={}", pos[0]);
        assert!((pos[1] -  7.0).abs() < 0.01, "y={}", pos[1]);
        assert!((pos[2] - 10.0).abs() < 0.01, "z={}", pos[2]);
    }
}
