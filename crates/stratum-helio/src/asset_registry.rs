use ahash::AHashMap;
use std::sync::Arc;

/// Handle to a mesh in the Helio renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MeshHandle(pub u64);

/// Handle to a material in the Helio renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MaterialHandle(pub u64);

/// Handle to a texture in the Helio renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureHandle(pub u64);

/// Handle to an object in the Helio renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObjectHandle(pub u64);

/// Registry for managing Helio asset handles with reference counting.
///
/// This registry tracks which chunks are using which assets, allowing
/// safe cleanup when chunks are evicted. Assets are only removed from
/// Helio when no chunks reference them.
pub struct AssetRegistry {
    /// Map from mesh ID to (handle, ref count).
    mesh_refs: AHashMap<u64, (MeshHandle, usize)>,
    /// Map from material ID to (handle, ref count).
    material_refs: AHashMap<u64, (MaterialHandle, usize)>,
    /// Map from texture ID to (handle, ref count).
    texture_refs: AHashMap<u64, (TextureHandle, usize)>,
    /// Next handle ID for meshes.
    next_mesh_id: u64,
    /// Next handle ID for materials.
    next_material_id: u64,
    /// Next handle ID for textures.
    next_texture_id: u64,
}

impl AssetRegistry {
    /// Create a new empty asset registry.
    pub fn new() -> Self {
        Self {
            mesh_refs: AHashMap::new(),
            material_refs: AHashMap::new(),
            texture_refs: AHashMap::new(),
            next_mesh_id: 1,
            next_material_id: 1,
            next_texture_id: 1,
        }
    }

    /// Register or increment reference to a mesh.
    /// Returns the handle and whether it's newly created.
    pub fn register_mesh(&mut self, mesh_id: u64) -> (MeshHandle, bool) {
        if let Some((handle, ref_count)) = self.mesh_refs.get_mut(&mesh_id) {
            *ref_count += 1;
            (*handle, false)
        } else {
            let handle = MeshHandle(self.next_mesh_id);
            self.next_mesh_id += 1;
            self.mesh_refs.insert(mesh_id, (handle, 1));
            (handle, true)
        }
    }

    /// Unregister or decrement reference to a mesh.
    /// Returns true if the mesh should be removed from Helio (ref count reached 0).
    pub fn unregister_mesh(&mut self, mesh_id: u64) -> Option<MeshHandle> {
        if let Some((handle, ref_count)) = self.mesh_refs.get_mut(&mesh_id) {
            *ref_count -= 1;
            if *ref_count == 0 {
                let handle = *handle;
                self.mesh_refs.remove(&mesh_id);
                return Some(handle);
            }
        }
        None
    }

    /// Register or increment reference to a material.
    pub fn register_material(&mut self, material_id: u64) -> (MaterialHandle, bool) {
        if let Some((handle, ref_count)) = self.material_refs.get_mut(&material_id) {
            *ref_count += 1;
            (*handle, false)
        } else {
            let handle = MaterialHandle(self.next_material_id);
            self.next_material_id += 1;
            self.material_refs.insert(material_id, (handle, 1));
            (handle, true)
        }
    }

    /// Unregister or decrement reference to a material.
    pub fn unregister_material(&mut self, material_id: u64) -> Option<MaterialHandle> {
        if let Some((handle, ref_count)) = self.material_refs.get_mut(&material_id) {
            *ref_count -= 1;
            if *ref_count == 0 {
                let handle = *handle;
                self.material_refs.remove(&material_id);
                return Some(handle);
            }
        }
        None
    }

    /// Register or increment reference to a texture.
    pub fn register_texture(&mut self, texture_id: u64) -> (TextureHandle, bool) {
        if let Some((handle, ref_count)) = self.texture_refs.get_mut(&texture_id) {
            *ref_count += 1;
            (*handle, false)
        } else {
            let handle = TextureHandle(self.next_texture_id);
            self.next_texture_id += 1;
            self.texture_refs.insert(texture_id, (handle, 1));
            (handle, true)
        }
    }

    /// Unregister or decrement reference to a texture.
    pub fn unregister_texture(&mut self, texture_id: u64) -> Option<TextureHandle> {
        if let Some((handle, ref_count)) = self.texture_refs.get_mut(&texture_id) {
            *ref_count -= 1;
            if *ref_count == 0 {
                let handle = *handle;
                self.texture_refs.remove(&texture_id);
                return Some(handle);
            }
        }
        None
    }

    /// Get current mesh count.
    pub fn mesh_count(&self) -> usize {
        self.mesh_refs.len()
    }

    /// Get current material count.
    pub fn material_count(&self) -> usize {
        self.material_refs.len()
    }

    /// Get current texture count.
    pub fn texture_count(&self) -> usize {
        self.texture_refs.len()
    }

    /// Clear all asset references.
    pub fn clear(&mut self) {
        self.mesh_refs.clear();
        self.material_refs.clear();
        self.texture_refs.clear();
    }
}

impl Default for AssetRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mesh_reference_counting() {
        let mut registry = AssetRegistry::new();

        let (handle1, is_new1) = registry.register_mesh(100);
        assert!(is_new1);

        let (handle2, is_new2) = registry.register_mesh(100);
        assert!(!is_new2);
        assert_eq!(handle1, handle2);

        assert_eq!(registry.mesh_count(), 1);

        assert!(registry.unregister_mesh(100).is_none());
        assert!(registry.unregister_mesh(100).is_some());
        assert_eq!(registry.mesh_count(), 0);
    }

    #[test]
    fn test_multiple_asset_types() {
        let mut registry = AssetRegistry::new();

        registry.register_mesh(1);
        registry.register_material(1);
        registry.register_texture(1);

        assert_eq!(registry.mesh_count(), 1);
        assert_eq!(registry.material_count(), 1);
        assert_eq!(registry.texture_count(), 1);

        registry.clear();
        assert_eq!(registry.mesh_count(), 0);
    }
}
