use glam::{Vec3, Vec4};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Unique identifier for a chunk in the world partition system.
/// Encoded as a 64-bit integer for efficient storage and hashing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChunkId(pub u64);

impl ChunkId {
    /// Create a ChunkId from 3D grid coordinates with LOD level.
    /// Format: [LOD:8][X:18][Y:18][Z:18] (2 bits reserved)
    pub fn from_coords(x: i32, y: i32, z: i32, lod: u8) -> Self {
        const COORD_BITS: u32 = 18;
        const COORD_MASK: i32 = (1 << COORD_BITS) - 1;

        let x_encoded = (x & COORD_MASK) as u64;
        let y_encoded = (y & COORD_MASK) as u64;
        let z_encoded = (z & COORD_MASK) as u64;
        let lod_encoded = (lod as u64) << 56;

        ChunkId(lod_encoded | (x_encoded << 36) | (y_encoded << 18) | z_encoded)
    }

    /// Decode the ChunkId back to grid coordinates and LOD.
    pub fn to_coords(self) -> (i32, i32, i32, u8) {
        const COORD_BITS: u32 = 18;
        const COORD_MASK: u64 = (1 << COORD_BITS) - 1;
        const SIGN_BIT: u64 = 1 << (COORD_BITS - 1);
        const SIGN_EXTEND: i32 = !((1 << COORD_BITS) - 1);

        let lod = (self.0 >> 56) as u8;
        let x_raw = ((self.0 >> 36) & COORD_MASK) as i32;
        let y_raw = ((self.0 >> 18) & COORD_MASK) as i32;
        let z_raw = (self.0 & COORD_MASK) as i32;

        // Sign-extend coordinates
        let x = if (x_raw as u64 & SIGN_BIT) != 0 { x_raw | SIGN_EXTEND } else { x_raw };
        let y = if (y_raw as u64 & SIGN_BIT) != 0 { y_raw | SIGN_EXTEND } else { y_raw };
        let z = if (z_raw as u64 & SIGN_BIT) != 0 { z_raw | SIGN_EXTEND } else { z_raw };

        (x, y, z, lod)
    }
}

impl fmt::Display for ChunkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (x, y, z, lod) = self.to_coords();
        write!(f, "ChunkId({}, {}, {} @ LOD{})", x, y, z, lod)
    }
}

/// Represents the loading and visibility state of a chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChunkState {
    /// Chunk is not loaded and not in memory.
    Unloaded,
    /// Chunk is being loaded from disk (async operation in progress).
    Loading,
    /// Chunk is loaded into memory but not currently visible.
    Loaded,
    /// Chunk is loaded and currently visible/rendered.
    Visible,
    /// Chunk is hidden (loaded but visibility toggled off).
    Hidden,
    /// Chunk is being evicted from memory.
    Evicting,
}

impl Default for ChunkState {
    fn default() -> Self {
        ChunkState::Unloaded
    }
}

/// Axis-aligned bounding box for spatial queries and frustum culling.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    /// Create a new AABB from min and max points.
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    /// Create an AABB from a center point and half-extents.
    pub fn from_center_extents(center: Vec3, half_extents: Vec3) -> Self {
        Self {
            min: center - half_extents,
            max: center + half_extents,
        }
    }

    /// Get the center point of the AABB.
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// Get the half-extents (size from center to edge).
    pub fn half_extents(&self) -> Vec3 {
        (self.max - self.min) * 0.5
    }

    /// Get the full size of the AABB.
    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }

    /// Check if this AABB contains a point.
    pub fn contains_point(&self, point: Vec3) -> bool {
        point.cmpge(self.min).all() && point.cmple(self.max).all()
    }

    /// Check if this AABB intersects another AABB.
    pub fn intersects(&self, other: &Aabb) -> bool {
        self.min.cmple(other.max).all() && self.max.cmpge(other.min).all()
    }

    /// Expand the AABB to include a point.
    pub fn expand_to_include(&mut self, point: Vec3) {
        self.min = self.min.min(point);
        self.max = self.max.max(point);
    }

    /// Get the maximum extent dimension (for LOD calculations).
    pub fn max_extent(&self) -> f32 {
        let size = self.size();
        size.x.max(size.y).max(size.z)
    }
}

/// View frustum for culling chunks outside the camera's view.
#[derive(Debug, Clone)]
pub struct Frustum {
    /// Six planes defining the frustum (left, right, bottom, top, near, far).
    /// Each plane is stored as (a, b, c, d) where ax + by + cz + d = 0.
    pub planes: [Vec4; 6],
}

impl Frustum {
    /// Create a frustum from a view-projection matrix.
    /// Extracts the 6 frustum planes from a column-major view-projection matrix.
    pub fn from_matrix(view_proj: glam::Mat4) -> Self {
        let mut planes = [Vec4::ZERO; 6];

        // For glam's column-major matrices, we need to extract rows
        // glam provides row(index) method which gives us the i-th row
        let row0 = view_proj.row(0);
        let row1 = view_proj.row(1);
        let row2 = view_proj.row(2);
        let row3 = view_proj.row(3);

        // Extract frustum planes using Gribb-Hartmann method
        // Left plane: row3 + row0
        planes[0] = row3 + row0;

        // Right plane: row3 - row0
        planes[1] = row3 - row0;

        // Bottom plane: row3 + row1
        planes[2] = row3 + row1;

        // Top plane: row3 - row1
        planes[3] = row3 - row1;

        // Near plane: row3 + row2
        planes[4] = row3 + row2;

        // Far plane: row3 - row2
        planes[5] = row3 - row2;

        // Normalize planes (divide by length of normal vector)
        for plane in &mut planes {
            let length = Vec3::new(plane.x, plane.y, plane.z).length();
            if length > 0.0 {
                *plane /= length;
            }
        }

        Self { planes }
    }

    /// Test if an AABB intersects this frustum (is potentially visible).
    pub fn intersects_aabb(&self, aabb: &Aabb) -> bool {
        for plane in &self.planes {
            let normal = Vec3::new(plane.x, plane.y, plane.z);
            let d = plane.w;

            // Find the positive vertex (furthest along plane normal)
            let p_vertex = Vec3::new(
                if normal.x >= 0.0 { aabb.max.x } else { aabb.min.x },
                if normal.y >= 0.0 { aabb.max.y } else { aabb.min.y },
                if normal.z >= 0.0 { aabb.max.z } else { aabb.min.z },
            );

            // If positive vertex is outside this plane, AABB is completely outside frustum
            if normal.dot(p_vertex) + d < 0.0 {
                return false;
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_id_encoding() {
        let id = ChunkId::from_coords(10, -5, 127, 3);
        let (x, y, z, lod) = id.to_coords();
        assert_eq!((x, y, z, lod), (10, -5, 127, 3));
    }

    #[test]
    fn test_chunk_id_negative_coords() {
        let id = ChunkId::from_coords(-100, -200, -50, 2);
        let (x, y, z, lod) = id.to_coords();
        assert_eq!((x, y, z, lod), (-100, -200, -50, 2));
    }

    #[test]
    fn test_aabb_operations() {
        let aabb = Aabb::from_center_extents(Vec3::ZERO, Vec3::splat(10.0));
        assert_eq!(aabb.center(), Vec3::ZERO);
        assert!(aabb.contains_point(Vec3::new(5.0, 5.0, 5.0)));
        assert!(!aabb.contains_point(Vec3::new(15.0, 0.0, 0.0)));
    }

    #[test]
    fn test_aabb_intersection() {
        let aabb1 = Aabb::new(Vec3::ZERO, Vec3::splat(10.0));
        let aabb2 = Aabb::new(Vec3::splat(5.0), Vec3::splat(15.0));
        let aabb3 = Aabb::new(Vec3::splat(20.0), Vec3::splat(30.0));

        assert!(aabb1.intersects(&aabb2));
        assert!(!aabb1.intersects(&aabb3));
    }
}
