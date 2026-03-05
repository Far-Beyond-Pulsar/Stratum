//! PBR material palette — one material per block type.

use stratum::MaterialHandle;
use stratum_helio::{HelioIntegration, Material};

use crate::blocks::Block;

/// Holds all GPU material handles, one per `Block` variant.
pub struct MaterialPalette {
    pub grass:       MaterialHandle,
    pub dirt:        MaterialHandle,
    pub stone:       MaterialHandle,
    pub wood:        MaterialHandle,
    pub leaves:      MaterialHandle,
    pub stone_brick: MaterialHandle,
    pub plank:       MaterialHandle,
    pub glass:       MaterialHandle,
    pub sand:        MaterialHandle,
    pub sandstone:   MaterialHandle,
    pub snow:        MaterialHandle,
    pub ice:         MaterialHandle,
    pub dark_wood:   MaterialHandle,
    pub dark_leaves: MaterialHandle,
    pub clay:        MaterialHandle,
    pub cobblestone: MaterialHandle,
    pub mossy_stone: MaterialHandle,
    pub water:       MaterialHandle,
    pub cactus:      MaterialHandle,
    pub thatch:      MaterialHandle,
}

impl MaterialPalette {
    /// Create all materials and register them in the asset registry.
    pub fn new(integration: &mut HelioIntegration) -> Self {
        let m = |int: &mut HelioIntegration, color: [f32; 4], roughness: f32| {
            let g = int.create_material(&Material::new().with_base_color(color).with_roughness(roughness));
            int.assets_mut().add_material(g)
        };
        Self {
            grass:       m(integration, [0.24, 0.55, 0.16, 1.0], 0.90),
            dirt:        m(integration, [0.47, 0.30, 0.14, 1.0], 1.00),
            stone:       m(integration, [0.50, 0.50, 0.50, 1.0], 0.85),
            wood:        m(integration, [0.40, 0.25, 0.10, 1.0], 0.80),
            leaves:      m(integration, [0.15, 0.45, 0.10, 1.0], 0.95),
            stone_brick: m(integration, [0.55, 0.52, 0.48, 1.0], 0.80),
            plank:       m(integration, [0.62, 0.45, 0.22, 1.0], 0.75),
            glass:       m(integration, [0.60, 0.80, 0.90, 0.5], 0.10),
            sand:        m(integration, [0.82, 0.75, 0.55, 1.0], 0.95),
            sandstone:   m(integration, [0.78, 0.68, 0.45, 1.0], 0.85),
            snow:        m(integration, [0.92, 0.93, 0.95, 1.0], 0.90),
            ice:         m(integration, [0.70, 0.85, 0.95, 0.8], 0.05),
            dark_wood:   m(integration, [0.25, 0.15, 0.08, 1.0], 0.80),
            dark_leaves: m(integration, [0.08, 0.30, 0.05, 1.0], 0.95),
            clay:        m(integration, [0.55, 0.42, 0.35, 1.0], 0.95),
            cobblestone: m(integration, [0.45, 0.45, 0.42, 1.0], 0.90),
            mossy_stone: m(integration, [0.35, 0.45, 0.30, 1.0], 0.90),
            water:       m(integration, [0.15, 0.35, 0.60, 0.6], 0.05),
            cactus:      m(integration, [0.18, 0.50, 0.12, 1.0], 0.85),
            thatch:      m(integration, [0.72, 0.60, 0.30, 1.0], 0.95),
        }
    }

    /// Map a `Block` to its registered `MaterialHandle`.
    pub fn handle_for(&self, block: Block) -> MaterialHandle {
        match block {
            Block::Grass       => self.grass,
            Block::Dirt        => self.dirt,
            Block::Stone       => self.stone,
            Block::Wood        => self.wood,
            Block::Leaves      => self.leaves,
            Block::StoneBrick  => self.stone_brick,
            Block::Plank       => self.plank,
            Block::Glass       => self.glass,
            Block::Sand        => self.sand,
            Block::Sandstone   => self.sandstone,
            Block::Snow        => self.snow,
            Block::Ice         => self.ice,
            Block::DarkWood    => self.dark_wood,
            Block::DarkLeaves  => self.dark_leaves,
            Block::Clay        => self.clay,
            Block::Cobblestone => self.cobblestone,
            Block::MossyStone  => self.mossy_stone,
            Block::Water       => self.water,
            Block::Cactus      => self.cactus,
            Block::Thatch      => self.thatch,
        }
    }
}
