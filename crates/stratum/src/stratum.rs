//! `Stratum` — the top-level world orchestrator.
//!
//! `Stratum` is the single entry point for all world orchestration logic.
//! It owns levels, cameras, and simulation state, and exposes a clean
//! two-call frame API:
//!
//! ```rust,ignore
//! stratum.tick(delta_time);                        // advance simulation
//! let views = stratum.build_views(w, h, time);     // produce render views
//! ```
//!
//! # Ownership model
//!
//! ```text
//! Stratum
//!  ├── Vec<Level>            (world content — entities, partition)
//!  ├── CameraRegistry        (all cameras, independent of levels)
//!  └── SimulationMode        (Editor | Game)
//! ```
//!
//! Cameras are decoupled from levels by design: an editor camera can exist
//! while no level is loaded, and one camera can look into a different level
//! than another (future multi-world support).

use glam::Vec3;

use crate::camera::{CameraId, StratumCamera};
use crate::camera_registry::CameraRegistry;
use crate::level::{Level, LevelId};
use crate::mode::SimulationMode;
use crate::render_graph::build_render_views;
use crate::render_view::RenderView;

/// Top-level world orchestrator for the Pulsar engine.
///
/// Create one instance per running world (typically one per application).
pub struct Stratum {
    mode:             SimulationMode,
    levels:           Vec<Level>,
    active_level_idx: Option<usize>,
    cameras:          CameraRegistry,
    /// Monotonically increasing counter for generating `LevelId`s.
    level_id_seq:     u64,
    /// Accumulated simulation time (only advances in `Game` mode).
    simulation_time:  f32,
}

impl Stratum {
    pub fn new(mode: SimulationMode) -> Self {
        Self {
            mode,
            levels:           Vec::new(),
            active_level_idx: None,
            cameras:          CameraRegistry::new(),
            level_id_seq:     1,
            simulation_time:  0.0,
        }
    }

    // ── Mode ──────────────────────────────────────────────────────────────────

    pub fn mode(&self) -> SimulationMode { self.mode }

    pub fn set_mode(&mut self, mode: SimulationMode) {
        log::info!("Stratum mode: {:?} → {:?}", self.mode, mode);
        self.mode = mode;
    }

    /// Toggle between `Editor` and `Game`. Hot-switchable mid-frame.
    pub fn toggle_mode(&mut self) {
        self.mode.toggle();
    }

    // ── Level management ──────────────────────────────────────────────────────

    /// Add a pre-built level. The first level added becomes the active level.
    pub fn add_level(&mut self, level: Level) -> LevelId {
        let id  = level.id();
        let idx = self.levels.len();
        self.levels.push(level);
        if self.active_level_idx.is_none() {
            self.active_level_idx = Some(idx);
        }
        id
    }

    /// Create a new level with sensible defaults and add it.
    ///
    /// `chunk_size`        — side length of one spatial cell in world units.
    /// `activation_radius` — distance at which chunks become resident.
    pub fn create_level(
        &mut self,
        name:             impl Into<String>,
        chunk_size:       f32,
        activation_radius: f32,
    ) -> LevelId {
        let id    = LevelId::new(self.level_id_seq);
        self.level_id_seq += 1;
        let level = Level::new(id, name, chunk_size, activation_radius);
        self.add_level(level);
        id
    }

    /// Make the level with `id` the active (primary) level.
    ///
    /// Returns `false` if no level with that ID exists.
    pub fn set_active_level(&mut self, id: LevelId) -> bool {
        if let Some(idx) = self.levels.iter().position(|l| l.id() == id) {
            self.active_level_idx = Some(idx);
            true
        } else {
            false
        }
    }

    pub fn active_level(&self) -> Option<&Level> {
        self.active_level_idx.map(|i| &self.levels[i])
    }

    pub fn active_level_mut(&mut self) -> Option<&mut Level> {
        self.active_level_idx.map(|i| &mut self.levels[i])
    }

    pub fn level(&self, id: LevelId) -> Option<&Level> {
        self.levels.iter().find(|l| l.id() == id)
    }

    pub fn level_mut(&mut self, id: LevelId) -> Option<&mut Level> {
        self.levels.iter_mut().find(|l| l.id() == id)
    }

    pub fn levels(&self) -> &[Level] { &self.levels }

    // ── Camera management ─────────────────────────────────────────────────────

    /// Access the camera registry (read-only).
    pub fn cameras(&self) -> &CameraRegistry { &self.cameras }

    /// Access the camera registry (mutable — for updating transforms etc.).
    pub fn cameras_mut(&mut self) -> &mut CameraRegistry { &mut self.cameras }

    /// Register a new camera. Returns the assigned `CameraId`.
    ///
    /// The `id` field of the supplied `StratumCamera` is overwritten by the
    /// registry; provide `CameraId::PLACEHOLDER` as the initial value.
    pub fn register_camera(&mut self, camera: StratumCamera) -> CameraId {
        self.cameras.register(camera)
    }

    pub fn unregister_camera(&mut self, id: CameraId) -> Option<StratumCamera> {
        self.cameras.unregister(id)
    }

    // ── Frame API ─────────────────────────────────────────────────────────────

    /// Advance the world by `delta_time` seconds.
    ///
    /// * In `Game` mode: increments `simulation_time` and updates the active
    ///   level's partition activation.
    /// * In `Editor` mode: only partition activation is updated (simulation
    ///   clock is frozen).
    ///
    /// Call once per frame, *before* `build_views`.
    pub fn tick(&mut self, delta_time: f32) {
        if self.mode.is_game() {
            self.simulation_time += delta_time;
        }

        // Gather camera positions to drive partition activation.
        let camera_positions: Vec<Vec3> = self.cameras
            .active_cameras()
            .map(|(_, c)| c.position)
            .collect();

        if let Some(level) = self.active_level_mut() {
            level.activate_partition_around(&camera_positions);
        }
    }

    /// Produce the list of render views for this frame.
    ///
    /// Returns one `RenderView` per active camera that is visible in the
    /// current mode, sorted by priority (ascending).
    ///
    /// Call *after* `tick()`.
    ///
    /// `window_width` / `window_height` are the primary surface pixel
    /// dimensions — used to compute each camera's aspect ratio.
    pub fn build_views(
        &self,
        window_width:  u32,
        window_height: u32,
        time:          f32,
    ) -> Vec<RenderView> {
        let Some(level) = self.active_level() else {
            return Vec::new();
        };
        build_render_views(
            &self.mode,
            &self.cameras,
            level,
            window_width,
            window_height,
            time,
        )
    }

    // ── Diagnostics ───────────────────────────────────────────────────────────

    /// Elapsed simulation time in seconds (Game mode only; frozen in Editor).
    pub fn simulation_time(&self) -> f32 { self.simulation_time }
}

// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use std::f32::consts::FRAC_PI_4;
    use crate::camera::{CameraKind, Projection};
    use crate::entity::{Components, MeshHandle, Transform};
    use crate::render_view::{RenderTargetHandle, Viewport};

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn make_stratum(mode: SimulationMode) -> Stratum {
        Stratum::new(mode)
    }

    fn game_camera() -> StratumCamera {
        StratumCamera {
            id:            CameraId::PLACEHOLDER,
            kind:          CameraKind::GameCamera { tag: "main".into() },
            position:      Vec3::new(0.0, 2.0, 10.0),
            yaw:           0.0,
            pitch:         -0.1,
            projection:    Projection::perspective(FRAC_PI_4, 0.1, 200.0),
            render_target: RenderTargetHandle::PrimarySurface,
            viewport:      Viewport::full(),
            priority:      0,
            active:        true,
        }
    }

    fn editor_camera() -> StratumCamera {
        StratumCamera {
            id:         CameraId::PLACEHOLDER,
            kind:       CameraKind::EditorPerspective,
            position:   Vec3::new(0.0, 5.0, 15.0),
            yaw:        0.0,
            pitch:      -0.3,
            projection: Projection::perspective(FRAC_PI_4, 0.1, 200.0),
            render_target: RenderTargetHandle::PrimarySurface,
            viewport:   Viewport::full(),
            priority:   0,
            active:     true,
        }
    }

    fn populate_level(stratum: &mut Stratum, level_id: LevelId) {
        let level = stratum.level_mut(level_id).unwrap();
        level.spawn_entity(
            Components::new()
                .with_transform(Transform::from_position(Vec3::new(0.0, 0.5, 0.0)))
                .with_mesh(MeshHandle(1)),
        );
        level.activate_all_chunks();
    }

    // ── Mode ──────────────────────────────────────────────────────────────────

    #[test]
    fn initial_mode_is_respected() {
        let s = make_stratum(SimulationMode::Editor);
        assert!(s.mode().is_editor());
    }

    #[test]
    fn set_mode_changes_mode() {
        let mut s = make_stratum(SimulationMode::Editor);
        s.set_mode(SimulationMode::Game);
        assert!(s.mode().is_game());
    }

    #[test]
    fn toggle_mode_flips_between_states() {
        let mut s = make_stratum(SimulationMode::Editor);
        s.toggle_mode();
        assert!(s.mode().is_game());
        s.toggle_mode();
        assert!(s.mode().is_editor());
    }

    // ── Level management ──────────────────────────────────────────────────────

    #[test]
    fn create_level_returns_unique_ids() {
        let mut s  = make_stratum(SimulationMode::Game);
        let id1 = s.create_level("a", 16.0, 32.0);
        let id2 = s.create_level("b", 16.0, 32.0);
        assert_ne!(id1, id2);
    }

    #[test]
    fn first_level_is_auto_active() {
        let mut s  = make_stratum(SimulationMode::Game);
        let id  = s.create_level("main", 16.0, 32.0);
        assert_eq!(s.active_level().unwrap().id(), id);
    }

    #[test]
    fn set_active_level_switches() {
        let mut s   = make_stratum(SimulationMode::Game);
        let _id1 = s.create_level("a", 16.0, 32.0);
        let id2  = s.create_level("b", 16.0, 32.0);
        assert!(s.set_active_level(id2));
        assert_eq!(s.active_level().unwrap().id(), id2);
    }

    #[test]
    fn set_active_level_unknown_returns_false() {
        let mut s = make_stratum(SimulationMode::Game);
        assert!(!s.set_active_level(LevelId::new(9999)));
    }

    #[test]
    fn level_lookup_by_id() {
        let mut s  = make_stratum(SimulationMode::Game);
        let id  = s.create_level("foo", 16.0, 32.0);
        assert!(s.level(id).is_some());
        assert!(s.level(LevelId::new(9999)).is_none());
    }

    #[test]
    fn no_active_level_build_views_returns_empty() {
        let s = make_stratum(SimulationMode::Game);
        assert!(s.build_views(1280, 720, 0.0).is_empty());
    }

    // ── Camera management ─────────────────────────────────────────────────────

    #[test]
    fn register_camera_returns_unique_id() {
        let mut s  = make_stratum(SimulationMode::Game);
        let id1 = s.register_camera(game_camera());
        let id2 = s.register_camera(game_camera());
        assert_ne!(id1, id2);
    }

    #[test]
    fn unregister_camera_removes_it() {
        let mut s  = make_stratum(SimulationMode::Game);
        let id  = s.register_camera(game_camera());
        assert!(s.unregister_camera(id).is_some());
        assert!(s.cameras().get(id).is_none());
    }

    // ── build_views — mode filtering ──────────────────────────────────────────

    #[test]
    fn build_views_game_mode_uses_game_camera() {
        let mut s     = make_stratum(SimulationMode::Game);
        let level_id  = s.create_level("world", 16.0, 48.0);
        populate_level(&mut s, level_id);
        s.register_camera(game_camera());
        s.register_camera(editor_camera());
        let views = s.build_views(1280, 720, 0.0);
        assert_eq!(views.len(), 1, "only the game camera should render in Game mode");
    }

    #[test]
    fn build_views_editor_mode_uses_editor_camera() {
        let mut s     = make_stratum(SimulationMode::Editor);
        let level_id  = s.create_level("world", 16.0, 48.0);
        populate_level(&mut s, level_id);
        s.register_camera(game_camera());
        s.register_camera(editor_camera());
        let views = s.build_views(1280, 720, 0.0);
        assert_eq!(views.len(), 1, "only the editor camera should render in Editor mode");
    }

    #[test]
    fn build_views_no_cameras_returns_empty() {
        let mut s    = make_stratum(SimulationMode::Game);
        let level_id = s.create_level("world", 16.0, 48.0);
        populate_level(&mut s, level_id);
        // No cameras registered
        assert!(s.build_views(1280, 720, 0.0).is_empty());
    }

    #[test]
    fn build_views_inactive_camera_not_rendered() {
        let mut s     = make_stratum(SimulationMode::Game);
        let level_id  = s.create_level("world", 16.0, 48.0);
        populate_level(&mut s, level_id);
        let mut cam   = game_camera();
        cam.active    = false;
        s.register_camera(cam);
        assert!(s.build_views(1280, 720, 0.0).is_empty());
    }

    // ── build_views — multi-camera ────────────────────────────────────────────

    #[test]
    fn build_views_two_game_cameras() {
        let mut s     = make_stratum(SimulationMode::Game);
        let level_id  = s.create_level("world", 16.0, 48.0);
        populate_level(&mut s, level_id);
        let mut c1    = game_camera();
        let mut c2    = game_camera();
        c1.kind = CameraKind::GameCamera { tag: "p1".into() };
        c2.kind = CameraKind::GameCamera { tag: "p2".into() };
        c1.viewport = Viewport::left_half();
        c2.viewport = Viewport::right_half();
        s.register_camera(c1);
        s.register_camera(c2);
        let views = s.build_views(1280, 720, 0.0);
        assert_eq!(views.len(), 2);
    }

    #[test]
    fn build_views_sorted_by_priority() {
        let mut s     = make_stratum(SimulationMode::Game);
        let level_id  = s.create_level("world", 16.0, 48.0);
        populate_level(&mut s, level_id);
        let mut hi    = game_camera();
        let mut lo    = game_camera();
        hi.kind = CameraKind::GameCamera { tag: "hi".into() };
        lo.kind = CameraKind::GameCamera { tag: "lo".into() };
        hi.priority = 10;
        lo.priority = -1;
        s.register_camera(hi);
        s.register_camera(lo);
        let views = s.build_views(1280, 720, 0.0);
        assert_eq!(views.len(), 2);
        assert!(views[0].priority <= views[1].priority);
    }

    // ── mode-switch live ──────────────────────────────────────────────────────

    #[test]
    fn mode_switch_changes_which_cameras_render() {
        let mut s     = make_stratum(SimulationMode::Game);
        let level_id  = s.create_level("world", 16.0, 48.0);
        populate_level(&mut s, level_id);
        s.register_camera(game_camera());
        s.register_camera(editor_camera());

        // Game mode → 1 game camera
        assert_eq!(s.build_views(1280, 720, 0.0).len(), 1);

        // Switch to Editor → 1 editor camera
        s.set_mode(SimulationMode::Editor);
        assert_eq!(s.build_views(1280, 720, 0.0).len(), 1);

        // Switch back → game camera again
        s.set_mode(SimulationMode::Game);
        assert_eq!(s.build_views(1280, 720, 0.0).len(), 1);
    }

    // ── tick / simulation_time ────────────────────────────────────────────────

    #[test]
    fn tick_advances_simulation_time_in_game_mode() {
        let mut s = make_stratum(SimulationMode::Game);
        s.create_level("world", 16.0, 32.0);
        s.register_camera(game_camera());
        s.tick(0.016);
        assert!((s.simulation_time() - 0.016).abs() < 1e-6);
    }

    #[test]
    fn tick_does_not_advance_time_in_editor_mode() {
        let mut s = make_stratum(SimulationMode::Editor);
        s.create_level("world", 16.0, 32.0);
        s.register_camera(editor_camera());
        s.tick(0.016);
        assert_eq!(s.simulation_time(), 0.0);
    }

    #[test]
    fn tick_accumulates_over_multiple_frames() {
        let mut s = make_stratum(SimulationMode::Game);
        s.create_level("world", 16.0, 32.0);
        s.register_camera(game_camera());
        for _ in 0..60 {
            s.tick(1.0 / 60.0);
        }
        assert!((s.simulation_time() - 1.0).abs() < 0.001);
    }

    // ── Visibility regression tests ───────────────────────────────────────────

    /// Exact mirror of `stratum_basic` setup — confirms entities survive
    /// the full tick → build_views → frustum-cull pipeline.
    #[test]
    fn demo_scene_entities_survive_frustum_cull() {
        let mut s    = make_stratum(SimulationMode::Game);
        let level_id = s.create_level("demo", 16.0, 48.0);

        // Same positions as stratum_basic.rs
        {
            let level = s.level_mut(level_id).unwrap();
            level.spawn_entity(Components::new()
                .with_transform(Transform::from_position(Vec3::new( 0.0, 0.5,  0.0)))
                .with_mesh(MeshHandle(1)));
            level.spawn_entity(Components::new()
                .with_transform(Transform::from_position(Vec3::new(-2.0, 0.4, -1.0)))
                .with_mesh(MeshHandle(2)));
            level.spawn_entity(Components::new()
                .with_transform(Transform::from_position(Vec3::new( 2.0, 0.3,  0.5)))
                .with_mesh(MeshHandle(3)));
            level.spawn_entity(Components::new()
                .with_transform(Transform::from_position(Vec3::new( 0.0, 0.0,  0.0)))
                .with_mesh(MeshHandle(4)));
            level.activate_all_chunks();
        }

        // Same camera as stratum_basic.rs
        let mut cam = game_camera();
        cam.position = Vec3::new(0.0, 2.5, 7.0);
        cam.yaw      = 0.0;
        cam.pitch    = -0.2;
        s.register_camera(cam);

        // Mirror frame loop: tick first, then build_views
        s.tick(0.016);
        let views = s.build_views(1280, 720, 0.0);

        assert_eq!(views.len(), 1, "one game camera → one view");
        assert!(
            !views[0].visible_entities.is_empty(),
            "entities at (0,0.5,0), (-2,0.4,-1), (2,0.3,0.5), (0,0,0) \
             should pass frustum cull for camera at (0,2.5,7) yaw=0 pitch=-0.2"
        );
    }

    /// Light entities must also survive the cull pass.
    #[test]
    fn demo_lights_survive_frustum_cull() {
        use crate::entity::LightData;

        let mut s    = make_stratum(SimulationMode::Game);
        let level_id = s.create_level("demo", 16.0, 48.0);

        {
            let level = s.level_mut(level_id).unwrap();
            level.spawn_entity(Components::new()
                .with_transform(Transform::from_position(Vec3::new( 0.0, 2.2, 0.0)))
                .with_light(LightData::Point { color: [1.0,0.55,0.15], intensity: 6.0, range: 5.0 }));
            level.spawn_entity(Components::new()
                .with_transform(Transform::from_position(Vec3::new(-3.5, 2.0,-1.5)))
                .with_light(LightData::Point { color: [0.25,0.5,1.0],  intensity: 5.0, range: 6.0 }));
            level.spawn_entity(Components::new()
                .with_transform(Transform::from_position(Vec3::new( 3.5, 1.5, 1.5)))
                .with_light(LightData::Point { color: [1.0,0.3,0.5],   intensity: 5.0, range: 6.0 }));
            level.activate_all_chunks();
        }

        let mut cam = game_camera();
        cam.position = Vec3::new(0.0, 2.5, 7.0);
        cam.yaw      = 0.0;
        cam.pitch    = -0.2;
        s.register_camera(cam);

        s.tick(0.016);
        let views = s.build_views(1280, 720, 0.0);

        assert_eq!(views.len(), 1);
        assert!(
            !views[0].visible_entities.is_empty(),
            "point lights near origin should pass frustum cull"
        );
    }
}
