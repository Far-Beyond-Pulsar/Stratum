//! Simulation mode — controls which cameras render and whether gameplay ticks.

/// The operational mode of the Stratum world.
///
/// Mode is hot-switchable via `Stratum::set_mode()` or
/// `Stratum::toggle_mode()` at any point during a session.
///
/// | Camera kind           | Editor | Game |
/// |-----------------------|--------|------|
/// | `EditorPerspective`   | ✓      | ✗    |
/// | `EditorOrthographic`  | ✓      | ✗    |
/// | `GameCamera`          | ✗      | ✓    |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimulationMode {
    /// Editor mode: editor cameras render; gameplay simulation is paused.
    ///
    /// Only `EditorPerspective` and `EditorOrthographic` cameras produce
    /// render views. Game cameras are hidden.
    Editor,

    /// Game mode: the runtime simulation is active.
    ///
    /// Only `GameCamera` instances produce render views.
    /// Gameplay systems (owned by the application) should call their own
    /// update logic; Stratum merely provides the world state.
    Game,
}

impl SimulationMode {
    #[inline] pub fn is_editor(&self) -> bool { matches!(self, Self::Editor) }
    #[inline] pub fn is_game  (&self) -> bool { matches!(self, Self::Game)   }

    /// Toggle between `Editor` and `Game`.
    pub fn toggle(&mut self) {
        *self = match self {
            Self::Editor => Self::Game,
            Self::Game   => Self::Editor,
        };
        log::info!("SimulationMode → {:?}", self);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_is_editor() {
        assert!(SimulationMode::Editor.is_editor());
        assert!(!SimulationMode::Editor.is_game());
    }

    #[test]
    fn game_is_game() {
        assert!(SimulationMode::Game.is_game());
        assert!(!SimulationMode::Game.is_editor());
    }

    #[test]
    fn toggle_editor_to_game() {
        let mut mode = SimulationMode::Editor;
        mode.toggle();
        assert!(mode.is_game());
    }

    #[test]
    fn toggle_game_to_editor() {
        let mut mode = SimulationMode::Game;
        mode.toggle();
        assert!(mode.is_editor());
    }

    #[test]
    fn toggle_twice_returns_to_original() {
        let mut mode = SimulationMode::Editor;
        mode.toggle();
        mode.toggle();
        assert!(mode.is_editor());
    }
}
