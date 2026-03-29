use stratum::{ChunkMetadata, WorldPartitionManager};
use glam::{Quat, Vec3};
use std::path::PathBuf;

fn main() {
    env_logger::init();

    println!("=== Stratum World Partition Example ===\n");

    // Create world partition manager
    // Parameters: chunk_size, visible_radius, preload_radius, unload_radius
    let mut world = WorldPartitionManager::new(
        100.0,  // 100m chunks
        300.0,  // Visible within 300m
        500.0,  // Preload within 500m
        700.0,  // Unload beyond 700m
    );

    println!("World partition manager created:");
    println!("  - Chunk size: 100m");
    println!("  - Visible radius: 300m");
    println!("  - Preload radius: 500m");
    println!("  - Unload radius: 700m\n");

    // Register a camera
    let camera_id = world.register_camera(Vec3::ZERO, Quat::IDENTITY);
    println!("Camera registered at origin\n");

    // Create a grid of chunks around the origin
    println!("Creating chunk grid...");
    let grid_size = 10;
    let mut chunk_count = 0;

    for x in -grid_size..=grid_size {
        for y in -1..=1 {
            for z in -grid_size..=grid_size {
                let metadata = ChunkMetadata::new(x, y, z, 0, 100.0);

                // Create a fake file path (in real use, these would be actual chunk files)
                let file_path = PathBuf::from(format!(
                    "world_data/chunks/chunk_{}_{}_{}_{}.bin",
                    x, y, z, metadata.lod
                ));

                world.upsert_chunk(metadata, file_path);
                chunk_count += 1;
            }
        }
    }

    println!("Created {} chunks in a {}x3x{} grid\n", chunk_count, grid_size * 2 + 1, grid_size * 2 + 1);

    // Simulate camera movement and world updates
    println!("=== Simulating camera movement ===\n");

    let simulation_frames = 300; // 5 seconds at 60 FPS
    let dt = 1.0 / 60.0;

    for frame in 0..simulation_frames {
        // Move camera in a circular path
        let time = frame as f32 * dt;
        let radius = 400.0;
        let camera_pos = Vec3::new(
            radius * (time * 0.5).cos(),
            10.0,
            radius * (time * 0.5).sin(),
        );

        // Update camera position
        world.update_camera_transform(camera_id, camera_pos, Quat::IDENTITY);

        // Tick the world partition system
        world.tick(dt);

        // Print status every 60 frames (1 second)
        if frame % 60 == 0 {
            let metrics = world.metrics();
            let visible = world.visible_chunk_ids();

            println!("Frame {}: Position ({:.1}, {:.1}, {:.1})",
                frame,
                camera_pos.x,
                camera_pos.y,
                camera_pos.z
            );
            println!("  Visible chunks: {}", visible.len());
            println!("  {}", metrics.format_summary());
            println!();
        }
    }

    // Final statistics
    println!("\n=== Final Statistics ===");
    let metrics = world.metrics();
    println!("{}", metrics.format_summary());
    println!("Chunks loaded per second: {:.2}", metrics.chunks_loaded_per_second());
    println!("Chunks evicted per second: {:.2}", metrics.chunks_evicted_per_second());
    println!("Failure rate: {:.2}%", metrics.failure_rate_percent());

    println!("\n=== Example Complete ===");
}
