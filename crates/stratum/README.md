# Stratum - World Partition Streaming System

**Stratum** is a high-performance world partition streaming system for large-scale 3D worlds, integrating seamlessly with the Helio renderer.

## Features

✅ **World Partitioning** - Automatically divides large worlds into manageable chunks
✅ **Async Streaming** - Background loading/unloading with framerate-independent budgets
✅ **LOD Management** - Distance-based level-of-detail selection
✅ **Multi-Camera Support** - Handle multiple viewpoints simultaneously
✅ **Frustum Culling** - Only load chunks visible to cameras
✅ **Serialization** - Save/load world state to compressed chunk files
✅ **In-Memory Caching** - Keep frequently accessed chunks cached
✅ **Helio Integration** - Automatic rendering via `stratum-helio` bridge

## Quick Start

```rust
use stratum::WorldPartitionManager;
use glam::{Vec3, Quat};

// Create world partition manager
let mut world = WorldPartitionManager::new(
    100.0,  // Chunk size (meters)
    300.0,  // Visible radius
    500.0,  // Preload radius
    700.0,  // Unload radius
);

// Register a camera
let camera_id = world.register_camera(Vec3::ZERO, Quat::IDENTITY);

// Add chunks to the world
let metadata = ChunkMetadata::new(0, 0, 0, 0, 100.0);
world.upsert_chunk(metadata, PathBuf::from("chunk_0_0_0.bin"));

// Update each frame
world.tick(delta_time);
```

## With Helio Rendering

```rust
use stratum_helio::StratumRenderer;

// Create renderer (wraps Helio)
let mut renderer = StratumRenderer::new(device, queue, width, height, format);
renderer.set_clear_color([0.1, 0.1, 0.15, 1.0]);

// Render the world
if let Some(camera) = world.registry.get_camera(camera_id) {
    renderer.render_world(&world, camera, &output_view)?;
}
```

That's it! **You never touch Helio directly** - Stratum handles everything.

## Architecture

- **`stratum`** - Core world partition system
- **`stratum-helio`** - Helio renderer integration (you rarely use this directly)
- **`examples`** - Demo applications

## Examples

Run the visual demo:
```bash
cargo run --bin visual_world
```

Controls:
- **WASD** - Move camera
- **Space/Shift** - Up/Down
- **ESC** - Exit

## How It Works

1. World is divided into fixed-size chunks
2. Cameras report their position each frame
3. Chunks within visible radius are loaded asynchronously
4. Frustum culling hides chunks outside camera view
5. Chunks beyond unload radius are evicted from memory
6. Helio renders visible chunks as wireframe boxes (or your custom geometry)

## Metrics

```rust
let metrics = world.metrics();
println!("Loaded: {}, Evicted: {}, Pending: {}",
    metrics.chunks_loaded,
    metrics.chunks_evicted,
    metrics.pending_load_tasks
);
```

## License

MIT OR Apache-2.0
