//! Voxel chunk storage - a 3D grid of blocks.

use super::blocks::Block;

/// Size of a chunk in voxels (must match ChunkMetadata size in world units).
pub const CHUNK_SIZE: usize = 16;

/// A chunk of voxels - 16x16x16 grid of blocks.
pub struct VoxelChunk {
    /// Flat array of blocks [x + y * SIZE + z * SIZE * SIZE].
    pub blocks: Box<[Block; CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE]>,
}

impl VoxelChunk {
    /// Create a new empty chunk (all air).
    pub fn new() -> Self {
        Self {
            blocks: Box::new([Block::Air; CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE]),
        }
    }

    /// Get block at local chunk coordinates (0..SIZE).
    pub fn get(&self, x: usize, y: usize, z: usize) -> Block {
        if x >= CHUNK_SIZE || y >= CHUNK_SIZE || z >= CHUNK_SIZE {
            return Block::Air;
        }
        self.blocks[x + y * CHUNK_SIZE + z * CHUNK_SIZE * CHUNK_SIZE]
    }

    /// Set block at local chunk coordinates (0..SIZE).
    pub fn set(&mut self, x: usize, y: usize, z: usize, block: Block) {
        if x >= CHUNK_SIZE || y >= CHUNK_SIZE || z >= CHUNK_SIZE {
            return;
        }
        self.blocks[x + y * CHUNK_SIZE + z * CHUNK_SIZE * CHUNK_SIZE] = block;
    }

    /// Count number of non-air blocks in this chunk.
    pub fn count_solid_blocks(&self) -> usize {
        self.blocks.iter().filter(|&&b| !b.is_air()).count()
    }
}

impl Default for VoxelChunk {
    fn default() -> Self {
        Self::new()
    }
}
