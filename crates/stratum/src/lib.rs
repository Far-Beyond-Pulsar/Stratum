// ─────────────────────────────────────────────────────────────────────────────
//  Stratum — World Orchestration Layer for Pulsar
// ─────────────────────────────────────────────────────────────────────────────
//!
//! Stratum sits between game/editor code and the Helio renderer.
//!
//! ## Responsibilities
//!
//! * **Level management** — load, unload, and activate `Level` containers.
//! * **World partition** — grid-based spatial streaming; chunk activation
//!   driven by camera proximity.
//! * **Camera registry** — every viewport into the world is a `StratumCamera`
//!   with a stable `CameraId`.
//! * **Mode switching** — `Editor` vs `Game`; hot-switchable at runtime.
//! * **Render view production** — `Stratum::build_views()` emits one
//!   `RenderView` per active camera each frame.
//!
//! ## Hard Separation Guarantee
//!
//! Stratum does **not** depend on any GPU crate. It contains no `wgpu`,
//! no `Arc<Buffer>`, no shader handles. All GPU work is done by the
//! `stratum-helio` integration crate which consumes `Vec<RenderView>`.
//!
//! ## Data Flow
//!
//! ```text
//!  Game/Editor code
//!        │  spawn entities, move cameras, switch modes
//!        ▼
//!  Stratum ── tick() → update partition activation
//!        │  build_views() → Vec<RenderView>
//!        ▼
//!  stratum-helio  translate RenderViews → Helio Scene + Camera
//!        │  submit_frame()
//!        ▼
//!  Helio (renderer) — zero knowledge of Levels or Entities
//! ```

pub mod chunk;
pub mod partition;
pub mod entity;
pub mod camera;
pub mod camera_registry;
pub mod render_view;
pub mod visibility;
pub mod render_graph;
pub mod level;
pub mod mode;
pub mod level_fs;

mod stratum;

// ── Top-level re-exports ──────────────────────────────────────────────────────
pub use stratum::Stratum;
pub use mode::SimulationMode;
pub use level::{Level, LevelId, StreamingState};
pub use chunk::{Chunk, ChunkCoord, ChunkState, Aabb};
pub use partition::WorldPartition;
pub use entity::{
    EntityId, EntityStore, Components, Transform,
    MeshHandle, MaterialHandle, LightData, BillboardData,
};
pub use camera::{CameraId, StratumCamera, CameraKind, Projection};
pub use camera_registry::CameraRegistry;
pub use render_view::{RenderView, RenderTargetHandle, Viewport};
pub use visibility::Frustum;
pub use level_fs::{
    LevelManifest, LevelStreamer, StreamEvent, LevelFsError,
    save_level, save_chunk, load_manifest, load_chunk, load_sector_index,
    chunk_to_components, chunk_on_disk, discover_chunk_coords,
};
