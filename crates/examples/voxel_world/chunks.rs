//! VoxelChunkManager — streaming chunk load/unload and face-culled mesh building.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use glam::Vec3;
use helio_render_v2::{GpuMesh, PackedVertex};

use stratum::{
    chunk_on_disk, ChunkCoord, Components, EntityId, Level,
    LevelStreamer, MaterialHandle, MeshHandle, StreamEvent, Transform,
    level_fs::format::ChunkFile,
};
use stratum_helio::AssetRegistry;

use crate::blocks::Block;
use crate::materials::MaterialPalette;
use crate::prefabs::PrefabLibrary;
use crate::terrain::*;
use crate::generation;
use crate::biomes::{self, Biome};

/// Max new chunk requests sent to the streamer per frame.
const MAX_REQUESTS_PER_FRAME: usize = 6;
/// Max completed chunk events processed (mesh uploads) per frame.
const MAX_UPLOADS_PER_FRAME: usize = 4;

// ── VoxelChunkManager ───────────────────────────────────────────────────────

pub struct VoxelChunkManager {
    dir:     PathBuf,
    use_fs:  bool,
    /// Chunks resident in the live `Level`: coord → (entity IDs, mesh handles).
    pub loaded:    HashMap<ChunkCoord, (Vec<EntityId>, Vec<MeshHandle>)>,
    pub in_flight: HashSet<ChunkCoord>,
    pub pending_ready: Vec<StreamEvent>,
    lib:           Arc<PrefabLibrary>,
}

impl VoxelChunkManager {
    pub fn new(dir: PathBuf, use_fs: bool) -> Self {
        Self {
            dir,
            use_fs,
            loaded:        HashMap::new(),
            in_flight:     HashSet::new(),
            pending_ready: Vec::new(),
            lib:           Arc::new(PrefabLibrary::new()),
        }
    }

    pub fn desired_set(&self, cam: Vec3) -> HashSet<ChunkCoord> {
        let cx = (cam.x / CHUNK_SIZE).floor() as i32;
        let cz = (cam.z / CHUNK_SIZE).floor() as i32;
        let mut set = HashSet::new();
        for dx in -LOAD_RADIUS..=LOAD_RADIUS {
            for dz in -LOAD_RADIUS..=LOAD_RADIUS {
                for y in 0..MAX_Y_CHUNKS {
                    set.insert(ChunkCoord::new(cx + dx, y, cz + dz));
                }
            }
        }
        set
    }

    pub fn update(
        &mut self,
        cam:      Vec3,
        level:    &mut Level,
        streamer: &LevelStreamer,
        assets:   &mut AssetRegistry,
    ) {
        let desired = self.desired_set(cam);

        // Evict chunks no longer in view
        let evict: Vec<ChunkCoord> = self.loaded.keys()
            .filter(|c| !desired.contains(c))
            .copied()
            .collect();
        for coord in evict {
            if let Some((ids, mesh_handles)) = self.loaded.remove(&coord) {
                for id in ids { level.despawn_entity(id); }
                for mh in mesh_handles { assets.remove(mh); }
            }
            level.partition_mut().remove_chunk(coord);
        }

        // Request new chunks sorted by distance
        let mut to_request: Vec<ChunkCoord> = desired.into_iter()
            .filter(|c| !self.loaded.contains_key(c) && !self.in_flight.contains(c))
            .collect();
        let (cx, cz) = (cam.x, cam.z);
        to_request.sort_unstable_by(|a, b| {
            let da = (a.x as f32 * CHUNK_SIZE - cx).powi(2)
                   + (a.z as f32 * CHUNK_SIZE - cz).powi(2);
            let db = (b.x as f32 * CHUNK_SIZE - cx).powi(2)
                   + (b.z as f32 * CHUNK_SIZE - cz).powi(2);
            da.partial_cmp(&db).unwrap()
        });

        for coord in to_request.into_iter().take(MAX_REQUESTS_PER_FRAME) {
            self.in_flight.insert(coord);
            if self.use_fs && chunk_on_disk(&self.dir, coord) {
                streamer.request_chunk(self.dir.clone(), coord);
            } else {
                let lib = Arc::clone(&self.lib);
                streamer.request_generate_transient(
                    coord,
                    Box::new(move |c| generation::build_chunk_file(c, &lib)),
                );
            }
        }
    }

    /// Accept newly arrived stream events into the pending queue.
    pub fn collect_events(&mut self, new_events: Vec<StreamEvent>) {
        self.pending_ready.extend(new_events);
    }

    /// Process up to `MAX_UPLOADS_PER_FRAME` pending events (GPU mesh uploads).
    pub fn flush_events(
        &mut self,
        level:   &mut Level,
        device:  &wgpu::Device,
        assets:  &mut AssetRegistry,
        palette: &MaterialPalette,
    ) {
        let take = self.pending_ready.len().min(MAX_UPLOADS_PER_FRAME);
        for event in self.pending_ready.drain(..take).collect::<Vec<_>>() {
            match event {
                StreamEvent::ChunkReady { coord, data } => {
                    self.in_flight.remove(&coord);
                    let submeshes = build_chunk_mesh(device, data, palette);
                    let cx = coord.x as f32 * CHUNK_SIZE + CHUNK_SIZE * 0.5;
                    let cz = coord.z as f32 * CHUNK_SIZE + CHUNK_SIZE * 0.5;
                    let chunk_centre = Vec3::new(cx, CHUNK_SIZE * 0.5, cz);

                    let mut ids = Vec::new();
                    let mut mesh_handles = Vec::new();

                    for (gpu_mesh, mat) in submeshes {
                        let mesh_h = assets.add(gpu_mesh);
                        mesh_handles.push(mesh_h);
                        ids.push(level.spawn_entity(
                            Components::new()
                                .with_transform(Transform::from_position(chunk_centre))
                                .with_mesh(mesh_h)
                                .with_material(mat)
                                .with_bounding_radius(CHUNK_SIZE * 2.5),
                        ));
                    }

                    level.partition_mut().get_or_create(coord).activate();
                    self.loaded.insert(coord, (ids, mesh_handles));
                }
                StreamEvent::ChunkError { coord, error } => {
                    self.in_flight.remove(&coord);
                    log::warn!("Chunk {:?}: {}", coord, error);
                }
            }
        }
    }
}

// ── Procedural terrain regeneration ──────────────────────────────────────────

/// Regenerate terrain voxels for a chunk from procedural noise functions.
/// This is called when loading a chunk to avoid storing terrain as entities.
fn regenerate_chunk_terrain(
    solid: &mut HashMap<(i32, i32, i32), Block>,
    coord: ChunkCoord,
) {
    let wy_min = coord.y * VOXELS_PER_CHUNK as i32;
    let wy_max = wy_min + VOXELS_PER_CHUNK as i32;
    let ox     = coord.x * VOXELS_PER_CHUNK as i32;
    let oz     = coord.z * VOXELS_PER_CHUNK as i32;

    if coord.y < 0 || coord.y >= MAX_Y_CHUNKS as i32 {
        return; // Out of bounds
    }

    for lx in 0..VOXELS_PER_CHUNK as i32 {
        for lz in 0..VOXELS_PER_CHUNK as i32 {
            let wx = ox + lx;
            let wz = oz + lz;
            let surface_h = terrain_height(wx, wz);
            let mf = mountain_factor(wx as f32, wz as f32);
            let biome = biomes::biome_at(wx, wz, mf);

            // Determine how high to fill (includes water above terrain)
            let fill_top = if surface_h < WATER_LEVEL && biome != Biome::Desert {
                WATER_LEVEL
            } else {
                surface_h
            };

            let y_lo = wy_min;
            let y_hi = wy_max.min(fill_top + 1);

            for wy in y_lo..y_hi {
                let block = if wy <= surface_h {
                    surface_block(wy, surface_h, biome)
                } else if biome == Biome::Taiga && wy == WATER_LEVEL {
                    Block::Ice
                } else {
                    Block::Water
                };

                solid.insert((wx, wy, wz), block);
            }
        }
    }
}

// ── Face-culled mesh builder ────────────────────────────────────────────────

fn build_chunk_mesh(
    device:  &wgpu::Device,
    chunk:   ChunkFile,
    palette: &MaterialPalette,
) -> Vec<(GpuMesh, MaterialHandle)> {
    // Collect solid voxels keyed by their integer min-corner.
    let mut solid: HashMap<(i32, i32, i32), Block> = HashMap::new();

    // ── Regenerate terrain procedurally ──────────────────────────────────────
    // Chunks only store placed structures/entities. Terrain is regenerated from
    // noise functions on load, eliminating ~95% of disk I/O.
    let coord = ChunkCoord {
        x: chunk.coord[0],
        y: chunk.coord[1],
        z: chunk.coord[2],
    };
    regenerate_chunk_terrain(&mut solid, coord);

    // ── Add placed entities from disk ────────────────────────────────────────
    for rec in &chunk.entities {
        if let (Some(t), Some(m)) = (&rec.transform, rec.material) {
            if let Some(block) = Block::from_mat_index(m) {
                solid.insert((
                    t.position[0].floor() as i32,
                    t.position[1].floor() as i32,
                    t.position[2].floor() as i32,
                ), block);
            }
        }
    }
    if solid.is_empty() { return vec![]; }

    // Face table (same winding as GpuMesh::cube).
    #[rustfmt::skip]
    const FACES: &[([f32;3], [i32;3], [[f32;3];4])] = &[
        ([0.,0., 1.], [0,0, 1], [[0.,0.,1.],[1.,0.,1.],[1.,1.,1.],[0.,1.,1.]]),
        ([0.,0.,-1.], [0,0,-1], [[1.,0.,0.],[0.,0.,0.],[0.,1.,0.],[1.,1.,0.]]),
        ([ 1.,0.,0.], [ 1,0,0], [[1.,0.,1.],[1.,0.,0.],[1.,1.,0.],[1.,1.,1.]]),
        ([-1.,0.,0.], [-1,0,0], [[0.,0.,0.],[0.,0.,1.],[0.,1.,1.],[0.,1.,0.]]),
        ([0., 1.,0.], [0, 1,0], [[0.,1.,1.],[1.,1.,1.],[1.,1.,0.],[0.,1.,0.]]),
        ([0.,-1.,0.], [0,-1,0], [[0.,0.,0.],[1.,0.,0.],[1.,0.,1.],[0.,0.,1.]]),
    ];
    const UVS: [[f32; 2]; 4] = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]];

    let mut verts: HashMap<Block, Vec<PackedVertex>> = HashMap::new();
    let mut idxs:  HashMap<Block, Vec<u32>>          = HashMap::new();

    for (&(bx, by, bz), &block) in &solid {
        let v  = verts.entry(block).or_default();
        let ix = idxs.entry(block).or_default();

        for (normal, nb, corners) in FACES {
            if solid.contains_key(&(bx + nb[0], by + nb[1], bz + nb[2])) { continue; }

            let base_vert = v.len() as u32;
            let c0 = corners[0];
            let c1 = corners[1];
            let td = [c1[0] - c0[0], c1[1] - c0[1], c1[2] - c0[2]];
            let tl = (td[0] * td[0] + td[1] * td[1] + td[2] * td[2]).sqrt().max(1e-8);
            let tangent = [td[0] / tl, td[1] / tl, td[2] / tl];

            for (ci, corner) in corners.iter().enumerate() {
                v.push(PackedVertex::new_with_tangent(
                    [bx as f32 + corner[0], by as f32 + corner[1], bz as f32 + corner[2]],
                    *normal,
                    UVS[ci],
                    tangent,
                ));
            }
            ix.extend_from_slice(&[
                base_vert, base_vert + 1, base_vert + 2,
                base_vert, base_vert + 2, base_vert + 3,
            ]);
        }
    }

    let mut result = Vec::new();
    for (block, v) in verts {
        if v.is_empty() { continue; }
        let ix = idxs.remove(&block).unwrap_or_default();
        result.push((GpuMesh::new(device, &v, &ix), palette.handle_for(block)));
    }
    result
}
