//! Simple cube-based meshing for voxel chunks.
//! Generates meshes from (0,0,0) to (16,16,16) in local space.

use super::blocks::Block;
use super::voxel_chunk::{VoxelChunk, CHUNK_SIZE};

/// Vertex data for a voxel face.
#[derive(Clone, Copy, Debug)]
pub struct VoxelVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub material_id: u32,
}

/// Standard cube face vertices.
/// Returns 4 vertices in the order: [v0, v1, v2, v3] for indices [0,1,2,0,2,3]
fn get_cube_face_vertices(face_index: usize, pos: [f32; 3], size: f32) -> [[f32; 3]; 4] {
    let half = size * 0.5;
    let [x, y, z] = pos;

    match face_index {
        0 => {
            // +X face (right) - normal points in +X direction
            // Bottom-left, bottom-right, top-right, top-left (CCW from outside)
            [[x + half, y - half, z - half],  // 0: bottom-near
             [x + half, y - half, z + half],  // 1: bottom-far
             [x + half, y + half, z + half],  // 2: top-far
             [x + half, y + half, z - half]]  // 3: top-near
        }
        1 => {
            // -X face (left) - normal points in -X direction
            [[x - half, y - half, z + half],  // 0: bottom-far (from -X view, this is near)
             [x - half, y - half, z - half],  // 1: bottom-near (from -X view, this is far)
             [x - half, y + half, z - half],  // 2: top-near
             [x - half, y + half, z + half]]  // 3: top-far
        }
        2 => {
            // +Y face (top) - normal points in +Y direction
            [[x - half, y + half, z - half],  // 0: near-left
             [x + half, y + half, z - half],  // 1: near-right
             [x + half, y + half, z + half],  // 2: far-right
             [x - half, y + half, z + half]]  // 3: far-left
        }
        3 => {
            // -Y face (bottom) - normal points in -Y direction
            [[x - half, y - half, z - half],  // 0
             [x - half, y - half, z + half],  // 1
             [x + half, y - half, z + half],  // 2
             [x + half, y - half, z - half]]  // 3
        }
        4 => {
            // +Z face (front) - normal points in +Z direction
            [[x - half, y - half, z + half],  // 0: bottom-left
             [x + half, y - half, z + half],  // 1: bottom-right
             [x + half, y + half, z + half],  // 2: top-right
             [x - half, y + half, z + half]]  // 3: top-left
        }
        5 => {
            // -Z face (back) - normal points in -Z direction
            [[x + half, y - half, z - half],  // 0
             [x - half, y - half, z - half],  // 1
             [x - half, y + half, z - half],  // 2
             [x + half, y + half, z - half]]  // 3
        }
        _ => unreachable!(),
    }
}

fn get_cube_face_normal(face_index: usize) -> [f32; 3] {
    match face_index {
        0 => [1.0, 0.0, 0.0],   // +X
        1 => [-1.0, 0.0, 0.0],  // -X
        2 => [0.0, 1.0, 0.0],   // +Y
        3 => [0.0, -1.0, 0.0],  // -Y
        4 => [0.0, 0.0, 1.0],   // +Z
        5 => [0.0, 0.0, -1.0],  // -Z
        _ => unreachable!(),
    }
}

/// Generate a cube mesh for the chunk.
/// Chunk spans from (0,0,0) to (16,16,16) in local space.
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

                // Position in chunk local space (0-16 range)
                let local_pos = [
                    x as f32 + 0.5,
                    y as f32 + 0.5,
                    z as f32 + 0.5,
                ];

                // Check each face for culling (6 faces)
                for face_idx in 0..6 {
                    let should_draw = match face_idx {
                        0 => chunk.get(x + 1, y, z).is_transparent() || chunk.get(x + 1, y, z).is_air(), // +X
                        1 => if x > 0 { chunk.get(x - 1, y, z).is_transparent() || chunk.get(x - 1, y, z).is_air() } else { true }, // -X
                        2 => chunk.get(x, y + 1, z).is_transparent() || chunk.get(x, y + 1, z).is_air(), // +Y
                        3 => if y > 0 { chunk.get(x, y - 1, z).is_transparent() || chunk.get(x, y - 1, z).is_air() } else { true }, // -Y
                        4 => chunk.get(x, y, z + 1).is_transparent() || chunk.get(x, y, z + 1).is_air(), // +Z
                        5 => if z > 0 { chunk.get(x, y, z - 1).is_transparent() || chunk.get(x, y, z - 1).is_air() } else { true }, // -Z
                        _ => unreachable!(),
                    };

                    if should_draw {
                        add_cube_face(
                            &mut vertices,
                            &mut indices,
                            face_idx,
                            local_pos,
                            1.0,
                            material_id,
                        );
                    }
                }
            }
        }
    }

    (vertices, indices)
}

/// Add a single cube face to the mesh.
fn add_cube_face(
    vertices: &mut Vec<VoxelVertex>,
    indices: &mut Vec<u32>,
    face_index: usize,
    pos: [f32; 3],
    size: f32,
    material_id: u32,
) {
    let base_index = vertices.len() as u32;
    let positions = get_cube_face_vertices(face_index, pos, size);
    let normal = get_cube_face_normal(face_index);

    // UV coordinates (same for all faces)
    let uvs = [
        [0.0, 1.0], // Bottom-left
        [1.0, 1.0], // Bottom-right
        [1.0, 0.0], // Top-right
        [0.0, 0.0], // Top-left
    ];

    // Add 4 vertices for this quad
    for i in 0..4 {
        vertices.push(VoxelVertex {
            position: positions[i],
            normal,
            uv: uvs[i],
            material_id,
        });
    }

    // Add 6 indices for 2 triangles (counter-clockwise)
    indices.extend_from_slice(&[
        base_index,
        base_index + 1,
        base_index + 2,
        base_index,
        base_index + 2,
        base_index + 3,
    ]);
}
