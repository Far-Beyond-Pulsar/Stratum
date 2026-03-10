//! `HelioIntegration` — Stratum-to-Helio render submission.
//!
//! `HelioIntegration` wraps a Helio `Renderer` and an `AssetRegistry`.
//! Each frame, the host calls `submit_frame()` with the `Vec<RenderView>`
//! produced by `Stratum::build_views()`. The integration:
//!
//! 1. Syncs the persistent mesh proxy registry: adds new entities via `add_object`,
//!    removes gone entities via `remove_object`, updates transforms via
//!    `update_transform`.
//! 2. Syncs the persistent light registry: adds new light entities via `add_light`,
//!    removes gone entities via `remove_light`, updates moved/changed lights via
//!    `update_light`. Zero GPU cost when lights are static.
//! 3. Syncs the persistent billboard registry for entity-spawned billboards.
//! 4. Sets sky atmosphere and skylight only when they change (dirty-checked).
//! 5. Builds a Helio `Camera` from each view matrix.
//! 6. Calls `renderer.render()` per view.
//!
//! ## Render target resolution
//!
//! * `RenderTargetHandle::PrimarySurface` → the `wgpu::TextureView` passed
//!   in directly by the caller (the swapchain image acquired each frame).
//! * `RenderTargetHandle::OffscreenTexture(name)` → a `wgpu::TextureView`
//!   registered via `register_offscreen_view`. Falls back to primary surface
//!   if the name is unknown.
//! * `ViewportSlot` → falls back to primary surface.

use std::collections::HashMap;

use glam::{Quat, Vec3};
use stratum::{ChunkState, EntityStore, Level, LightData, RenderTargetHandle, RenderView, WorldPartition};
use stratum::entity::EntityId;
use stratum::{SkyAtmosphereData, SkylightData};
use helio_render_v2::{ObjectId, Renderer};
use helio_render_v2::scene::{LightId, BillboardId};
use helio_render_v2::features::BillboardInstance;

use crate::asset_registry::AssetRegistry;
use crate::bridge::{
    render_view_to_camera,
    stratum_light_to_scene_light,
    stratum_sky_atmosphere_to_helio,
    stratum_skylight_to_helio,
};

/// Owns the Helio renderer and the mesh asset registry, and drives render
/// submission for each frame.
pub struct HelioIntegration {
    renderer:         Renderer,
    assets:           AssetRegistry,
    /// Persistent per-entity Helio object IDs. Populated by `sync_entity_objects`;
    /// entries survive across frames for as long as the entity exists in the level.
    entity_objects:   HashMap<EntityId, ObjectId>,
    /// Persistent per-entity light IDs. Added/removed/updated by `sync_lights_and_sky`.
    light_objects:    HashMap<EntityId, LightId>,
    /// Persistent per-entity billboard IDs (entity-spawned only).
    billboard_objects: HashMap<EntityId, BillboardId>,
    /// Persistent billboard IDs for externally-injected extra billboards
    /// (e.g., RC probe grid visualization). Managed by `set_extra_billboards`.
    extra_billboard_ids: Vec<BillboardId>,
    /// Named offscreen render targets. Populated by the host when
    /// `RenderTargetHandle::OffscreenTexture` cameras are in use.
    offscreen_views:  HashMap<String, wgpu::TextureView>,
    /// Last sky atmosphere uploaded — used to skip redundant `set_sky_atmosphere`
    /// calls (which re-render the expensive sky LUT).
    cached_sky_atm:   Option<SkyAtmosphereData>,
    /// Last skylight uploaded — used to skip redundant `set_skylight` calls.
    cached_skylight:  Option<SkylightData>,
}

impl HelioIntegration {
    pub fn new(renderer: Renderer, assets: AssetRegistry) -> Self {
        Self {
            renderer,
            assets,
            entity_objects:       HashMap::new(),
            light_objects:        HashMap::new(),
            billboard_objects:    HashMap::new(),
            extra_billboard_ids:  Vec::new(),
            offscreen_views:      HashMap::new(),
            cached_sky_atm:       None,
            cached_skylight:      None,
        }
    }

    // ── Accessors ─────────────────────────────────────────────────────────────

    pub fn renderer    (&self)     -> &Renderer         { &self.renderer }
    pub fn renderer_mut(&mut self) -> &mut Renderer     { &mut self.renderer }
    pub fn assets      (&self)     -> &AssetRegistry    { &self.assets }
    pub fn assets_mut  (&mut self) -> &mut AssetRegistry { &mut self.assets }

    // ── Material creation ─────────────────────────────────────────────────────

    /// Create a GPU material from a `helio_render_v2::Material` descriptor and
    /// return its `GpuMaterial`. The result can be stored in the `AssetRegistry`
    /// via `assets_mut().add_material(mat)` to obtain a `MaterialHandle`.
    pub fn create_material(&mut self, material: &helio_render_v2::Material) -> helio_render_v2::GpuMaterial {
        self.renderer.create_material(material)
    }

    // ── Offscreen texture registry ────────────────────────────────────────────

    /// Register a named offscreen `TextureView` as a render target.
    ///
    /// Cameras whose `render_target` is `RenderTargetHandle::OffscreenTexture(name)`
    /// will render to this view. Overwrites any previous registration for `name`.
    pub fn register_offscreen_view(&mut self, name: impl Into<String>, view: wgpu::TextureView) {
        self.offscreen_views.insert(name.into(), view);
    }

    /// Remove a named offscreen view. The contained `TextureView` is dropped.
    pub fn unregister_offscreen_view(&mut self, name: &str) {
        self.offscreen_views.remove(name);
    }

    // ── Extra billboards ──────────────────────────────────────────────────────

    /// Set extra billboards that will be rendered alongside entity-spawned
    /// billboards. Useful for debug overlays like RC probe grids.
    ///
    /// Uses the persistent `add_billboard` / `remove_billboard` API so there
    /// is no per-frame Vec allocation. On subsequent calls with the same
    /// count, billboard positions are updated in-place (O(N) but no alloc).
    pub fn set_extra_billboards(&mut self, billboards: Vec<BillboardInstance>) {
        let new_count = billboards.len();
        let old_count = self.extra_billboard_ids.len();

        if new_count < old_count {
            // Remove excess.
            for id in self.extra_billboard_ids.drain(new_count..) {
                self.renderer.remove_billboard(id);
            }
        } else if new_count > old_count {
            // Add new entries.
            for instance in &billboards[old_count..] {
                let id = self.renderer.add_billboard(instance.clone());
                self.extra_billboard_ids.push(id);
            }
        }

        // Update positions/colors for the common prefix.
        for (id, instance) in self.extra_billboard_ids.iter().zip(billboards.iter()) {
            self.renderer.update_billboard(*id, instance.clone());
        }
    }

    /// Remove all extra billboards previously registered via `set_extra_billboards`.
    pub fn clear_extra_billboards(&mut self) {
        for id in self.extra_billboard_ids.drain(..) {
            self.renderer.remove_billboard(id);
        }
    }
    // ── Object registry sync ──────────────────────────────────────────────────────

    /// Synchronise the Helio persistent proxy registry against the current
    /// level's **active** chunk set.
    ///
    /// Only entities whose chunk is in `ChunkState::Active` are registered;
    /// entities in deactivated / unloaded chunks are removed from the GPU
    /// proxy registry even though they remain in the `EntityStore`.  This
    /// mirrors exactly what `build_render_views` does for geometry candidates.
    ///
    /// ## Strategy
    ///
    /// Objects are registered with Helio **once** and kept resident in GPU
    /// memory for their entire lifetime — regardless of chunk streaming state.
    /// Chunk activation/deactivation only flips the proxy's `enabled` flag
    /// via `enable_object` / `disable_object`, which costs a single `bool`
    /// write with zero GPU allocation.  This is the key advantage of the
    /// persistent proxy API: streaming causes no GPU buffer churn.
    ///
    /// Only genuine despawns trigger `remove_object`.
    ///
    /// * First seen (any state)  → `add_object` (disabled if chunk inactive).
    /// * Chunk activates         → `enable_object`.
    /// * Chunk deactivates       → `disable_object`.
    /// * Entity despawned        → `remove_object`.
    /// * Transform changed       → `update_transform` (zero-cost when unchanged).
    pub fn sync_entity_objects(&mut self, level: &Level) {
        let store = level.entities();

        // Build the currently-active set once — O(active entities).
        let active_ids: std::collections::HashSet<EntityId> =
            level.partition().active_entities().into_iter().collect();

        // ── Remove proxies for truly despawned entities ───────────────────────
        // Triggered only when the entity is gone from the store or lost its mesh.
        // Chunk deactivation does NOT remove — it only disables.
        let despawned: Vec<EntityId> = self.entity_objects.keys()
            .filter(|&&id| store.get(id).map_or(true, |c| c.mesh.is_none()))
            .copied()
            .collect();
        for id in despawned {
            if let Some(obj_id) = self.entity_objects.remove(&id) {
                self.renderer.remove_object(obj_id);
            }
        }

        // ── Register any not-yet-seen mesh entity (active or inactive) ────────
        for (entity_id, components) in store.iter() {
            let Some(mesh_handle) = components.mesh else { continue };
            if self.entity_objects.contains_key(&entity_id) { continue }

            if let Some(gpu_mesh) = self.assets.get(mesh_handle) {
                let material = components.material
                    .and_then(|mh| self.assets.get_material(mh));
                let transform = components.transform.as_ref()
                    .map(|t| glam::Mat4::from_scale_rotation_translation(t.scale, t.rotation, t.position))
                    .unwrap_or(glam::Mat4::IDENTITY);
                let obj_id = self.renderer.add_object(gpu_mesh, material, transform);
                // Supply a bounding sphere so the renderer can frustum-cull this object.
                if components.bounding_radius > 0.0 {
                    self.renderer.set_object_bounds(obj_id, components.bounding_radius);
                }
                // Register disabled if the entity's chunk isn't active yet.
                if !active_ids.contains(&entity_id) {
                    self.renderer.disable_object(obj_id);
                }
                self.entity_objects.insert(entity_id, obj_id);
            } else {
                log::warn!(
                    "Entity {:?} references unregistered MeshHandle({:?}) — skipped",
                    entity_id, mesh_handle
                );
            }
        }

        // ── Enable / disable proxies to match current chunk activation ────────
        // Also update transforms for all enabled (active) objects.
        for (entity_id, &obj_id) in &self.entity_objects {
            let is_active = active_ids.contains(entity_id);
            let currently_enabled = self.renderer.is_object_enabled(obj_id);

            if is_active && !currently_enabled {
                self.renderer.enable_object(obj_id);
            } else if !is_active && currently_enabled {
                self.renderer.disable_object(obj_id);
            }

            // Update transform for active objects only (no-op when matrix unchanged).
            if is_active {
                if let Some(components) = store.get(*entity_id) {
                    let transform = components.transform.as_ref()
                        .map(|t| glam::Mat4::from_scale_rotation_translation(t.scale, t.rotation, t.position))
                        .unwrap_or(glam::Mat4::IDENTITY);
                    self.renderer.update_transform(obj_id, transform);
                }
            }
        }
    }
    // ── Persistent light + sky sync ───────────────────────────────────────────

    /// Synchronise persistent light and billboard proxies, and update sky /
    /// atmosphere state, against the current level's entity store.
    ///
    /// Called once per frame in `submit_frame`, before any per-view render.
    ///
    /// ## Lights
    ///
    /// Scans **all** entities (not just visible ones) so shadow-casting lights
    /// outside the view frustum are still registered. Uses a persistent
    /// `EntityId → LightId` map:
    ///
    /// * New light entity    → `add_light`
    /// * Light entity gone   → `remove_light`
    /// * Light entity present → `update_light` (cheap; GPU upload only when data changed)
    ///
    /// ## Entity billboards
    ///
    /// Same persistent-proxy pattern via `EntityId → BillboardId`.
    ///
    /// ## Sky atmosphere / skylight
    ///
    /// First entity carrying `sky_atmosphere` / `skylight` components wins.
    /// `set_sky_atmosphere` / `set_skylight` are called **only when the data
    /// changes**, avoiding expensive sky-LUT re-renders at steady state.
    fn sync_lights_and_sky(&mut self, level: &Level) {
        let store = level.entities();

        // ── Lights ────────────────────────────────────────────────────────────

        // Collect all current light entities from the store (active or not).
        let mut current_lights: HashMap<EntityId, helio_render_v2::SceneLight> = HashMap::new();
        for (id, c) in store.iter() {
            let Some(light) = &c.light else { continue };
            let Some(tf) = &c.transform else { continue };
            current_lights.insert(id, stratum_light_to_scene_light(light, tf.position.to_array()));
        }

        // Remove lights for despawned entities.
        let removed: Vec<EntityId> = self.light_objects.keys()
            .filter(|id| !current_lights.contains_key(*id))
            .copied()
            .collect();
        for id in removed {
            if let Some(lid) = self.light_objects.remove(&id) {
                self.renderer.remove_light(lid);
            }
        }

        // Add new lights; update existing ones.
        for (id, scene_light) in current_lights {
            if let Some(&lid) = self.light_objects.get(&id) {
                self.renderer.update_light(lid, scene_light);
            } else {
                let lid = self.renderer.add_light(scene_light);
                self.light_objects.insert(id, lid);
            }
        }

        // ── Entity billboards ─────────────────────────────────────────────────

        let mut current_bb: HashMap<EntityId, BillboardInstance> = HashMap::new();
        for (id, c) in store.iter() {
            let Some(bb) = &c.billboard else { continue };
            let Some(tf) = &c.transform else { continue };
            current_bb.insert(id,
                BillboardInstance::new(tf.position.to_array(), bb.size)
                    .with_color(bb.color)
                    .with_screen_scale(bb.screen_scale),
            );
        }

        let removed_bb: Vec<EntityId> = self.billboard_objects.keys()
            .filter(|id| !current_bb.contains_key(*id))
            .copied()
            .collect();
        for id in removed_bb {
            if let Some(bid) = self.billboard_objects.remove(&id) {
                self.renderer.remove_billboard(bid);
            }
        }

        for (id, instance) in current_bb {
            if let Some(&bid) = self.billboard_objects.get(&id) {
                self.renderer.update_billboard(bid, instance);
            } else {
                let bid = self.renderer.add_billboard(instance);
                self.billboard_objects.insert(id, bid);
            }
        }

        // ── Sky atmosphere / skylight ─────────────────────────────────────────
        // Scan all entities; first entity with each component wins.

        let mut new_sky_atm: Option<&SkyAtmosphereData> = None;
        let mut new_skylight: Option<&SkylightData> = None;
        for (_id, c) in store.iter() {
            if new_sky_atm.is_none() { new_sky_atm = c.sky_atmosphere.as_ref(); }
            if new_skylight.is_none() { new_skylight = c.skylight.as_ref(); }
            if new_sky_atm.is_some() && new_skylight.is_some() { break; }
        }

        // Upload sky atmosphere only when it actually changed (avoids re-rendering
        // the expensive sky LUT every frame).
        if new_sky_atm != self.cached_sky_atm.as_ref() {
            self.cached_sky_atm = new_sky_atm.cloned();
            self.renderer.set_sky_atmosphere(new_sky_atm.map(stratum_sky_atmosphere_to_helio));
        }

        if new_skylight != self.cached_skylight.as_ref() {
            self.cached_skylight = new_skylight.cloned();
            self.renderer.set_skylight(new_skylight.map(stratum_skylight_to_helio));
        }
    }

    // ── Lifecycle ─────────────────────────────────────────────────────────────

    /// Notify the renderer that the output surface was resized.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.renderer.resize(width, height);
    }

    // ── Debug drawing ─────────────────────────────────────────────────────────

    /// Submit debug wireframe boxes for every chunk in `partition`.
    ///
    /// Color-coding by [`ChunkState`]:
    /// * **Active**    — green  (`[0.0, 1.0, 0.0, 0.4]`)
    /// * **Loading**   — yellow (`[1.0, 1.0, 0.0, 0.4]`)
    /// * **Unloading** — orange (`[1.0, 0.5, 0.0, 0.3]`)
    /// * **Unloaded**  — gray   (`[0.5, 0.5, 0.5, 0.15]`)
    ///
    /// Call this after `submit_frame` (or before — shapes are transient and
    /// cleared automatically by the renderer after each render call).
    pub fn debug_draw_world_partition(&mut self, partition: &WorldPartition) {
        for chunk in partition.chunks() {
            let center       = chunk.bounds.center();
            let half_extents = chunk.bounds.half_extents();
            let color = match chunk.state {
                ChunkState::Active    => [0.0, 1.0, 0.0, 0.4],
                ChunkState::Loading   => [1.0, 1.0, 0.0, 0.4],
                ChunkState::Unloading => [1.0, 0.5, 0.0, 0.3],
                ChunkState::Unloaded  => [0.5, 0.5, 0.5, 0.15],
            };
            self.renderer.debug_box(center, half_extents, Quat::IDENTITY, color, 0.03);
        }
    }

    /// Submit debug attenuation volumes for every light entity in `store`.
    ///
    /// * **Point** light → wireframe sphere at the light position with radius = `range`.
    /// * **Spot**  light → wireframe cone with apex at position, pointing along
    ///   `direction`, height = `range`, base radius = `range * tan(outer_angle)`.
    /// * **Directional** light → three short arrows in the light direction (no
    ///   attenuation to visualise, so just a direction indicator).
    ///
    /// Call before `submit_frame` so shapes are flushed with the same render call.
    pub fn debug_draw_lights(&mut self, store: &EntityStore) {
        for (_id, components) in store.iter() {
            let (Some(light), Some(transform)) = (&components.light, &components.transform)
            else { continue };

            let pos = transform.position;

            match light {
                LightData::Point { color, range, .. } => {
                    let c = [color[0], color[1], color[2], 0.5];
                    self.renderer.debug_sphere(pos, *range, c, 0.03);
                }

                LightData::Spot { direction, color, range, outer_angle, .. } => {
                    let dir = Vec3::from(*direction).normalize_or_zero();
                    let base_radius = range * outer_angle.tan();
                    let c = [color[0], color[1], color[2], 0.55];
                    self.renderer.debug_cone(pos, dir, *range, base_radius, c, 0.03);
                }

                LightData::Directional { direction, color, .. } => {
                    // Three parallel arrow shafts to show direction (no range).
                    let dir  = Vec3::from(*direction).normalize_or_zero();
                    let c    = [color[0], color[1], color[2], 0.6];
                    for offset in [Vec3::ZERO, Vec3::X * 0.4, Vec3::Z * 0.4] {
                        let start = pos + offset;
                        self.renderer.debug_line(start, start + dir * 3.0, c, 0.03);
                    }
                }
            }
        }
    }

    // ── Frame submission ──────────────────────────────────────────────────────
    /// Submit all render views for one frame.
    ///
    /// # Parameters
    ///
    /// | Name                  | Description                                 |
    /// |-----------------------|---------------------------------------------|
    /// | `views`               | Output of `Stratum::build_views()`          |
    /// | `level`               | Active level (entity data for scene build)  |
    /// | `primary_surface`     | The swapchain image acquired this frame     |
    /// | `delta_time`          | Frame delta in seconds                      |
    ///
    /// Views are already sorted by priority when produced by Stratum; this
    /// function submits them in order.
    pub fn submit_frame(
        &mut self,
        views:          &[RenderView],
        level:          &Level,
        primary_surface: &wgpu::TextureView,
        delta_time:     f32,
    ) -> helio_render_v2::Result<()> {
        // Sync all persistent proxies once per frame (meshes, lights, sky).
        // Zero GPU cost at steady state — only dirty slots are uploaded.
        self.sync_entity_objects(level);
        self.sync_lights_and_sky(level);

        for view in views {
            let camera = render_view_to_camera(view);

            // Resolve offscreen target name before borrowing renderer mutably.
            let offscreen_name: Option<String> = match &view.render_target {
                RenderTargetHandle::OffscreenTexture(name)
                    if self.offscreen_views.contains_key(name.as_str()) =>
                {
                    Some(name.clone())
                }
                _ => None,
            };

            if let Some(ref name) = offscreen_name {
                if let Some(offscreen) = self.offscreen_views.get(name.as_str()) {
                    self.renderer.render(&camera, offscreen, delta_time)?;
                    continue;
                }
            }

            // Warn for unresolved targets and fall back to primary surface.
            match &view.render_target {
                RenderTargetHandle::OffscreenTexture(name) => {
                    log::warn!(
                        "Unresolved offscreen texture '{}' — routing to primary surface",
                        name
                    );
                }
                RenderTargetHandle::PrimarySurface => {}
                other => {
                    log::warn!(
                        "Unresolved render target {:?} — routing to primary surface",
                        other
                    );
                }
            }
            self.renderer.render(&camera, primary_surface, delta_time)?;
        }

        Ok(())
    }
}
