use crate::asset_registry::{AssetRegistry, ObjectHandle};
use stratum::{ChunkId, WorldPartitionManager};
use glam::Vec3;
use ahash::AHashMap;
use log::debug;

/// Convert a Stratum world-space camera to an abstract external camera.
/// This is a thin shim: actual Helio API may differ, but this is the expected
/// conversion layer for rendering integration.
pub fn stratum_camera_to_external<P>(position: Vec3, view_matrix: P, projection_matrix: P) -> (Vec3, P, P) {
    (position, view_matrix, projection_matrix)
}

/// Group ID in Helio's scene graph for chunk-based visibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GroupId(pub u32);

/// Adapter that bridges Stratum's world partition system with Helio's Scene.
///
/// This adapter maintains the relationship between Stratum chunks and Helio
/// scene objects, handling the conversion and synchronization between the two
/// systems.
pub struct StratumHelioAdapter {
    /// Asset registry for reference-counted Helio handles.
    asset_registry: AssetRegistry,
    /// Map from ChunkId to the Helio GroupId.
    chunk_to_group: AHashMap<ChunkId, GroupId>,
    /// Map from ChunkId to list of ObjectHandles in that chunk.
    chunk_objects: AHashMap<ChunkId, Vec<ObjectHandle>>,
    /// Next group ID to assign.
    next_group_id: u32,
}

impl StratumHelioAdapter {
    /// Create a new adapter.
    pub fn new() -> Self {
        Self {
            asset_registry: AssetRegistry::new(),
            chunk_to_group: AHashMap::new(),
            chunk_objects: AHashMap::new(),
            next_group_id: 1,
        }
    }

    /// Allocate a new group ID for a chunk.
    pub fn allocate_group(&mut self, chunk_id: ChunkId) -> GroupId {
        let group_id = GroupId(self.next_group_id);
        self.next_group_id += 1;
        self.chunk_to_group.insert(chunk_id, group_id);
        debug!("Allocated group {:?} for chunk {:?}", group_id, chunk_id);
        group_id
    }

    /// Get the group ID for a chunk, if it exists.
    pub fn get_chunk_group(&self, chunk_id: ChunkId) -> Option<GroupId> {
        self.chunk_to_group.get(&chunk_id).copied()
    }

    /// Register objects for a chunk.
    pub fn register_chunk_objects(&mut self, chunk_id: ChunkId, objects: Vec<ObjectHandle>) {
        debug!("Registering {} objects for chunk {:?}", objects.len(), chunk_id);
        self.chunk_objects.insert(chunk_id, objects);
    }

    /// Unregister all objects for a chunk.
    pub fn unregister_chunk_objects(&mut self, chunk_id: ChunkId) -> Option<Vec<ObjectHandle>> {
        let objects = self.chunk_objects.remove(&chunk_id);
        if let Some(ref objs) = objects {
            debug!("Unregistered {} objects for chunk {:?}", objs.len(), chunk_id);
        }
        objects
    }

    /// Get asset registry for manual asset management.
    pub fn asset_registry(&self) -> &AssetRegistry {
        &self.asset_registry
    }

    /// Get mutable asset registry for manual asset management.
    pub fn asset_registry_mut(&mut self) -> &mut AssetRegistry {
        &mut self.asset_registry
    }

    /// Commit visible chunks to Helio scene.
    /// This would call Helio's show_group/hide_group based on chunk visibility.
    pub fn commit_visibility(&mut self, world: &WorldPartitionManager) {
        let visible_chunks = world.visible_chunk_ids();
        debug!("Committing visibility for {} chunks", visible_chunks.len());

        // In a real implementation, this would call:
        // - scene.show_group(group_id) for visible chunks
        // - scene.hide_group(group_id) for non-visible chunks
        //
        // For now, we just track the state internally.
    }

    /// Get statistics about the adapter state.
    pub fn stats(&self) -> AdapterStats {
        AdapterStats {
            active_chunks: self.chunk_to_group.len(),
            total_objects: self.chunk_objects.values().map(|v| v.len()).sum(),
            mesh_count: self.asset_registry.mesh_count(),
            material_count: self.asset_registry.material_count(),
            texture_count: self.asset_registry.texture_count(),
        }
    }
}

impl Default for StratumHelioAdapter {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about the adapter state.
#[derive(Debug, Clone)]
pub struct AdapterStats {
    pub active_chunks: usize,
    pub total_objects: usize,
    pub mesh_count: usize,
    pub material_count: usize,
    pub texture_count: usize,
}

impl AdapterStats {
    pub fn format_summary(&self) -> String {
        format!(
            "Chunks: {} | Objects: {} | Meshes: {} | Materials: {} | Textures: {}",
            self.active_chunks,
            self.total_objects,
            self.mesh_count,
            self.material_count,
            self.texture_count
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Mat4;

    #[test]
    fn bridge_camera_conversion_roundtrip() {
        let pos = Vec3::new(1.0, 2.0, 3.0);
        let view = Mat4::IDENTITY;
        let proj = Mat4::IDENTITY;
        let (p, v, pr) = stratum_camera_to_external(pos, view, proj);
        assert_eq!(p, pos);
        assert_eq!(v, Mat4::IDENTITY);
        assert_eq!(pr, Mat4::IDENTITY);
    }

    #[test]
    fn test_adapter_group_allocation() {
        let mut adapter = StratumHelioAdapter::new();
        let chunk_id = ChunkId(12345);

        let group_id = adapter.allocate_group(chunk_id);
        assert_eq!(adapter.get_chunk_group(chunk_id), Some(group_id));
    }

    #[test]
    fn test_adapter_object_registration() {
        let mut adapter = StratumHelioAdapter::new();
        let chunk_id = ChunkId(12345);
        let objects = vec![ObjectHandle(1), ObjectHandle(2), ObjectHandle(3)];

        adapter.register_chunk_objects(chunk_id, objects.clone());

        let stats = adapter.stats();
        assert_eq!(stats.total_objects, 3);

        let removed = adapter.unregister_chunk_objects(chunk_id);
        assert_eq!(removed, Some(objects));

        let stats = adapter.stats();
        assert_eq!(stats.total_objects, 0);
    }
}
