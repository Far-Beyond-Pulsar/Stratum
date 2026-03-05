//! Compact voxel grid storage — block palette + bit-packing.
//!
//! Instead of storing one EntityRecord per voxel, we use:
//! - A palette of unique block types (u8 indices)
//! - A bit-packed array of block indices
//! - Saves 50-100x disk space vs entity-per-voxel
//!
//! Note: Currently voxel_grid is not used since terrain is procedurally
//! regenerated on load. This module is here for future use when player
//! edits need to be persisted across chunk saves.

use std::collections::HashMap;

/// Maximum blocks per palette. With 4 bits per block, we can store
/// up to 16 unique block types per chunk.
const PALETTE_SIZE: usize = 16;
const BITS_PER_BLOCK: usize = 4;
const BLOCKS_PER_U32: usize = 32 / BITS_PER_BLOCK; // 8 blocks per u32

/// Serializable compact voxel grid with block palette.
///
/// 16×16×3 chunks (768 blocks total) stored as:
/// - Palette: up to 16 unique block types with their indices
/// - Data: bit-packed u32 array (4 bits per block index)
///
/// Saves ~150 bytes per chunk vs 115KB for entity-per-voxel approach.
#[derive(Debug, Clone)]
pub struct VoxelGrid {
    /// Palette of unique block types. Index in this vec is the block's ID in the grid.
    pub palette: Vec<u8>,
    /// Bit-packed block indices. Layout: y-major (Y varies fastest),
    /// within each Y-layer, Z then X.
    /// Empty/missing palette index treated as last palette entry or air.
    pub data: Vec<u32>,
}

impl VoxelGrid {
    /// Create a new empty grid with optional capacity hint.
    pub fn new() -> Self {
        Self {
            palette: vec![],
            data: vec![],
        }
    }

    /// Create from block data. Automatically deduplicates blocks into a palette.
    ///
    /// `blocks`: Vec of (x, y, z, block_type) where block_type is the Block discriminant.
    pub fn from_blocks(blocks: &[(u32, u32, u32, u8)]) -> Self {
        let mut palette: Vec<u8> = vec![];
        let mut palette_map = std::collections::HashMap::new();
        let mut data = vec![0u32; ((16 * 16 * 3) + BLOCKS_PER_U32 - 1) / BLOCKS_PER_U32];

        for &(x, y, z, block_type) in blocks {
            // Get or create palette index for this block type
            let pal_idx = if let Some(&idx) = palette_map.get(&block_type) {
                idx
            } else {
                let idx = palette.len() as u8;
                if palette.len() >= PALETTE_SIZE {
                    // Palette overflow — skip this block (treat as air)
                    continue;
                }
                palette.push(block_type);
                palette_map.insert(block_type, idx);
                idx
            };

            // Set bit-packed value
            let flat_idx = (y as usize * 16 * 16) + (z as usize * 16) + (x as usize);
            let data_idx = flat_idx / BLOCKS_PER_U32;
            let bit_offset = (flat_idx % BLOCKS_PER_U32) * BITS_PER_BLOCK;
            data[data_idx] |= (pal_idx as u32) << bit_offset;
        }

        Self { palette, data }
    }

    /// Get block type at chunk-local coordinates (0-15 for x/z, 0-2 for y).
    pub fn get(&self, x: u32, y: u32, z: u32) -> u8 {
        if x >= 16 || y >= 3 || z >= 16 {
            return 0; // Out of bounds → air/void
        }
        let flat_idx = (y as usize * 16 * 16) + (z as usize * 16) + (x as usize);
        if self.data.is_empty() {
            return 0;
        }
        let data_idx = flat_idx / BLOCKS_PER_U32;
        let bit_offset = (flat_idx % BLOCKS_PER_U32) * BITS_PER_BLOCK;
        if data_idx >= self.data.len() {
            return 0;
        }
        let pal_idx = ((self.data[data_idx] >> bit_offset) & 0xF) as usize;
        if pal_idx < self.palette.len() {
            self.palette[pal_idx]
        } else {
            0 // Invalid index → air
        }
    }

    /// Set block type at chunk-local coordinates.
    pub fn set(&mut self, x: u32, y: u32, z: u32, block_type: u8) {
        if x >= 16 || y >= 3 || z >= 16 {
            return; // Out of bounds
        }

        // Ensure palette has this block type
        let pal_idx = if let Some(pos) = self.palette.iter().position(|&b| b == block_type) {
            pos as u8
        } else {
            if self.palette.len() >= PALETTE_SIZE {
                return; // Palette full, can't store this block
            }
            let idx = self.palette.len() as u8;
            self.palette.push(block_type);
            idx
        };

        // Expand data vec if needed
        let required_len = ((16 * 16 * 3) + BLOCKS_PER_U32 - 1) / BLOCKS_PER_U32;
        if self.data.len() < required_len {
            self.data.resize(required_len, 0);
        }

        let flat_idx = (y as usize * 16 * 16) + (z as usize * 16) + (x as usize);
        let data_idx = flat_idx / BLOCKS_PER_U32;
        let bit_offset = (flat_idx % BLOCKS_PER_U32) * BITS_PER_BLOCK;
        let mask = 0xF << bit_offset;
        self.data[data_idx] = (self.data[data_idx] & !mask) | ((pal_idx as u32) << bit_offset);
    }

    /// Iterate over all blocks as (x, y, z, block_type).
    pub fn iter(&self) -> impl Iterator<Item = (u32, u32, u32, u8)> + '_ {
        (0..(16 * 16 * 3)).map(move |flat_idx| {
            let y = (flat_idx / (16 * 16)) as u32;
            let z = ((flat_idx % (16 * 16)) / 16) as u32;
            let x = (flat_idx % 16) as u32;
            let block_type = self.get(x, y, z);
            (x, y, z, block_type)
        })
    }

    /// True if grid is empty (no blocks stored).
    pub fn is_empty(&self) -> bool {
        self.palette.is_empty() || self.data.iter().all(|&w| w == 0)
    }
}

impl Default for VoxelGrid {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_voxel_grid_set_get() {
        let mut grid = VoxelGrid::new();
        grid.set(0, 0, 0, 1);
        assert_eq!(grid.get(0, 0, 0), 1);
        assert_eq!(grid.get(1, 0, 0), 0);
    }

    #[test]
    fn test_voxel_grid_palette() {
        let blocks = vec![(0, 0, 0, 1), (1, 0, 0, 2), (2, 0, 0, 1)];
        let grid = VoxelGrid::from_blocks(&blocks);
        assert_eq!(grid.palette.len(), 2); // Only 2 unique types
        assert_eq!(grid.get(0, 0, 0), 1);
        assert_eq!(grid.get(1, 0, 0), 2);
        assert_eq!(grid.get(2, 0, 0), 1);
    }
}
