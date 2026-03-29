//! Stratum - World Partition Streaming System
//!
//! Stratum is a high-performance world partition streaming system designed for
//! large-scale 3D worlds. It provides:
//!
//! - **World Partitioning**: Divides the world into manageable chunks
//! - **Streaming System**: Asynchronous loading/unloading with budgets
//! - **LOD Management**: Level-of-detail based on distance and screen space
//! - **Multi-Camera Support**: Handle multiple viewpoints simultaneously
//! - **Serialization**: Save and load world state to/from disk
//! - **In-Memory Caching**: Keep frequently accessed chunks in memory
//!
//! # Architecture
//!
//! Stratum follows a layered architecture:
//! - **Core Types**: ChunkId, AABB, Frustum, and state management
//! - **Chunk System**: Chunk metadata, loading states, and serialization
//! - **Camera System**: Multi-camera support with frustum culling
//! - **World Manager**: WorldPartitionManager orchestrates streaming
//! - **IO System**: Async loading with compression and caching
//!
//! # Example
//!
//! ```rust,no_run
//! use stratum::WorldPartitionManager;
//! use glam::Vec3;
//!
//! let mut world = WorldPartitionManager::new(100.0, 200.0, 400.0, 600.0);
//! let camera_id = world.register_camera(Vec3::ZERO, glam::Quat::IDENTITY);
//!
//! // Update world each frame
//! world.tick(0.016); // 60 FPS
//! ```

pub mod camera;
pub mod chunk;
pub mod types;
pub mod world_partition;
pub mod streaming;
pub mod metrics;
pub mod io;

pub use crate::camera::{Camera, CameraError, CameraId, CameraRegistry};
pub use crate::chunk::{Chunk, ChunkData, ChunkError, ChunkMetadata};
pub use crate::types::{Aabb, ChunkId, ChunkState, Frustum};
pub use crate::world_partition::{StreamingBudget, WorldPartitionMetrics, WorldPartitionManager};
pub use crate::streaming::StreamingPolicy;
pub use crate::io::{ChunkIoManager, ChunkIoTask};
