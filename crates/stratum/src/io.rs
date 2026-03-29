use crate::chunk::{ChunkData, ChunkError};
use crate::types::ChunkId;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;

/// Async chunk IO task for loading from disk.
pub struct ChunkIoTask {
    pub chunk_id: ChunkId,
    pub file_path: PathBuf,
    pub compressed: bool,
}

/// Manager for asynchronous chunk IO operations.
pub struct ChunkIoManager {
    runtime: Arc<tokio::runtime::Runtime>,
}

impl ChunkIoManager {
    /// Create a new ChunkIoManager with a dedicated async runtime.
    pub fn new() -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create IO runtime");

        Self {
            runtime: Arc::new(runtime),
        }
    }

    /// Load a chunk from disk asynchronously.
    pub async fn load_chunk(
        &self,
        _chunk_id: ChunkId,
        file_path: PathBuf,
        compressed: bool,
    ) -> Result<ChunkData, ChunkError> {
        let bytes = fs::read(&file_path).await?;
        ChunkData::deserialize(&bytes, compressed)
    }

    /// Save a chunk to disk asynchronously.
    pub async fn save_chunk(
        &self,
        chunk_data: &ChunkData,
        file_path: PathBuf,
        compressed: bool,
    ) -> Result<(), ChunkError> {
        let bytes = chunk_data.serialize(compressed)?;
        fs::write(&file_path, &bytes).await?;
        Ok(())
    }

    /// Load multiple chunks in parallel.
    pub async fn load_chunks_batch(
        &self,
        tasks: Vec<ChunkIoTask>,
    ) -> Vec<(ChunkId, Result<ChunkData, ChunkError>)> {
        let mut handles = Vec::new();

        for task in tasks {
            let file_path = task.file_path.clone();
            let chunk_id = task.chunk_id;
            let compressed = task.compressed;

            let handle = tokio::spawn(async move {
                let result = async {
                    let bytes = fs::read(&file_path).await?;
                    ChunkData::deserialize(&bytes, compressed)
                }
                .await;
                (chunk_id, result)
            });

            handles.push(handle);
        }

        let mut results = Vec::new();
        for handle in handles {
            if let Ok(result) = handle.await {
                results.push(result);
            }
        }

        results
    }
}

impl Default for ChunkIoManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::ChunkMetadata;

    #[test]
    fn test_io_manager_creation() {
        let _manager = ChunkIoManager::new();
        // Just verify it can be created without panicking
    }

    #[tokio::test]
    async fn test_chunk_save_and_load() {
        let manager = ChunkIoManager::new();
        let metadata = ChunkMetadata::new(0, 0, 0, 0, 100.0);
        let chunk_data = ChunkData::new(metadata);

        let temp_path = std::env::temp_dir().join("test_chunk.bin");

        // Save
        manager
            .save_chunk(&chunk_data, temp_path.clone(), true)
            .await
            .unwrap();

        // Load
        let loaded = manager
            .load_chunk(metadata.id, temp_path.clone(), true)
            .await
            .unwrap();

        assert_eq!(loaded.version, ChunkData::VERSION);

        // Cleanup
        let _ = std::fs::remove_file(temp_path);
    }
}
