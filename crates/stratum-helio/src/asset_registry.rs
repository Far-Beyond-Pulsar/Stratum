//! Asset registry — maps `MeshHandle` → `GpuMesh` and `MaterialHandle` → `GpuMaterial`.
//!
//! `Stratum` entities reference meshes and materials by opaque handles. The host
//! application registers actual `GpuMesh` / `GpuMaterial` objects here before the
//! first frame. `HelioIntegration` reads this registry when building Helio `Scene`s.

use std::collections::HashMap;
use stratum::{MeshHandle, MaterialHandle};
use helio_render_v2::{GpuMesh, GpuMaterial};

/// Registry that owns `GpuMesh` and `GpuMaterial` objects, keyed by handles.
pub struct AssetRegistry {
    meshes:      HashMap<MeshHandle, GpuMesh>,
    materials:   HashMap<MaterialHandle, GpuMaterial>,
    next_handle: u64,
}

impl AssetRegistry {
    pub fn new() -> Self {
        Self { meshes: HashMap::new(), materials: HashMap::new(), next_handle: 1 }
    }

    // ── Handle allocation ─────────────────────────────────────────────────────

    /// Allocate a fresh `MeshHandle` without registering a mesh yet.
    pub fn alloc_handle(&mut self) -> MeshHandle {
        let h = MeshHandle(self.next_handle);
        self.next_handle += 1;
        h
    }

    // ── Mesh registration ─────────────────────────────────────────────────────

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

    pub fn get(&self, handle: MeshHandle) -> Option<&GpuMesh> {
        self.meshes.get(&handle)
    }

    pub fn remove(&mut self, handle: MeshHandle) -> Option<GpuMesh> {
        self.meshes.remove(&handle)
    }

    pub fn len     (&self) -> usize { self.meshes.len() }
    pub fn is_empty(&self) -> bool  { self.meshes.is_empty() }

    // ── Material registration ─────────────────────────────────────────────────

    /// Allocate a fresh `MaterialHandle` without registering a material yet.
    pub fn alloc_material_handle(&mut self) -> MaterialHandle {
        let h = MaterialHandle(self.next_handle);
        self.next_handle += 1;
        h
    }

    /// Register `material` under the given handle (replaces any existing entry).
    pub fn register_material(&mut self, handle: MaterialHandle, material: GpuMaterial) {
        self.materials.insert(handle, material);
    }

    /// Allocate a new handle, register `material` under it, and return the handle.
    pub fn add_material(&mut self, material: GpuMaterial) -> MaterialHandle {
        let h = self.alloc_material_handle();
        self.materials.insert(h, material);
        h
    }

    pub fn get_material(&self, handle: MaterialHandle) -> Option<&GpuMaterial> {
        self.materials.get(&handle)
    }

    pub fn remove_material(&mut self, handle: MaterialHandle) -> Option<GpuMaterial> {
        self.materials.remove(&handle)
    }
}

impl Default for AssetRegistry {
    fn default() -> Self { Self::new() }
}
