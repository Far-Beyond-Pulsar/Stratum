//! Face-culled voxel mesh generation.

use super::blocks::Block;
use super::voxel_chunk::{VoxelChunk, CHUNK_SIZE};
use glam::Vec3;

/// Vertex data for a voxel face.
#[derive(Clone, Copy, Debug)]
pub struct VoxelVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub material_id: u32,
}

/// Face direction for culling.
#[derive(Clone, Copy, Debug)]
enum Face {
    PosX, // Right
    NegX, // Left
    PosY, // Top
    NegY, // Bottom
    PosZ, // Front
    NegZ, // Back
}

impl Face {
    fn normal(self) -> [f32; 3] {
        match self {
            Face::PosX => [1.0, 0.0, 0.0],
            Face::NegX => [-1.0, 0.0, 0.0],
            Face::PosY => [0.0, 1.0, 0.0],
            Face::NegY => [0.0, -1.0, 0.0],
            Face::PosZ => [0.0, 0.0, 1.0],
            Face::NegZ => [0.0, 0.0, -1.0],
        }
    }

    /// Get vertices for this face (4 corners, counter-clockwise when looking at face).
    fn vertices(self, x: f32, y: f32, z: f32) -> [[f32; 3]; 4] {
        match self {
            Face::PosX => [
                // Right face (+X)
                [x + 1.0, y, z],
                [x + 1.0, y + 1.0, z],
                [x + 1.0, y + 1.0, z + 1.0],
                [x + 1.0, y, z + 1.0],
            ],
            Face::NegX => [
                // Left face (-X)
                [x, y, z + 1.0],
                [x, y + 1.0, z + 1.0],
                [x, y + 1.0, z],
                [x, y, z],
            ],
            Face::PosY => [
                // Top face (+Y)
                [x, y + 1.0, z],
                [x, y + 1.0, z + 1.0],
                [x + 1.0, y + 1.0, z + 1.0],
                [x + 1.0, y + 1.0, z],
            ],
            Face::NegY => [
                // Bottom face (-Y)
                [x, y, z],
                [x + 1.0, y, z],
                [x + 1.0, y, z + 1.0],
                [x, y, z + 1.0],
            ],
            Face::PosZ => [
                // Front face (+Z)
                [x, y, z + 1.0],
                [x, y + 1.0, z + 1.0],
                [x + 1.0, y + 1.0, z + 1.0],
                [x + 1.0, y, z + 1.0],
            ],
            Face::NegZ => [
                // Back face (-Z)
                [x + 1.0, y, z],
                [x + 1.0, y + 1.0, z],
                [x, y + 1.0, z],
                [x, y, z],
            ],
        }
    }

    /// UV coordinates for this face (same for all faces).
    fn uvs(self) -> [[f32; 2]; 4] {
        [
            [0.0, 1.0], // Bottom-left
            [0.0, 0.0], // Top-left
            [1.0, 0.0], // Top-right
            [1.0, 1.0], // Bottom-right
        ]
    }
}

/// Generate a face-culled mesh for a voxel chunk.
/// Returns (vertices, indices).
pub fn generate_chunk_mesh(chunk: &VoxelChunk) -> (Vec<VoxelVertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    for x in 0..CHUNK_SIZE {
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                let block = chunk.get(x, y, z);

                // Skip air blocks
                if block.is_air() {
                    continue;
                }

                let material_id = block as u32;

                // Check each face for occlusion
                let faces_to_draw = [
                    (Face::PosX, chunk.get(x + 1, y, z)),
                    (Face::NegX, if x > 0 { chunk.get(x - 1, y, z) } else { Block::Air }),
                    (Face::PosY, chunk.get(x, y + 1, z)),
                    (Face::NegY, if y > 0 { chunk.get(x, y - 1, z) } else { Block::Air }),
                    (Face::PosZ, chunk.get(x, y, z + 1)),
                    (Face::NegZ, if z > 0 { chunk.get(x, y, z - 1) } else { Block::Air }),
                ];

                for (face, neighbor) in faces_to_draw {
                    // Draw face if neighbor is transparent or air
                    if neighbor.is_transparent() || neighbor.is_air() {
                        add_face(
                            &mut vertices,
                            &mut indices,
                            face,
                            x as f32,
                            y as f32,
                            z as f32,
                            material_id,
                        );
                    }
                }
            }
        }
    }

    (vertices, indices)
}

/// Add a single face to the mesh.
fn add_face(
    vertices: &mut Vec<VoxelVertex>,
    indices: &mut Vec<u32>,
    face: Face,
    x: f32,
    y: f32,
    z: f32,
    material_id: u32,
) {
    let base_index = vertices.len() as u32;
    let positions = face.vertices(x, y, z);
    let uvs = face.uvs();
    let normal = face.normal();

    // Add 4 vertices for this quad
    for i in 0..4 {
        vertices.push(VoxelVertex {
            position: positions[i],
            normal,
            uv: uvs[i],
            material_id,
        });
    }

    // Add 6 indices for 2 triangles (quad)
    indices.extend_from_slice(&[
        base_index,
        base_index + 1,
        base_index + 2,
        base_index,
        base_index + 2,
        base_index + 3,
    ]);
}
