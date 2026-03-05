//! Prefab catalogue — all reusable voxel structures.

pub mod trees;
pub mod houses;
pub mod village;
pub mod towers;
pub mod waterways;

use glam::Vec3;
use stratum::{Components, MaterialHandle, Prefab, Transform};

use crate::blocks::Block;

// ── Helper ──────────────────────────────────────────────────────────────────

/// Create a single-voxel entity in prefab-local space.
pub fn voxel_entity(lx: f32, ly: f32, lz: f32, block: Block) -> Components {
    Components::new()
        .with_transform(Transform::from_position(Vec3::new(lx + 0.5, ly + 0.5, lz + 0.5)))
        .with_material(MaterialHandle(block.mat_index()))
        .with_bounding_radius(0.866)
}

// ── Library ─────────────────────────────────────────────────────────────────

/// All prefabs used for world generation, built once at startup.
pub struct PrefabLibrary {
    // Trees
    pub oak:          Prefab,
    pub pine:         Prefab,
    pub birch:        Prefab,
    pub willow:       Prefab,
    pub jungle_tree:  Prefab,
    pub cactus:       Prefab,
    pub dead_tree:    Prefab,

    // Houses
    pub cottage:      Prefab,
    pub manor:        Prefab,
    pub tower_house:  Prefab,
    pub market_stall: Prefab,
    pub barn:         Prefab,
    pub desert_house: Prefab,
    pub taiga_cabin:  Prefab,

    // Village infrastructure
    pub well:         Prefab,
    pub fountain:     Prefab,
    pub lamppost:     Prefab,

    // Towers
    pub ruined_tower: Prefab,
    pub watchtower:   Prefab,
    pub lighthouse:   Prefab,

    // Waterways
    pub aqueduct_pillar: Prefab,
    pub aqueduct_span:   Prefab,
    pub dock:            Prefab,
}

impl PrefabLibrary {
    pub fn new() -> Self {
        Self {
            // Trees
            oak:          trees::make_oak(),
            pine:         trees::make_pine(),
            birch:        trees::make_birch(),
            willow:       trees::make_willow(),
            jungle_tree:  trees::make_jungle_tree(),
            cactus:       trees::make_cactus(),
            dead_tree:    trees::make_dead_tree(),

            // Houses
            cottage:      houses::make_cottage(),
            manor:        houses::make_manor(),
            tower_house:  houses::make_tower_house(),
            market_stall: houses::make_market_stall(),
            barn:         houses::make_barn(),
            desert_house: houses::make_desert_house(),
            taiga_cabin:  houses::make_taiga_cabin(),

            // Village
            well:         village::make_well(),
            fountain:     village::make_fountain(),
            lamppost:     village::make_lamppost(),

            // Towers
            ruined_tower: towers::make_ruined_tower(),
            watchtower:   towers::make_watchtower(),
            lighthouse:   towers::make_lighthouse(),

            // Waterways
            aqueduct_pillar: waterways::make_aqueduct_pillar(),
            aqueduct_span:   waterways::make_aqueduct_span(),
            dock:            waterways::make_dock(),
        }
    }
}
