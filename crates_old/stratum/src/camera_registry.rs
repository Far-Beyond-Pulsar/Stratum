//! Camera registry — owns and vends all `StratumCamera`s by `CameraId`.

use std::collections::HashMap;
use crate::camera::{CameraId, CameraKind, StratumCamera};

/// Central store for every camera in the world.
///
/// Cameras are independent of `Level`s. The registry lives in `Stratum` and
/// survives level loads/unloads. Game cameras and editor cameras coexist here;
/// `SimulationMode` controls which ones produce render views.
pub struct CameraRegistry {
    next_id: u64,
    cameras: HashMap<CameraId, StratumCamera>,
}

impl CameraRegistry {
    pub fn new() -> Self {
        Self { next_id: 1, cameras: HashMap::new() }
    }

    // ── Lifecycle ─────────────────────────────────────────────────────────────

    /// Register a camera, assigning it a unique `CameraId`.
    ///
    /// The `id` field of the supplied camera is overwritten with the assigned ID.
    pub fn register(&mut self, mut camera: StratumCamera) -> CameraId {
        let id = CameraId::new(self.next_id);
        self.next_id += 1;
        camera.id = id;
        self.cameras.insert(id, camera);
        id
    }

    /// Remove a camera. Returns its data if it existed.
    pub fn unregister(&mut self, id: CameraId) -> Option<StratumCamera> {
        self.cameras.remove(&id)
    }

    // ── Access ────────────────────────────────────────────────────────────────

    pub fn get    (&self,     id: CameraId) -> Option<&StratumCamera>     { self.cameras.get(&id) }
    pub fn get_mut(&mut self, id: CameraId) -> Option<&mut StratumCamera> { self.cameras.get_mut(&id) }

    // ── Iteration ─────────────────────────────────────────────────────────────

    /// All cameras (any kind, active or not).
    pub fn iter(&self) -> impl Iterator<Item = (CameraId, &StratumCamera)> {
        self.cameras.iter().map(|(&id, c)| (id, c))
    }

    /// Cameras marked `active = true`.
    pub fn active_cameras(&self) -> impl Iterator<Item = (CameraId, &StratumCamera)> {
        self.cameras
            .iter()
            .filter(|(_, c)| c.active)
            .map(|(&id, c)| (id, c))
    }

    /// Active editor cameras (Perspective or Orthographic).
    pub fn editor_cameras(&self) -> impl Iterator<Item = (CameraId, &StratumCamera)> {
        self.cameras
            .iter()
            .filter(|(_, c)| c.active && matches!(
                c.kind,
                CameraKind::EditorPerspective | CameraKind::EditorOrthographic
            ))
            .map(|(&id, c)| (id, c))
    }

    /// Active game cameras.
    pub fn game_cameras(&self) -> impl Iterator<Item = (CameraId, &StratumCamera)> {
        self.cameras
            .iter()
            .filter(|(_, c)| c.active && matches!(c.kind, CameraKind::GameCamera { .. }))
            .map(|(&id, c)| (id, c))
    }

    pub fn len     (&self) -> usize { self.cameras.len() }
    pub fn is_empty(&self) -> bool  { self.cameras.is_empty() }
}

impl Default for CameraRegistry {
    fn default() -> Self { Self::new() }
}

// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use crate::camera::{CameraKind, Projection};
    use crate::render_view::{RenderTargetHandle, Viewport};

    fn make_game_camera(active: bool) -> StratumCamera {
        StratumCamera {
            id:            CameraId::PLACEHOLDER,
            kind:          CameraKind::GameCamera { tag: "main".into() },
            position:      Vec3::ZERO,
            yaw:           0.0,
            pitch:         0.0,
            projection:    Projection::perspective(1.0, 0.1, 100.0),
            render_target: RenderTargetHandle::PrimarySurface,
            viewport:      Viewport::full(),
            priority:      0,
            active,
        }
    }

    fn make_editor_camera() -> StratumCamera {
        StratumCamera {
            id:         CameraId::PLACEHOLDER,
            kind:       CameraKind::EditorPerspective,
            position:   Vec3::ZERO,
            yaw:        0.0,
            pitch:      0.0,
            projection: Projection::perspective(1.0, 0.1, 100.0),
            render_target: RenderTargetHandle::PrimarySurface,
            viewport:   Viewport::full(),
            priority:   0,
            active:     true,
        }
    }

    #[test]
    fn register_assigns_unique_ids() {
        let mut reg = CameraRegistry::new();
        let id1 = reg.register(make_game_camera(true));
        let id2 = reg.register(make_game_camera(true));
        assert_ne!(id1, id2);
    }

    #[test]
    fn register_overwrites_placeholder_id() {
        let mut reg = CameraRegistry::new();
        let id = reg.register(make_game_camera(true));
        assert_ne!(id, CameraId::PLACEHOLDER);
    }

    #[test]
    fn get_returns_registered_camera() {
        let mut reg = CameraRegistry::new();
        let id  = reg.register(make_game_camera(true));
        assert!(reg.get(id).is_some());
    }

    #[test]
    fn unregister_removes_camera() {
        let mut reg = CameraRegistry::new();
        let id = reg.register(make_game_camera(true));
        assert!(reg.unregister(id).is_some());
        assert!(reg.get(id).is_none());
    }

    #[test]
    fn active_cameras_excludes_inactive() {
        let mut reg = CameraRegistry::new();
        reg.register(make_game_camera(false));
        reg.register(make_game_camera(true));
        assert_eq!(reg.active_cameras().count(), 1);
    }

    #[test]
    fn editor_cameras_filter() {
        let mut reg = CameraRegistry::new();
        reg.register(make_game_camera(true));
        reg.register(make_editor_camera());
        assert_eq!(reg.editor_cameras().count(), 1);
    }

    #[test]
    fn game_cameras_filter() {
        let mut reg = CameraRegistry::new();
        reg.register(make_game_camera(true));
        reg.register(make_editor_camera());
        assert_eq!(reg.game_cameras().count(), 1);
    }

    #[test]
    fn len_and_is_empty() {
        let mut reg = CameraRegistry::new();
        assert!(reg.is_empty());
        reg.register(make_game_camera(true));
        assert_eq!(reg.len(), 1);
    }
}
