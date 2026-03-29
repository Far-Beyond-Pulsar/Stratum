use glam::{Quat, Vec3};
use log::info;
use std::{fs, path::PathBuf, thread, time::Duration};

use stratum::{ChunkData, ChunkMetadata, WorldPartitionManager};

fn create_chunk_file(path: &PathBuf, chunk_metadata: &ChunkMetadata) {
    let chunk_data = ChunkData::new(chunk_metadata.clone());
    let bytes = chunk_data.serialize(true).expect("serialize chunk");
    fs::create_dir_all(path.parent().unwrap()).expect("create chunk dir");
    fs::write(path, bytes).expect("write chunk file");
}

fn main() {
    env_logger::init();
    let base_dir = std::env::temp_dir().join("stratum_world_partition_demo");
    let _ = fs::remove_dir_all(&base_dir);

    let mut manager = WorldPartitionManager::new(16.0, 40.0, 80.0, 120.0);

    for x in -2..=2 {
        for y in -2..=2 {
            let metadata = ChunkMetadata::new(x, y, 0, 0, 16.0);
            let path = base_dir.join(format!("chunk_{}_{}.bin", x, y));
            create_chunk_file(&path, &metadata);
            manager.upsert_chunk(metadata, path);
        }
    }

    let camera_id = manager.register_camera(Vec3::new(0.0, 8.0, 0.0), Quat::IDENTITY);

    for frame in 0..120 {
        let distance = (frame as f32 / 120.0) * 120.0 - 60.0;
        manager.update_camera_transform(camera_id, Vec3::new(distance, 8.0, 0.0), Quat::IDENTITY);
        manager.tick(1.0 / 60.0);

        let metrics = manager.metrics();
        info!("frame {} | visible {} | loaded {} | evicted {} | pending {} | tick ms {}",
            frame,
            manager.visible_chunk_ids().len(),
            metrics.chunks_loaded,
            metrics.chunks_evicted,
            metrics.pending_load_tasks,
            metrics.last_commit_ms.as_millis(),
        );
        thread::sleep(Duration::from_millis(16));
    }

    info!("demo complete");
}
