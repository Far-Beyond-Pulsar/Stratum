# World Partition Streaming: Best-of-Both Pragmatic Plan (Helio)

## 1. Objective

Implement UE5.7-style 3D world partition streaming in Helio with a pragmatic architecture:
- Core renderer (`crates/helio`) provides optimized hooks and low-level chunk visibility helpers.
- Wrapper crate manages world, streaming policy, async IO, LOD and scheduling.
- Framerate-independent, budgeted background load/unload.


## 2. Rationale: Why "best of both"?

- `helio` is already highly-efficient on the GPU path and supports O(1) persistent scene updates.
- World management and load rules are game-specific; best kept in a higher-level crate (`helio-world` / app layer).
- Core changes remain minimal and stable; behavior extends mechanics in wrapper with small API tweaks.


## 3. Existing Helio features to leverage

### 3.1 Scene core

- `Scene::insert_object`, `remove_object`, `update_object_transform` are O(1)
- persistent vs optimized mode (`objects_layout_optimized`) with via `Scene::optimize_scene_layout`
- `Scene::flush()` reachable cheap path; `objects_dirty`, `vg_objects_dirty`
- `group_hidden` mask and `SceneActorTrait` for custom actors

### 3.2 Resource handles

- `MeshPool`, `SparsePool<TextureRecord>`, `SparsePool<MaterialRecord>`, `DenseArena<ObjectRecord>`
- `TextureId`, `MaterialId`, `ObjectId`, `VirtualMeshId`, `VirtualObjectId` etc

### 3.3 Virtual geometry

- `vg_meshes`, `vg_objects`, `vg_cpumeshlets`, `vg_cpu_instances` exist
- `Scene::rebuild_vg_buffers` path is similar to world chunk geometry rebind


## 4. Proposed API extensions in `crates/helio`

### 4.1 New module: `scene/partition.rs`

- `pub struct ChunkId(u64)`
- `pub struct WorldChunkMetadata { aabb: Aabb, center: Vec3, lod: u8, group: GroupId, ... }`
- `pub enum ChunkState { Unloaded, Loading, Loaded, Visible, Hidden, Evicting }

### 4.2 Scene methods

- `fn insert_chunk(&mut self, chunk_id: ChunkId, object_ids: &[ObjectId], metadata: &WorldChunkMetadata)`
- `fn remove_chunk(&mut self, chunk_id: ChunkId)`
- `fn set_chunk_visible(&mut self, chunk_id: ChunkId, visible: bool)` (calls hide groups)
- `fn chunk_info(&self, chunk_id: ChunkId) -> Option<ChunkInfo>`

### 4.3 Group and chunk mapping

- Add `chunk->group_id` table in `Scene`.
- Add `group_id` to `ObjectDescriptor` (to allow group-level culling/hide). `GroupMask` already exists.
- Helper: `Scene::hide_group`, `Scene::show_group` can operate chunk-level.

### 4.4 Exporting internals for wrapper

- `fn collect_object_bounds(&self) -> impl Iterator<Item=(ObjectId, Aabb)>` for scheduler queries.
- `fn is_group_loaded(&self, group_id: GroupId) -> bool` for quick state.


## 5. Wrapper crate: `helio-world` (recommended)

### 5.1 Core components

- `struct WorldPartitionManager` (owns world metadata and chunk states)
- `struct Chunk` (ID, AABB, cell index, state, object references, loading priority)
- `struct StreamingBudget { max_io_ms: Duration, max_upload_bytes: usize, max_tasks: usize }`
- `struct ChunkLoadTask` (source path, chunk id, load state, LOD target)

### 5.2 Per-frame update chain

1. `update_camera(camera: &Camera)` updates world partition state and predicted viewer velocity.
2. `evaluate_lod_sets()` produces `required_chunks`, `preload_chunks`, `evict_chunks`.
3. `schedule_tasks()` enqueues chunk load/unload jobs using budget.
4. `process_io_workers()` does async decode in background threads.
5. `commit_ready_chunks(scene: &mut Scene)` inserts into `scene` with stable handles.
6. `flush scene` and render.
7. `cleanup_evicted` after frame.

### 5.3 Dispatch and async model

- Dedicated threadpool or `tokio` tasks for file decompression.
- `ChunkLoadResult` staging queue (MPSC). commit on main thread.
- Track `load_frame_age` for timeouts & starvation.
- `progress_limiter` with token bucket to keep load independent of `FPS`.

### 5.4 Visibility/LOD selection

- Frustum query first (CPU): use chunk AABB + camera frustum.
- Metric: `screen_size = distance / max(WorldChunkExtent)`; choose LOD threshold.
- Hysteresis ring: `near_radius`, `preload_radius`, `unload_radius`.
- Keep partly visible chunks in a prioritized "transient" list to avoid pop-in.

### 5.5 Chunk commit / eviction semantics

1. On chunk loaded:
   - create or reuse mesh/material/texture handles in `Scene`.
   - insert objects with `object_descriptor.groups` set to chunk group.
   - mark `chunk.state = ChunkState::Loaded`.

2. On chunk become visible:
   - `scene.set_chunk_visible(chunk_id, true)` / `scene.show_group(chunk_group)`.
   - optionally call `scene.update_object_transform` for animated objects.

3. On chunk hide/unload:
   - `scene.set_chunk_visible(chunk_id, false)` hides quickly via group mask.
   - if eviction condition met, remove objects and resources from scene:
     - `scene.remove_object(object_id)` in batch.
     - `scene.remove_mesh(mesh_id)` / `scene.remove_material(material_id)` once refcount reaches 0.
   - release memory in wrapping crate and possibly keep compressed chunk cache.

4. Asynchronous flush:
   - avoid calling `scene.flush()` on each chunk event; do once per frame.
   - for high-rate streaming, use frame budget `max_chunk_commits_per_frame`.

### 5.6 Metrics and telemetry (must-have)

- `WorldPartitionMetrics { chunks_loaded, chunks_evicted, pending_io_tasks, avg_commit_ms, max_upload_bytes_per_frame }`
- Track `chunk_load_latency` (request->GPU-ready) and `chunk_failures`.
- Expose debug overlay with:
  - chunk grid + state color, culling frustum, camera predicted path.
  - CPU ms for update/eval/schedule/io/commit versus available T.


## 6. Implementation milestones

1. Starter PR in `crates/helio`:
   - Add `ChunkId`, `WorldChunkMetadata`, `chunk group` in `ObjectDescriptor`.
   - Add `Scene::set_chunk_visible`, `Scene::remove_chunk`, `Scene::collect_object_bounds`.
   - Unit tests for group hide/show and chunk transitions.

2. Wrapper PR `crates/helio-world` (or game crate):
   - implement `WorldPartitionManager` full state machine.
   - wire in `update(camera)`, `tick(dt)`.
   - scheduling + worker thread pool + result queue + commit.
   - test harness with synthetic grid.

3. Integration demo PR in `crates/examples`:
   - add `streaming_outdoor_city.rs`.
   - use baked chunk files with 100+ chunk objects.
   - measure sustained 120Hz with 1ms budget.

4. Optimization PR:
   - GPU cluster culling aligned with existing Helio pass culling.
   - batch chunk load/path building to avoid per-object updates.
   - optional `Scene::optimize_scene_layout` at commit to aggregate static objects.

5. Finalize with docs and migration guide.


## 7. Footguns and known traps

- **Chunk-state drift**: never hold `Scene` object IDs after removing from scene unless guaranteed alive. Keep mapping in wrapper only.
- **GroupID overflow**: `GroupMask` may be limited (64 bits?). track with `GroupIdAllocator` and refuse over-allocation.
- **GPU buffer thrashing**: avoid calling `scene.optimize_scene_layout` too often during streaming; do it at stable points.
- **Blocking on IO**: decode + mesh upload must not block main frame. Do not deserialize and push to `Scene` directly from worker thread.
- **Race conditions**: chunk load full pipeline is async. any `ChunkId` state change in worker and main must be atomic and cross-thread-safe (mutex or `Atomic` state machine).
- **LOD ping-pong**: make extreme min time thresholds to avoid constant LOD swap in border areas.
- **Asset churn**: do not keep too many chunks loaded, or memory and upload driver will spike.
- **Resource refcount bugs**: if multiple chunks share mesh/material, ensure `Scene` reference count is per-resource, not per-chunk.


## 8. Testing matrix

- unit tests for `Scene`:
  - `test_chunk_visibility_toggle` with group hide states.
  - `test_chunk_insertion_removal` with 1k objects.
  - `test_scene_flush_no_header` for no state changes.

- wrapper tests:
  - `test_worldpartition_expand_contract` with camera path.
  - `test_budget_respected` with synthetic load tasks and timers.
  - `test_async_load_cancel` if chunk becomes not needed mid-load.

- integration:
  - traverse Peers salad 1GB world at 30fps, check load/unload counts.
  - Fuzz `ChunkState` transitions.


## 9. Execution recommendation

- Start in wrapper crate first; avoid heavy core dev.
- Keep `helio` surface minimal (chunk hints + group helper).
- If proof-of-concept is stable, migrate minimal world partition datatypes into `helio::world` submodule for reuse.


## 10. Summary

The plan is to implement a high-performance, framerate-independent world partition pipeline with least risk:
- maintain Helio renderer path performance
- wrapper layer controls policy + async mesh IO
- core hook points in `scene` for group-based chunk states
- strict budgets and metrics to avoid frame stalls

This can be given to an implementation-focused LLM and code-first engineer as a complete spec.
