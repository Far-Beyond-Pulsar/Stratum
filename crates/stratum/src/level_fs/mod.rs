//! Level file system — directory-based level persistence and async streaming.
//!
//! ## On-disk layout
//!
//! ```text
//! {level_dir}/
//!   level.json                               Root manifest
//!   indexes/
//!     {sector_x}/
//!       {sector_y}/
//!         {sector_z}/
//!           index.json                       Sector index (lists chunks in this region)
//!   chunks/
//!     {x}_{y}_{z}.chunk.json                 Entity data for one partition chunk
//! ```
//!
//! ## Index system for large levels
//!
//! Sector coordinates are computed as `coord.div_euclid(index_bucket_size)`.
//! With the default bucket size of **64**, each sector covers a 64³ region of
//! chunk-coordinate space.  The hierarchical
//! `indexes/{sector_x}/{sector_y}/{sector_z}/index.json` tree means no single
//! directory ever grows unbounded, supporting absurdly large levels: a level
//! spanning ±1 000 000 chunk coords has ~31 250 sectors per axis, each backed
//! by a small JSON index file listing at most 64³ = 262 144 entries.
//!
//! Sector coordinates may be negative (e.g. `indexes/-3/0/-1/index.json`) —
//! all major operating systems support minus-prefixed directory names.
//!
//! ## Async streaming
//!
//! [`LevelStreamer`] owns a background OS thread.  Each frame, call
//! [`LevelStreamer::request_chunk`] for every chunk that is *about* to enter
//! the activation radius, then call [`LevelStreamer::poll_loaded`] to receive
//! completed [`StreamEvent`]s.  Loaded entity records can be converted back to
//! `(EntityId, Components)` pairs via [`chunk_to_components`].
//!
//! No async runtime (tokio, async-std) is required.

pub mod format;
pub mod io;
pub mod streaming;

pub use format::LevelManifest;
pub use io::{
    chunk_on_disk, chunk_to_components, discover_chunk_coords,
    load_chunk, load_manifest, load_sector_index,
    save_chunk, save_level, sector_for, DEFAULT_BUCKET_SIZE,
};
pub use streaming::{LevelStreamer, StreamEvent};

use thiserror::Error;

// ── LevelFsError ─────────────────────────────────────────────────────────────

/// Error type for all level file system operations.
#[derive(Debug, Error)]
pub enum LevelFsError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// The requested chunk file does not exist on disk.
    ///
    /// This is expected for chunks that have never been saved (e.g. freshly
    /// activated empty cells).  The streaming layer should treat this as an
    /// empty chunk rather than a fatal error.
    #[error("chunk not found: {0}")]
    ChunkNotFound(String),
}
