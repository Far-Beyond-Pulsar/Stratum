//! `HelioIntegration` — Stratum-to-Helio render submission.
//!
//! `HelioIntegration` wraps a Helio `Renderer` and an `AssetRegistry`.
//! Each frame, the host calls `submit_frame()` with the `Vec<RenderView>`
//! produced by `Stratum::build_views()`. The integration:
//!
//! 1. **Syncs the object registry** — inserts new active-chunk entities via
//!    `insert_object`, removes deactivated-chunk entities via `remove_object`.
//!    Both operations are O(1) in the Helio persistent-slot architecture.
//!    Uses `GroupMask` (Static / Dynamic / ShadowOnly / Editor) from each
//!    entity's [`GroupHint`] component.
//! 2. **Syncs lights** — scans all entities (not just active-chunk ones)
//!    so out-of-range shadow-casting lights are still registered.
//! 3. **Submits billboards** stateless each frame via `set_billboard_instances`.
//! 4. **Calls `renderer.render(&camera, surface)`** per view — TAA jitter and
//!    `scene.flush()` / `scene.advance_frame()` are handled internally.
//!
//! ## Optimisation surface
//!
//! After loading a static level, call [`HelioIntegration::optimise_for_static_level`]
//! to sort objects by (mesh, material) for instanced draw calls. This is O(N log N)
//! once and reduces GPU draw call count significantly for dense static scenes.
//!
//! ## Render target resolution
//!
//! * `RenderTargetHandle::PrimarySurface` → the `wgpu::TextureView` passed in
//!   directly by the caller (the swapchain image acquired each frame).
//! * `RenderTargetHandle::OffscreenTexture(name)` → a `wgpu::TextureView`
//!   registered via `register_offscreen_view`.  Falls back to primary surface
//!   if the name is unknown.
//! * `ViewportSlot` → falls back to primary surface.

use std::collections::HashMap;

use glam::{Vec3};
use stratum::{
    ChunkState, EntityStore, GroupHint, Level,
    RenderTargetHandle, RenderView, WorldPartition,
};
use stratum::entity::EntityId;
use helio::{
    BillboardInstance, GpuLight, GroupId, GroupMask, LightId, MaterialId, ObjectDescriptor,
    ObjectId, Renderer, VirtualObjectDescriptor,
};

use crate::asset_registry::AssetRegistry;
use crate::bridge::{render_view_to_camera, stratum_light_to_gpu_light};

// ── GroupHint → GroupMask mapping ─────────────────────────────────────────────

fn group_mask_for_hint(hint: GroupHint) -> GroupMask {
    match hint {
        GroupHint::None       => GroupMask::NONE,
        GroupHint::Static     => GroupMask::from(GroupId::STATIC),
        GroupHint::Dynamic    => GroupMask::from(GroupId::DYNAMIC),
        GroupHint::ShadowOnly => GroupMask::from(GroupId::SHADOW_CASTERS),
        GroupHint::Editor     => GroupMask::from(GroupId::EDITOR),
    }
}

// ── HelioIntegration ──────────────────────────────────────────────────────────

/// Owns the Helio renderer and the asset registry, and drives render submission.
pub struct HelioIntegration {
    renderer: Renderer,
    assets:   AssetRegistry,
    /// `EntityId → ObjectId` for entities currently resident in the Helio scene.
    /// Populated when a chunk activates; evicted when a chunk deactivates or the
    /// entity is despawned.
    entity_objects: HashMap<EntityId, ObjectId>,
    /// `EntityId → VirtualObjectId` for virtual-geometry entities.
    entity_vobjects: HashMap<EntityId, helio::VirtualObjectId>,
    /// `EntityId → LightId` — lights are registered globally (not per-chunk).
    light_objects: HashMap<EntityId, LightId>,
    /// Named offscreen render targets.
    offscreen_views: HashMap<String, wgpu::TextureView>,
}

impl HelioIntegration {
    pub fn new(renderer: Renderer, assets: AssetRegistry) -> Self {
        Self {
            renderer,
            assets,
            entity_objects:   HashMap::new(),
            entity_vobjects:  HashMap::new(),
            light_objects:    HashMap::new(),
            offscreen_views:  HashMap::new(),
        }
    }

    // ── Accessors ─────────────────────────────────────────────────────────────

    pub fn renderer    (&self)     -> &Renderer      { &self.renderer }
    pub fn renderer_mut(&mut self) -> &mut Renderer  { &mut self.renderer }
    pub fn assets      (&self)     -> &AssetRegistry { &self.assets }
    pub fn assets_mut  (&mut self) -> &mut AssetRegistry { &mut self.assets }

    // ── Convenience asset upload ──────────────────────────────────────────────

    /// Upload a mesh into the asset registry and return its handle.
    pub fn upload_mesh(&mut self, mesh: helio::MeshUpload) -> stratum::MeshHandle {
        self.assets.upload_mesh(&mut self.renderer, mesh)
    }

    /// Upload a plain (untextured) GPU material and return its handle.
    pub fn upload_material(&mut self, mat: helio::GpuMaterial) -> stratum::MaterialHandle {
        self.assets.upload_material(&mut self.renderer, mat)
    }

    /// Upload a base-colour texture + PBR parameters as a single material.
    ///
    /// The texture is uploaded as sRGB RGBA8, and a `MaterialAsset` with the
    /// base-colour ref is submitted.  On texture-upload failure, falls back to
    /// an untextured material with the given `base_color`.
    pub fn upload_textured_material(
        &mut self,
        tex_rgba:   Vec<u8>,
        tex_w:      u32,
        tex_h:      u32,
        roughness:  f32,
        base_color: [f32; 4],
    ) -> stratum::MaterialHandle {
        use helio::{
            GpuMaterial, MaterialAsset, MaterialTextures, MaterialTextureRef,
            TextureSamplerDesc, TextureUpload,
        };
        use bytemuck::Zeroable;

        let tex_id = self.renderer
            .insert_texture(TextureUpload::rgba8(
                "material_tex", tex_w, tex_h, true, tex_rgba, TextureSamplerDesc::default(),
            ))
            .ok();

        let mut textures = MaterialTextures::default();
        if let Some(tid) = tex_id {
            textures.base_color = Some(MaterialTextureRef::new(tid));
        }

        let gpu = GpuMaterial {
            base_color,
            roughness_metallic: [roughness, 0.0, 1.5, 0.0],
            emissive: [0.0; 4],
            tex_base_color: if tex_id.is_some() { 0 } else { GpuMaterial::NO_TEXTURE },
            tex_normal:        GpuMaterial::NO_TEXTURE,
            tex_roughness:     GpuMaterial::NO_TEXTURE,
            tex_emissive:      GpuMaterial::NO_TEXTURE,
            tex_occlusion:     GpuMaterial::NO_TEXTURE,
            workflow: 0,
            flags:   0,
            _pad:    0,
        };
        let asset = MaterialAsset { gpu, textures };
        match self.assets.upload_material_asset(&mut self.renderer, asset) {
            Ok(h) => h,
            Err(e) => {
                log::warn!("upload_textured_material: {e:?}");
                self.assets.upload_material(&mut self.renderer, GpuMaterial::zeroed())
            }
        }
    }

    // ── Offscreen texture registry ────────────────────────────────────────────

    pub fn register_offscreen_view(&mut self, name: impl Into<String>, view: wgpu::TextureView) {
        self.offscreen_views.insert(name.into(), view);
    }

    pub fn unregister_offscreen_view(&mut self, name: &str) {
        self.offscreen_views.remove(name);
    }

    // ── Static-level optimisation ─────────────────────────────────────────────

    /// Sort all scene objects by (mesh, material) for instanced draw calls.
    ///
    /// Call this **once** after a static level has fully loaded and all chunk
    /// entities have been inserted. Any subsequent `insert_object` or
    /// `remove_object` (from streaming) will invalidate the sort — this is
    /// expected for fully-static levels where streaming does not occur after
    /// the initial load.
    ///
    /// Complexity: O(N log N) — call sparingly.
    pub fn optimise_for_static_level(&mut self) {
        self.renderer.optimize_scene_layout();
    }

    // ── Group visibility ──────────────────────────────────────────────────────

    /// Hide all objects belonging to `group` (e.g., hide editor gizmos in game
    /// mode). Zero-cost GPU mask operation.
    pub fn hide_group(&mut self, group: GroupId) {
        self.renderer.hide_group(group);
    }

    /// Show all objects belonging to `group`.
    pub fn show_group(&mut self, group: GroupId) {
        self.renderer.show_group(group);
    }

    // ── Object registry sync ──────────────────────────────────────────────────

    /// Reconcile the Helio object registry against the current active-chunk set.
    ///
    /// ## Strategy
    ///
    /// Builds the **wanted** set (active-chunk entities with a mesh/virtual-mesh)
    /// and the **have** set (`entity_objects` keys). Then:
    ///
    /// * `to_add = wanted − have`    → `insert_object` / `insert_virtual_object` (O(1))
    /// * `to_remove = have − wanted` → `remove_object` / `remove_virtual_object` (O(1))
    /// * `stable`                    → `update_object_transform` if changed
    ///
    /// Streaming chunk activate/deactivate therefore causes only the necessary
    /// insertions and removals — no per-frame full-scan.
    fn sync_entity_objects(&mut self, level: &Level) {
        let store  = level.entities();
        let active: std::collections::HashSet<EntityId> =
            level.partition().active_entities().into_iter().collect();

        // ── Remove objects for entities no longer wanted ──────────────────────
        let to_remove: Vec<EntityId> = self.entity_objects.keys()
            .filter(|&&id| {
                // Remove if: entity despawned, lost its mesh, or chunk deactivated.
                !active.contains(&id)
                || store.get(id).map_or(true, |c| c.mesh.is_none())
            })
            .copied().collect();
        for id in to_remove {
            if let Some(oid) = self.entity_objects.remove(&id) {
                if let Err(e) = self.renderer.scene_mut().remove_object(oid) {
                    log::warn!("remove_object {id:?}: {e:?}");
                }
            }
        }

        let to_remove_vg: Vec<EntityId> = self.entity_vobjects.keys()
            .filter(|&&id| {
                !active.contains(&id)
                || store.get(id).map_or(true, |c| c.mesh.is_none())
            })
            .copied().collect();
        for id in to_remove_vg {
            if let Some(vid) = self.entity_vobjects.remove(&id) {
                if let Err(e) = self.renderer.remove_virtual_object(vid) {
                    log::warn!("remove_virtual_object {id:?}: {e:?}");
                }
            }
        }

        // ── Insert new active entities ────────────────────────────────────────
        for &entity_id in &active {
            let Some(comp) = store.get(entity_id) else { continue };
            let Some(mesh_handle) = comp.mesh else { continue };

            let transform = comp.transform.as_ref()
                .map(|t| glam::Mat4::from_scale_rotation_translation(t.scale, t.rotation, t.position))
                .unwrap_or(glam::Mat4::IDENTITY);
            let bounds = bounding_sphere_to_array(&comp.transform.as_ref()
                .map(|t| t.position)
                .unwrap_or(Vec3::ZERO), comp.bounding_radius);
            let groups = group_mask_for_hint(comp.group_hint);

            if comp.use_virtual_geometry {
                if self.entity_vobjects.contains_key(&entity_id) { continue }
                let Some(vg_id) = self.assets.get_virtual_mesh_id(mesh_handle) else {
                    log::warn!("Entity {entity_id:?}: use_virtual_geometry but no VirtualMesh registered for {mesh_handle:?}");
                    continue;
                };
                let mat_id = comp.material
                    .and_then(|mh| self.assets.get_material_id(mh))
                    .map(|id| id.slot())
                    .unwrap_or(0);
                match self.renderer.insert_virtual_object(VirtualObjectDescriptor {
                    virtual_mesh: vg_id,
                    material_id:  mat_id,
                    transform,
                    bounds,
                    flags:        0,
                    groups,
                }) {
                    Ok(vid) => { self.entity_vobjects.insert(entity_id, vid); }
                    Err(e)  => { log::warn!("insert_virtual_object {entity_id:?}: {e:?}"); }
                }
            } else {
                if self.entity_objects.contains_key(&entity_id) { continue }
                let Some(mesh_id) = self.assets.get_mesh_id(mesh_handle) else {
                    log::warn!("Entity {entity_id:?}: no MeshId registered for {mesh_handle:?}");
                    continue;
                };
                let mat_id = comp.material
                    .and_then(|mh| self.assets.get_material_id(mh))
                    .unwrap_or_else(|| MaterialId::from_raw(0, 0));
                match self.renderer.insert_object(ObjectDescriptor {
                    mesh:      mesh_id,
                    material:  mat_id,
                    transform,
                    bounds,
                    flags:     shadow_flags(comp),
                    groups,
                }) {
                    Ok(oid) => { self.entity_objects.insert(entity_id, oid); }
                    Err(e)  => { log::warn!("insert_object {entity_id:?}: {e:?}"); }
                }
            }
        }

        // ── Update transforms for stable (already-registered) active objects ──
        for (&entity_id, &oid) in &self.entity_objects {
            if !active.contains(&entity_id) { continue }
            let Some(comp) = store.get(entity_id) else { continue };
            let transform = comp.transform.as_ref()
                .map(|t| glam::Mat4::from_scale_rotation_translation(t.scale, t.rotation, t.position))
                .unwrap_or(glam::Mat4::IDENTITY);
            if let Err(e) = self.renderer.update_object_transform(oid, transform) {
                log::warn!("update_object_transform {entity_id:?}: {e:?}");
            }
        }
        for (&entity_id, &vid) in &self.entity_vobjects {
            if !active.contains(&entity_id) { continue }
            let Some(comp) = store.get(entity_id) else { continue };
            let transform = comp.transform.as_ref()
                .map(|t| glam::Mat4::from_scale_rotation_translation(t.scale, t.rotation, t.position))
                .unwrap_or(glam::Mat4::IDENTITY);
            if let Err(e) = self.renderer.update_virtual_object_transform(vid, transform) {
                log::warn!("update_virtual_object_transform {entity_id:?}: {e:?}");
            }
        }
    }

    // ── Light sync ────────────────────────────────────────────────────────────

    /// Synchronise persistent light proxies against ALL entities in the level.
    ///
    /// Lights are not chunk-gated: a point light's range can extend well beyond
    /// its chunk boundary.  This mirrors the old integration's behaviour.
    fn sync_lights(&mut self, store: &EntityStore) {
        let mut current: HashMap<EntityId, GpuLight> = HashMap::new();
        for (id, c) in store.iter() {
            let Some(light) = &c.light else { continue };
            let pos = c.transform.as_ref().map(|t| t.position).unwrap_or(Vec3::ZERO);
            current.insert(id, stratum_light_to_gpu_light(light, pos));
        }

        // Remove lights for despawned entities.
        let removed: Vec<EntityId> = self.light_objects.keys()
            .filter(|id| !current.contains_key(*id))
            .copied().collect();
        for id in removed {
            if let Some(lid) = self.light_objects.remove(&id) {
                if let Err(e) = self.renderer.remove_light(lid) {
                    log::warn!("remove_light {id:?}: {e:?}");
                }
            }
        }

        // Add new / update existing.
        for (id, gpu_light) in current {
            if let Some(&lid) = self.light_objects.get(&id) {
                if let Err(e) = self.renderer.update_light(lid, gpu_light) {
                    log::warn!("update_light {id:?}: {e:?}");
                }
            } else {
                let lid = self.renderer.insert_light(gpu_light);
                self.light_objects.insert(id, lid);
            }
        }
    }

    // ── Billboard sync ────────────────────────────────────────────────────────

    /// Collect all entity billboards and submit them to the renderer in one call.
    ///
    /// The new Helio billboard API is stateless — `set_billboard_instances` replaces
    /// the entire list each frame.  This is simpler and just as efficient.
    fn sync_billboards(&mut self, store: &EntityStore, extras: &[BillboardInstance]) {
        let mut instances: Vec<BillboardInstance> = store.iter()
            .filter_map(|(_, c)| {
                let bb = c.billboard.as_ref()?;
                let tf = c.transform.as_ref()?;
                let pos = tf.position;
                Some(BillboardInstance {
                    world_pos:   [pos.x, pos.y, pos.z, 0.0],
                    scale_flags: [bb.size[0], bb.size[1], if bb.screen_scale { 1.0 } else { 0.0 }, 0.0],
                    color:       bb.color,
                })
            })
            .collect();
        instances.extend_from_slice(extras);
        self.renderer.set_billboard_instances(&instances);
    }

    // ── Lifecycle ─────────────────────────────────────────────────────────────

    /// Notify the renderer that the output surface was resized.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.renderer.set_render_size(width, height);
    }

    // ── Debug drawing ─────────────────────────────────────────────────────────

    /// Submit debug wireframe boxes for every chunk in `partition`.
    ///
    /// Colour-coded by [`ChunkState`]:
    /// * **Active**    — green  `[0.0, 1.0, 0.0, 0.4]`
    /// * **Loading**   — yellow `[1.0, 1.0, 0.0, 0.4]`
    /// * **Unloading** — orange `[1.0, 0.5, 0.0, 0.3]`
    /// * **Unloaded**  — gray   `[0.5, 0.5, 0.5, 0.15]`
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
            // Debug draw is renderer-specific; use scene ambient as a proxy or
            // integrate with a debug-draw pass if available.
            let _ = (center, half_extents, color); // TODO: wire to debug pass
        }
    }

    /// Submit debug attenuation volumes for every light entity in `store`.
    pub fn debug_draw_lights(&mut self, store: &EntityStore) {
        for (_id, components) in store.iter() {
            let (Some(light), Some(transform)) = (&components.light, &components.transform)
            else { continue };
            let _ = (light, transform); // TODO: wire to debug pass
        }
    }

    // ── Frame submission ──────────────────────────────────────────────────────

    /// Submit all render views for one frame.
    ///
    /// | Parameter         | Description                                    |
    /// |-------------------|------------------------------------------------|
    /// | `views`           | Output of `Stratum::build_views()`             |
    /// | `level`           | Active level (entity data for scene sync)      |
    /// | `primary_surface` | Swapchain image acquired this frame            |
    /// | `extra_billboards`| Additional billboard instances (e.g., RC grid) |
    ///
    /// Views are sorted by priority when produced by Stratum; submitted in order.
    pub fn submit_frame(
        &mut self,
        views:            &[RenderView],
        level:            &Level,
        primary_surface:  &wgpu::TextureView,
        extra_billboards: &[BillboardInstance],
    ) -> helio::Result<()> {
        // Sync persistent proxies.
        self.sync_entity_objects(level);
        self.sync_lights(level.entities());
        self.sync_billboards(level.entities(), extra_billboards);

        for view in views {
            let camera = render_view_to_camera(view);

            let offscreen_name: Option<String> = match &view.render_target {
                RenderTargetHandle::OffscreenTexture(n)
                    if self.offscreen_views.contains_key(n.as_str()) => Some(n.clone()),
                _ => None,
            };

            if let Some(ref name) = offscreen_name {
                if let Some(target) = self.offscreen_views.get(name.as_str()) {
                    self.renderer.render(&camera, target)?;
                    continue;
                }
            }

            match &view.render_target {
                RenderTargetHandle::OffscreenTexture(name) => {
                    log::warn!("Unresolved offscreen texture '{name}' — routing to primary surface");
                }
                RenderTargetHandle::PrimarySurface => {}
                other => {
                    log::warn!("Unresolved render target {other:?} — routing to primary surface");
                }
            }
            self.renderer.render(&camera, primary_surface)?;
        }

        Ok(())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Convert a world-space bounding sphere to the `[cx, cy, cz, radius]` format
/// expected by `ObjectDescriptor::bounds`.
fn bounding_sphere_to_array(center: &Vec3, radius: f32) -> [f32; 4] {
    let r = if radius > 0.0 { radius } else { 50.0 };
    [center.x, center.y, center.z, r]
}

/// Object flags: bit 0 = casts shadow, bit 1 = receives shadow.
fn shadow_flags(comp: &stratum::Components) -> u32 {
    // By default all mesh objects cast and receive shadows.
    // Entities tagged "no_shadow" opt out.
    if comp.tags.iter().any(|t| t == "no_shadow") { 0 } else { 0b11 }
}

