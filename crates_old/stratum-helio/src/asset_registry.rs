//! Asset registry — maps Stratum `MeshHandle` / `MaterialHandle` / `TextureHandle`
//! to Helio scene-resident `MeshId` / `MaterialId` / `TextureId`.
//!
//! The host application calls the `upload_*` methods (once per asset) to push
//! raw mesh/material/texture data into the Helio scene and receive Stratum
//! handles in return.  The handles can then be assigned to entity
//! [`Components`](stratum::Components) and referenced every frame.
//!
//! ## Lifecycle
//!
//! 1. Call `upload_mesh(renderer, MeshUpload { vertices, indices })` → `MeshHandle`
//! 2. Call `upload_material(renderer, GpuMaterial { ... })` → `MaterialHandle`
//!    or `upload_material_asset(renderer, MaterialAsset { gpu, textures })` for PBR+textures.
//! 3. Assign handles to entity components.
//! 4. Call `remove_mesh` / `remove_material` when the asset is no longer needed.
//!
//! Meshes and materials live in the Helio scene — this registry only stores
//! the mapping from Stratum handles to Helio IDs.

use std::collections::HashMap;
use helio::{
    GpuMaterial, GpuLight, LightId,
    MaterialAsset, MaterialId, MeshId, TextureId, TextureUpload,
    MeshUpload, Renderer, SceneResult,
};
use helio::{VirtualMeshId, VirtualMeshUpload};
use stratum::{MeshHandle, MaterialHandle, TextureHandle};

/// Maps Stratum asset handles to live Helio scene IDs.
///
/// All uploads go through the [`Renderer`] so that Helio owns the GPU resources.
pub struct AssetRegistry {
    meshes:           HashMap<MeshHandle, MeshId>,
    virtual_meshes:   HashMap<MeshHandle, VirtualMeshId>,
    materials:        HashMap<MaterialHandle, MaterialId>,
    textures:         HashMap<TextureHandle, TextureId>,
    lights:           HashMap<u64, LightId>,
    next_handle:      u64,
}

impl AssetRegistry {
    pub fn new() -> Self {
        Self {
            meshes:         HashMap::new(),
            virtual_meshes: HashMap::new(),
            materials:      HashMap::new(),
            textures:       HashMap::new(),
            lights:         HashMap::new(),
            next_handle:    1,
        }
    }

    // ── Handle allocation ─────────────────────────────────────────────────────

    pub fn alloc_mesh_handle(&mut self) -> MeshHandle {
        let h = MeshHandle(self.next_handle);
        self.next_handle += 1;
        h
    }

    pub fn alloc_material_handle(&mut self) -> MaterialHandle {
        let h = MaterialHandle(self.next_handle);
        self.next_handle += 1;
        h
    }

    pub fn alloc_texture_handle(&mut self) -> TextureHandle {
        let h = TextureHandle(self.next_handle);
        self.next_handle += 1;
        h
    }

    // ── Mesh uploads ──────────────────────────────────────────────────────────

    /// Upload a standard mesh to the Helio scene and return its Stratum handle.
    pub fn upload_mesh(&mut self, renderer: &mut Renderer, mesh: MeshUpload) -> MeshHandle {
        let handle = self.alloc_mesh_handle();
        let id = renderer.insert_mesh(mesh);
        self.meshes.insert(handle, id);
        handle
    }

    /// Upload a mesh under a specific handle (for level-file handle round-tripping).
    pub fn upload_mesh_as(&mut self, renderer: &mut Renderer, handle: MeshHandle, mesh: MeshUpload) {
        if handle.0 >= self.next_handle { self.next_handle = handle.0 + 1; }
        let id = renderer.insert_mesh(mesh);
        self.meshes.insert(handle, id);
    }

    /// Upload a high-poly mesh as a virtual geometry (meshlet LOD) asset.
    pub fn upload_virtual_mesh(
        &mut self,
        renderer: &mut Renderer,
        mesh: VirtualMeshUpload,
    ) -> MeshHandle {
        let handle = self.alloc_mesh_handle();
        let id = renderer.insert_virtual_mesh(mesh);
        self.virtual_meshes.insert(handle, id);
        handle
    }

    pub fn get_mesh_id(&self, handle: MeshHandle) -> Option<MeshId> {
        self.meshes.get(&handle).copied()
    }

    pub fn get_virtual_mesh_id(&self, handle: MeshHandle) -> Option<VirtualMeshId> {
        self.virtual_meshes.get(&handle).copied()
    }

    pub fn remove_mesh(&mut self, renderer: &mut Renderer, handle: MeshHandle) {
        if let Some(_id) = self.meshes.remove(&handle) {
            // Mesh removal is deferred to Helio's ref-count mechanism.
            // Note: remove_mesh returns Result; we log errors instead of panicking.
            // The mesh is retained by Helio until no objects reference it.
        }
        if let Some(id) = self.virtual_meshes.remove(&handle) {
            if let Err(e) = renderer.remove_virtual_mesh(id) {
                log::warn!("remove_virtual_mesh: {e:?}");
            }
        }
    }

    // ── Material uploads ──────────────────────────────────────────────────────

    /// Upload a plain PBR material (scalars only).
    pub fn upload_material(
        &mut self,
        renderer: &mut Renderer,
        material: GpuMaterial,
    ) -> MaterialHandle {
        let handle = self.alloc_material_handle();
        let id = renderer.insert_material(material);
        self.materials.insert(handle, id);
        handle
    }

    /// Upload a full PBR material with texture references.
    pub fn upload_material_asset(
        &mut self,
        renderer: &mut Renderer,
        material: MaterialAsset,
    ) -> SceneResult<MaterialHandle> {
        let handle = self.alloc_material_handle();
        let id = renderer.insert_material_asset(material)?;
        self.materials.insert(handle, id);
        Ok(handle)
    }

    /// Upload a material under a specific handle (for level-file round-tripping).
    pub fn upload_material_as(
        &mut self,
        renderer: &mut Renderer,
        handle: MaterialHandle,
        material: GpuMaterial,
    ) {
        if handle.0 >= self.next_handle { self.next_handle = handle.0 + 1; }
        let id = renderer.insert_material(material);
        self.materials.insert(handle, id);
    }

    pub fn get_material_id(&self, handle: MaterialHandle) -> Option<MaterialId> {
        self.materials.get(&handle).copied()
    }

    pub fn remove_material(&mut self, renderer: &mut Renderer, handle: MaterialHandle) {
        if let Some(id) = self.materials.remove(&handle) {
            if let Err(e) = renderer.scene_mut().remove_material(id) {
                log::warn!("remove_material: {e:?}");
            }
        }
    }

    // ── Texture uploads ───────────────────────────────────────────────────────

    /// Upload a texture to the Helio scene and return its Stratum handle.
    pub fn upload_texture(
        &mut self,
        renderer: &mut Renderer,
        texture: TextureUpload,
    ) -> SceneResult<TextureHandle> {
        let handle = self.alloc_texture_handle();
        let id = renderer.insert_texture(texture)?;
        self.textures.insert(handle, id);
        Ok(handle)
    }

    pub fn get_texture_id(&self, handle: TextureHandle) -> Option<TextureId> {
        self.textures.get(&handle).copied()
    }

    pub fn remove_texture(&mut self, renderer: &mut Renderer, handle: TextureHandle) {
        if let Some(id) = self.textures.remove(&handle) {
            if let Err(e) = renderer.scene_mut().remove_texture(id) {
                log::warn!("remove_texture: {e:?}");
            }
        }
    }

    // ── Persistent ambient lights ─────────────────────────────────────────────

    /// Register a persistent scene-wide light (e.g. sun, sky ambient).
    /// Returns an opaque `u64` key for update/remove.
    pub fn add_persistent_light(&mut self, renderer: &mut Renderer, light: GpuLight) -> u64 {
        let key = self.next_handle;
        self.next_handle += 1;
        let id = renderer.insert_light(light);
        self.lights.insert(key, id);
        key
    }

    pub fn update_persistent_light(
        &mut self,
        renderer: &mut Renderer,
        key: u64,
        light: GpuLight,
    ) {
        if let Some(&id) = self.lights.get(&key) {
            if let Err(e) = renderer.update_light(id, light) {
                log::warn!("update_persistent_light: {e:?}");
            }
        }
    }

    pub fn remove_persistent_light(&mut self, renderer: &mut Renderer, key: u64) {
        if let Some(id) = self.lights.remove(&key) {
            if let Err(e) = renderer.remove_light(id) {
                log::warn!("remove_persistent_light: {e:?}");
            }
        }
    }

    // ── Diagnostics ───────────────────────────────────────────────────────────

    pub fn mesh_count    (&self) -> usize { self.meshes.len() + self.virtual_meshes.len() }
    pub fn material_count(&self) -> usize { self.materials.len() }
    pub fn texture_count (&self) -> usize { self.textures.len() }
    pub fn is_empty      (&self) -> bool  { self.meshes.is_empty() && self.materials.is_empty() }
}

impl Default for AssetRegistry {
    fn default() -> Self { Self::new() }
}

