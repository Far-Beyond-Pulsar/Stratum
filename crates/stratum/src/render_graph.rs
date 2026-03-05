//! Render view graph — per-frame `Vec<RenderView>` assembly.
//!
//! `build_render_views` is the core of Stratum's render orchestration. It:
//! 1. Queries the active partition for all resident entity IDs.
//! 2. Iterates active cameras, filtering by `SimulationMode`.
//! 3. Runs per-camera frustum culling to produce `visible_entities`.
//! 4. Packs everything into a `RenderView` and returns the sorted list.
//!
//! The function is pure given its inputs — no mutation, no side effects.

use crate::camera::CameraKind;
use crate::camera_registry::CameraRegistry;
use crate::level::Level;
use crate::mode::SimulationMode;
use crate::render_view::RenderView;
use crate::visibility::{Frustum, visibility_cull};

/// Decide whether a camera of `kind` produces a render view in `mode`.
fn camera_active_in_mode(mode: &SimulationMode, kind: &CameraKind) -> bool {
    match (mode, kind) {
        (SimulationMode::Editor, CameraKind::EditorPerspective)  => true,
        (SimulationMode::Editor, CameraKind::EditorOrthographic) => true,
        (SimulationMode::Editor, CameraKind::GameCamera { .. })  => false,
        (SimulationMode::Game,   CameraKind::GameCamera { .. })  => true,
        (SimulationMode::Game,   _)                              => false,
    }
}

/// Build the complete, sorted list of render views for one frame.
///
/// # Parameters
///
/// | Name            | Description                                           |
/// |-----------------|-------------------------------------------------------|
/// | `mode`          | Current `Editor` or `Game` mode                       |
/// | `cameras`       | Camera registry                                       |
/// | `level`         | Active level (entity store + partition)               |
/// | `window_width`  | Render target width in pixels (for aspect calculation)|
/// | `window_height` | Render target height in pixels                        |
/// | `time`          | Elapsed time in seconds (forwarded to shaders)        |
///
/// Returns views sorted by `priority` (ascending, lower = renders first).
///
/// # Candidate strategy
///
/// Geometry (meshes, billboards) is sourced only from **active** chunks so
/// that unloaded/loading geometry stays invisible. Lights are sourced from
/// **all** chunks because a point/spot light's influence radius (`range`) can
/// reach well into neighbouring chunks — restricting lights to active chunks
/// would silently black-out areas that are fully loaded and visible.
/// Frustum culling handles the final visibility decision for both.
pub fn build_render_views(
    mode:          &SimulationMode,
    cameras:       &CameraRegistry,
    level:         &Level,
    window_width:  u32,
    window_height: u32,
    time:          f32,
) -> Vec<RenderView> {
    let store = level.entities();

    // Geometry candidates: active chunks only (respects streaming state).
    let active_entities = level.partition().active_entities();

    // Light candidates: all chunks — a light's range can span chunk boundaries.
    // Build a deduplicated union: start with active entities, then append any
    // light-only entities from inactive chunks that aren't already included.
    let all_entities = level.partition().all_entities();
    let active_set: std::collections::HashSet<_> = active_entities.iter().copied().collect();
    let light_candidates: Vec<_> = all_entities
        .into_iter()
        .filter(|id| {
            // Already covered by active set — skip to avoid duplicates.
            if active_set.contains(id) { return false; }
            // Include only if this entity is a light (meshes stay partition-gated).
            store.get(*id).map(|c| c.light.is_some()).unwrap_or(false)
        })
        .collect();

    // Full candidate list: active geometry + out-of-range lights.
    let candidates: Vec<_> = active_entities
        .iter()
        .copied()
        .chain(light_candidates)
        .collect();

    let mut views: Vec<RenderView> = cameras
        .active_cameras()
        .filter(|(_, cam)| camera_active_in_mode(mode, &cam.kind))
        .map(|(cam_id, cam)| {
            let aspect    = cam.viewport.aspect(window_width, window_height);
            let view_proj = cam.view_proj(aspect);
            let frustum   = Frustum::from_view_proj(&view_proj);
            let visible   = visibility_cull(&candidates, store, &frustum);

            log::trace!(
                "Camera {:?} → {} visible entities",
                cam_id,
                visible.len()
            );

            RenderView {
                camera_id:        cam_id,
                view_proj,
                camera_position:  cam.position,
                time,
                render_target:    cam.render_target.clone(),
                viewport:         cam.viewport,
                visible_entities: visible,
                priority:         cam.priority,
            }
        })
        .collect();

    // Lower priority index renders first (background → foreground).
    views.sort_by_key(|v| v.priority);
    views
}
