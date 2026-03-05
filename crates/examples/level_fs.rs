//! # Level File System — end-to-end walkthrough
//!
//! This example demonstrates the full lifecycle of a partitioned level stored
//! on disk with Stratum's level file system:
//!
//! 1. **Build** a level with entities spread across multiple spatial chunks.
//! 2. **Save** it — produces a directory tree with `level.json`, sector
//!    indexes under `indexes/{x}/{y}/{z}/`, and per-chunk data files under
//!    `chunks/`.
//! 3. **Inspect** the manifest and sector index programmatically.
//! 4. **Load a specific chunk** synchronously (one-off editor-style access).
//! 5. **Stream chunks asynchronously** via `LevelStreamer` — the same path a
//!    running game takes as the camera approaches un-loaded regions.
//!
//! No GPU, no window, no renderer — this example is entirely headless.

use std::path::PathBuf;
use std::time::Duration;
use std::thread;

use glam::Vec3;

use stratum::{
    // Level construction
    Level, LevelId,
    // Entity model
    Components, Transform, MeshHandle, MaterialHandle, LightData, BillboardData,
    // Spatial types
    ChunkCoord,
    // Level file system
    save_level, load_manifest, load_chunk, load_sector_index, chunk_to_components,
    LevelStreamer, StreamEvent, LevelFsError,
    level_fs::io::DEFAULT_BUCKET_SIZE,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn level_dir() -> PathBuf {
    // Stored at {workspace_root}/levels/example_world/
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("levels")
        .join("example_world")
}

fn heading(title: &str) {
    println!("\n── {} {}", title, "─".repeat(60usize.saturating_sub(title.len() + 4)));
}

// ─────────────────────────────────────────────────────────────────────────────
fn main() {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    // ── 1. Build a level ──────────────────────────────────────────────────────
    heading("1. Building level");

    //  chunk_size = 32 m  |  activation_radius = 96 m  (≈ 3-chunk view distance)
    let mut level = Level::new(LevelId::new(1), "example_world", 32.0, 96.0);

    // Scatter entities across several chunks so we get multiple chunk files and
    // multiple sector-index entries.
    let positions: &[(&str, Vec3)] = &[
        // chunk (0,0,0)
        ("rock_a",      Vec3::new( 10.0,  0.0,  10.0)),
        ("rock_b",      Vec3::new( 15.0,  0.0,   5.0)),
        // chunk (1,0,0)  — world x ∈ [32, 64)
        ("tree_oak",    Vec3::new( 40.0,  0.0,  20.0)),
        ("campfire",    Vec3::new( 50.0,  0.0,  15.0)),
        // chunk (0,0,1)  — world z ∈ [32, 64)
        ("ruins_arch",  Vec3::new( 12.0,  0.0,  45.0)),
        // chunk (-1,0,0) — world x ∈ [-32, 0)
        ("torch_left",  Vec3::new(-10.0,  2.0,   8.0)),
        ("torch_right", Vec3::new(-10.0,  2.0,  24.0)),
        // chunk (2,0,2)  — world x ∈ [64,96), z ∈ [64,96)
        ("boss_statue", Vec3::new( 70.0,  0.0,  70.0)),
    ];

    for (tag, pos) in positions {
        let id = level.spawn_entity(
            Components::new()
                .with_transform(Transform::from_position(*pos))
                .with_mesh(MeshHandle(1))
                .with_material(MaterialHandle(42))
                .with_tag(*tag),
        );
        println!("  Spawned {:>12}  id={:>3}  chunk={:?}",
            tag, id.raw(),
            level.partition().coord_for(*pos));
    }

    // A light entity in chunk (0,0,0)
    level.spawn_entity(
        Components::new()
            .with_transform(Transform::from_position(Vec3::new(8.0, 3.0, 8.0)))
            .with_light(LightData::Point {
                color:     [1.0, 0.9, 0.6],
                intensity: 800.0,
                range:     20.0,
            })
            .with_billboard(BillboardData::new(0.4, 0.4, [1.0, 0.9, 0.6, 1.0]))
            .with_tag("lantern"),
    );

    println!("\n  Total entities : {}", level.entities().len());
    println!("  Chunk count    : {}", level.partition().chunks().count());

    // ── 2. Save the level ─────────────────────────────────────────────────────
    heading("2. Saving level to disk");

    let dir = level_dir();
    // Remove any leftover from a previous run.
    let _ = std::fs::remove_dir_all(&dir);

    save_level(&level, &dir).expect("save_level failed");
    println!("  Saved to: {}", dir.display());

    // Show the directory tree.
    print_dir_tree(&dir, 0);

    // ── 3. Inspect manifest and sector index ──────────────────────────────────
    heading("3. Manifest & sector index");

    let manifest = load_manifest(&dir).expect("load_manifest failed");
    println!("  name              : {}", manifest.name);
    println!("  id                : {}", manifest.id);
    println!("  chunk_size        : {} m", manifest.chunk_size);
    println!("  activation_radius : {} m", manifest.activation_radius);
    println!("  index_bucket_size : {}", manifest.index_bucket_size);
    println!("  chunk_count       : {}", manifest.chunk_count);

    // Load the sector index for the chunk at the origin.
    let origin_coord = ChunkCoord::new(0, 0, 0);
    let sector_idx = load_sector_index(&dir, origin_coord, DEFAULT_BUCKET_SIZE)
        .expect("load_sector_index failed");
    println!("\n  Sector {:?} — {} entries:", sector_idx.sector, sector_idx.entries.len());
    for entry in &sector_idx.entries {
        println!("    chunk {:>3},{:>3},{:>3}  —  {} entities",
            entry.coord[0], entry.coord[1], entry.coord[2],
            entry.entity_count);
    }

    // ── 4. Load one chunk synchronously ───────────────────────────────────────
    heading("4. Synchronous chunk load");

    let coord = level.partition().coord_for(Vec3::new(40.0, 0.0, 20.0)); // tree_oak
    println!("  Loading chunk {:?} …", coord);

    let chunk_file = load_chunk(&dir, coord).expect("load_chunk failed");
    println!("  Entities in chunk:");
    for ent in &chunk_file.entities {
        println!("    id={:<3}  mesh={:?}  tags={:?}", ent.id, ent.mesh, ent.tags);
    }

    // Convert records back to `Components` (what the game would do at runtime).
    let pairs = chunk_to_components(chunk_file);
    assert_eq!(pairs.len(), 2, "expected tree_oak + campfire");
    println!("  Reconstructed {} component set(s) — ready to re-spawn.", pairs.len());

    // ── 5. Async streaming via LevelStreamer ───────────────────────────────────
    heading("5. Async streaming");

    let dir_clone = dir.clone();
    let streamer = LevelStreamer::new();

    // Simulate the camera advancing toward several chunks that need to load.
    // The actual partition system calls partition.update_activation(); here we
    // just exercise the streamer directly.
    let chunks_to_stream = vec![
        ChunkCoord::new( 0, 0,  0),
        ChunkCoord::new( 1, 0,  0),
        ChunkCoord::new( 0, 0,  1),
        ChunkCoord::new(-1, 0,  0),
        ChunkCoord::new( 2, 0,  2),
        // This one was never written — expect a ChunkError (empty cell).
        ChunkCoord::new(99, 0, 99),
    ];

    println!("  Requesting {} chunk loads …", chunks_to_stream.len());
    for coord in &chunks_to_stream {
        streamer.request_chunk(dir_clone.clone(), *coord);
    }

    // Poll until all expected events arrive (with a timeout guard).
    let mut ready   = 0usize;
    let mut errors  = 0usize;
    let deadline    = std::time::Instant::now() + Duration::from_secs(5);

    while ready + errors < chunks_to_stream.len() {
        if std::time::Instant::now() > deadline {
            println!("  WARNING: timed out waiting for streamer events");
            break;
        }

        for event in streamer.poll_loaded() {
            match event {
                StreamEvent::ChunkReady { coord, data } => {
                    ready += 1;
                    println!("  ✓  Chunk {:>3},{:>3},{:>3}  loaded — {} entities",
                        coord.x, coord.y, coord.z, data.entities.len());
                }
                StreamEvent::ChunkError { coord, error } => {
                    errors += 1;
                    // Empty / unsaved cells are expected and not fatal.
                    println!("  ⚠  Chunk {:>3},{:>3},{:>3}  not found ({})",
                        coord.x, coord.y, coord.z, error);
                }
            }
        }

        if ready + errors < chunks_to_stream.len() {
            thread::sleep(Duration::from_millis(10));
        }
    }

    println!("\n  Results: {} ready, {} errors/missing", ready, errors);
    assert_eq!(ready,  5, "5 chunks should load successfully");
    assert_eq!(errors, 1, "1 chunk was never written (coord 99,0,99)");

    // ── Missing chunk check ───────────────────────────────────────────────────
    heading("6. ChunkNotFound error type");

    let res = load_chunk(&dir, ChunkCoord::new(999, 999, 999));
    match res {
        Err(LevelFsError::ChunkNotFound(desc)) =>
            println!("  Got expected ChunkNotFound: {}", desc),
        other =>
            panic!("unexpected result: {:?}", other),
    }

    // ── Done ──────────────────────────────────────────────────────────────────
    heading("Done");
    println!("  All steps completed successfully.");
    println!("  Level files saved to: {}\n", dir.display());
}

// ── Utility: print a simple tree ─────────────────────────────────────────────

fn print_dir_tree(path: &PathBuf, depth: usize) {
    let indent = "  ".repeat(depth + 1);
    let Ok(entries) = std::fs::read_dir(path) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let p    = entry.path();
        let name = p.file_name().unwrap_or_default().to_string_lossy();
        if p.is_dir() {
            println!("{}📁 {}/", indent, name);
            print_dir_tree(&p, depth + 1);
        } else {
            let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            println!("{}📄 {}  ({} bytes)", indent, name, size);
        }
    }
}
