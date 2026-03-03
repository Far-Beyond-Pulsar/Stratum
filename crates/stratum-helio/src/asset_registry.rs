//! Asset registry — maps `MeshHandle` → `GpuMesh`.
//!
//! `Stratum` entities reference meshes by an opaque `MeshHandle`. The host
//! application registers actual `GpuMesh` objects here before the first frame.
//! `HelioIntegration` reads this registry when building Helio `Scene`s.

use std::collections::HashMap;
use stratum::MeshHandle;
use helio_render_v2::GpuMesh;

/// Registry that owns `GpuMesh` objects, keyed by `MeshHandle`.
pub struct AssetRegistry {
    meshes:      HashMap<MeshHandle, GpuMesh>,
    next_handle: u64,
}

impl AssetRegistry {
    pub fn new() -> Self {
        Self { meshes: HashMap::new(), next_handle: 1 }
    }

    // ── Handle allocation ─────────────────────────────────────────────────────

    /// Allocate a fresh `MeshHandle` without registering a mesh yet.
    pub fn alloc_handle(&mut self) -> MeshHandle {
        let h = MeshHandle(self.next_handle);
        self.next_handle += 1;
        h
    }

    // ── Registration ──────────────────────────────────────────────────────────

    /// Register `mesh` under the given handle (replaces any existing entry).
    pub fn register(&mut self, handle: MeshHandle, mesh: GpuMesh) {
        self.meshes.insert(handle, mesh);
    }

    /// Allocate a new handle, register `mesh` under it, and return the handle.
    pub fn add(&mut self, mesh: GpuMesh) -> MeshHandle {
        let h = self.alloc_handle();
        self.meshes.insert(h, mesh);
        h
    }

    // ── Lookup ────────────────────────────────────────────────────────────────

    pub fn get(&self, handle: MeshHandle) -> Option<&GpuMesh> {
        self.meshes.get(&handle)
    }

    pub fn remove(&mut self, handle: MeshHandle) -> Option<GpuMesh> {
        self.meshes.remove(&handle)
    }

    pub fn len     (&self) -> usize { self.meshes.len() }
    pub fn is_empty(&self) -> bool  { self.meshes.is_empty() }
}

impl Default for AssetRegistry {
    fn default() -> Self { Self::new() }
}
