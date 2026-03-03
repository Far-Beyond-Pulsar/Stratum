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
pub fn build_render_views(
    mode:          &SimulationMode,
    cameras:       &CameraRegistry,
    level:         &Level,
    window_width:  u32,
    window_height: u32,
    time:          f32,
) -> Vec<RenderView> {
    // Resident entity candidates (from active partition chunks).
    let active_entities = level.partition().active_entities();
    let store           = level.entities();

    let mut views: Vec<RenderView> = cameras
        .active_cameras()
        .filter(|(_, cam)| camera_active_in_mode(mode, &cam.kind))
        .map(|(cam_id, cam)| {
            let aspect    = cam.viewport.aspect(window_width, window_height);
            let view_proj = cam.view_proj(aspect);
            let frustum   = Frustum::from_view_proj(&view_proj);
            let visible   = visibility_cull(&active_entities, store, &frustum);

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
