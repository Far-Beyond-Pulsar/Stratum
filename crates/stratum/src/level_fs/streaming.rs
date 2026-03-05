//! Background chunk streaming via a dedicated OS thread.
//!
//! `LevelStreamer` owns a worker thread that performs blocking file I/O off the
//! main thread.  The main thread enqueues `request_chunk` calls each frame
//! (typically for every chunk that is *about* to enter the activation radius)
//! and drains `poll_loaded` to receive the results.
//!
//! No async runtime is required — the streamer uses plain `std::sync::mpsc`
//! channels and a single `std::thread`.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use crate::chunk::ChunkCoord;

use super::format::ChunkFile;
use super::io;

// ── Internal channel types ────────────────────────────────────────────────────

enum Request {
    Load { level_dir: PathBuf, coord: ChunkCoord },
    Shutdown,
}

// ── Public event type ─────────────────────────────────────────────────────────

/// Result of a completed async chunk load.
pub enum StreamEvent {
    /// The chunk file was read and decoded successfully.
    ChunkReady {
        coord: ChunkCoord,
        data:  ChunkFile,
    },
    /// The load failed (file missing, JSON error, etc.).
    ChunkError {
        coord: ChunkCoord,
        error: String,
    },
}

// ── LevelStreamer ─────────────────────────────────────────────────────────────

/// Drives asynchronous chunk streaming for one or more levels.
///
/// # Usage
/// ```text
/// let streamer = LevelStreamer::new();
///
/// // Each frame, for every chunk about to enter the activation radius:
/// streamer.request_chunk(level_path.clone(), coord);
///
/// // Later in the same frame (or the next), drain completed loads:
/// for event in streamer.poll_loaded() {
///     match event {
///         StreamEvent::ChunkReady { coord, data } => { /* spawn entities */ }
///         StreamEvent::ChunkError { coord, error } => { log::warn!(…); }
///     }
/// }
/// ```
pub struct LevelStreamer {
    tx:      Sender<Request>,
    rx:      Receiver<StreamEvent>,
    // Kept alive so the thread is joined on drop.
    _thread: JoinHandle<()>,
}

impl LevelStreamer {
    /// Spawn the background worker thread.
    pub fn new() -> Self {
        let (req_tx, req_rx) = mpsc::channel::<Request>();
        let (evt_tx, evt_rx) = mpsc::channel::<StreamEvent>();

        let handle = thread::Builder::new()
            .name("stratum-level-streamer".into())
            .spawn(move || worker(req_rx, evt_tx))
            .expect("failed to spawn level streamer thread");

        Self { tx: req_tx, rx: evt_rx, _thread: handle }
    }

    /// Enqueue an async load request for the chunk at `coord` inside the level
    /// rooted at `level_dir`.  Non-blocking — returns immediately.
    pub fn request_chunk(&self, level_dir: PathBuf, coord: ChunkCoord) {
        // Silently ignore send errors: they only occur after `Shutdown`, which
        // means we are already in a drop/shutdown path.
        let _ = self.tx.send(Request::Load { level_dir, coord });
    }

    /// Drain all `StreamEvent`s that have completed since the last call.
    ///
    /// Returns an empty `Vec` if nothing has finished yet.  Intended to be
    /// called once per frame.
    pub fn poll_loaded(&self) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        while let Ok(evt) = self.rx.try_recv() {
            events.push(evt);
        }
        events
    }
}

impl Default for LevelStreamer {
    fn default() -> Self { Self::new() }
}

impl Drop for LevelStreamer {
    fn drop(&mut self) {
        let _ = self.tx.send(Request::Shutdown);
        // The JoinHandle is consumed by `_thread`; Rust will join the thread
        // when `_thread` is dropped after this impl runs.
    }
}

// ── Worker ────────────────────────────────────────────────────────────────────

fn worker(req_rx: Receiver<Request>, evt_tx: Sender<StreamEvent>) {
    for request in req_rx {
        match request {
            Request::Shutdown => break,
            Request::Load { level_dir, coord } => {
                let event = match io::load_chunk(&level_dir, coord) {
                    Ok(data) => StreamEvent::ChunkReady { coord, data },
                    Err(e)   => StreamEvent::ChunkError { coord, error: e.to_string() },
                };
                if evt_tx.send(event).is_err() {
                    // Receiver dropped — main thread is shutting down.
                    break;
                }
            }
        }
    }
}
