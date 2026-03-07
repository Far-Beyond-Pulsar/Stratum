//! PBR material palette — one material per block type with Faithful 64x textures.

use stratum::MaterialHandle;
use stratum_helio::{HelioIntegration, Material, TextureData};

use crate::blocks::Block;

// ── Texture loading ─────────────────────────────────────────────────────────

fn load_block_texture(path: &str) -> (Vec<u8>, u32, u32) {
    let asset_bytes: Option<&'static [u8]> = match path {
        "grass_block_top.png"    => Some(include_bytes!("assets/block/grass_block_top.png")),
        "dirt.png"               => Some(include_bytes!("assets/block/dirt.png")),
        "stone.png"              => Some(include_bytes!("assets/block/stone.png")),
        "oak_log.png"            => Some(include_bytes!("assets/block/oak_log.png")),
        "oak_leaves.png"         => Some(include_bytes!("assets/block/oak_leaves.png")),
        "stone_bricks.png"       => Some(include_bytes!("assets/block/stone_bricks.png")),
        "oak_planks.png"         => Some(include_bytes!("assets/block/oak_planks.png")),
        "glass.png"              => Some(include_bytes!("assets/block/glass.png")),
        "sand.png"               => Some(include_bytes!("assets/block/sand.png")),
        "sandstone_top.png"      => Some(include_bytes!("assets/block/sandstone_top.png")),
        "snow.png"               => Some(include_bytes!("assets/block/snow.png")),
        "ice.png"                => Some(include_bytes!("assets/block/ice.png")),
        "dark_oak_log.png"       => Some(include_bytes!("assets/block/dark_oak_log.png")),
        "dark_oak_leaves.png"    => Some(include_bytes!("assets/block/dark_oak_leaves.png")),
        "clay.png"               => Some(include_bytes!("assets/block/clay.png")),
        "cobblestone.png"        => Some(include_bytes!("assets/block/cobblestone.png")),
        "mossy_cobblestone.png"  => Some(include_bytes!("assets/block/mossy_cobblestone.png")),
        "cactus_side.png"        => Some(include_bytes!("assets/block/cactus_side.png")),
        "hay_block_top.png"      => Some(include_bytes!("assets/block/hay_block_top.png")),
        // ── New textures ──
        "bedrock.png"            => Some(include_bytes!("assets/block/bedrock.png")),
        "gravel.png"             => Some(include_bytes!("assets/block/gravel.png")),
        "coal_ore.png"           => Some(include_bytes!("assets/block/coal_ore.png")),
        "iron_ore.png"           => Some(include_bytes!("assets/block/iron_ore.png")),
        "gold_ore.png"           => Some(include_bytes!("assets/block/gold_ore.png")),
        "diamond_ore.png"        => Some(include_bytes!("assets/block/diamond_ore.png")),
        "birch_log.png"          => Some(include_bytes!("assets/block/birch_log.png")),
        "birch_leaves.png"       => Some(include_bytes!("assets/block/birch_leaves.png")),
        "spruce_log.png"         => Some(include_bytes!("assets/block/spruce_log.png")),
        "spruce_leaves.png"      => Some(include_bytes!("assets/block/spruce_leaves.png")),
        "jungle_log.png"         => Some(include_bytes!("assets/block/jungle_log.png")),
        "jungle_leaves.png"      => Some(include_bytes!("assets/block/jungle_leaves.png")),
        "podzol_top.png"         => Some(include_bytes!("assets/block/podzol_top.png")),
        "pumpkin_side.png"       => Some(include_bytes!("assets/block/pumpkin_side.png")),
        "melon_side.png"         => Some(include_bytes!("assets/block/melon_side.png")),
        "red_mushroom.png"       => Some(include_bytes!("assets/block/red_mushroom.png")),
        "brown_mushroom.png"     => Some(include_bytes!("assets/block/brown_mushroom.png")),
        "obsidian.png"           => Some(include_bytes!("assets/block/obsidian.png")),
        "lily_pad.png"           => Some(include_bytes!("assets/block/lily_pad.png")),
        "water_still.png"        => Some(include_bytes!("assets/block/water_still.png")),
        "sugar_cane.png"         => Some(include_bytes!("assets/block/sugar_cane.png")),
        "dead_bush.png"          => Some(include_bytes!("assets/block/dead_bush.png")),
        "poppy.png"              => Some(include_bytes!("assets/block/poppy.png")),
        "dandelion.png"          => Some(include_bytes!("assets/block/dandelion.png")),
        _ => None,
    };

    let img = asset_bytes
        .and_then(|bytes| image::load_from_memory(bytes).ok())
        .unwrap_or_else(|| {
            log::warn!("Could not load texture '{}', using white fallback", path);
            let mut px = image::RgbaImage::new(1, 1);
            px.put_pixel(0, 0, image::Rgba([128, 128, 128, 255]));
            image::DynamicImage::ImageRgba8(px)
        })
        .into_rgba8();
    let (w, h) = img.dimensions();
    (img.into_raw(), w, h)
}

/// Holds all GPU material handles, one per `Block` variant.
pub struct MaterialPalette {
    pub grass:          MaterialHandle,
    pub dirt:           MaterialHandle,
    pub stone:          MaterialHandle,
    pub wood:           MaterialHandle,
    pub leaves:         MaterialHandle,
    pub stone_brick:    MaterialHandle,
    pub plank:          MaterialHandle,
    pub glass:          MaterialHandle,
    pub sand:           MaterialHandle,
    pub sandstone:      MaterialHandle,
    pub snow:           MaterialHandle,
    pub ice:            MaterialHandle,
    pub dark_wood:      MaterialHandle,
    pub dark_leaves:    MaterialHandle,
    pub clay:           MaterialHandle,
    pub cobblestone:    MaterialHandle,
    pub mossy_stone:    MaterialHandle,
    pub water:          MaterialHandle,
    pub cactus:         MaterialHandle,
    pub thatch:         MaterialHandle,
    // ── New materials ──
    pub bedrock:        MaterialHandle,
    pub gravel:         MaterialHandle,
    pub coal_ore:       MaterialHandle,
    pub iron_ore:       MaterialHandle,
    pub gold_ore:       MaterialHandle,
    pub diamond_ore:    MaterialHandle,
    pub birch_log:      MaterialHandle,
    pub birch_leaves:   MaterialHandle,
    pub spruce_log:     MaterialHandle,
    pub spruce_leaves:  MaterialHandle,
    pub jungle_log:     MaterialHandle,
    pub jungle_leaves:  MaterialHandle,
    pub podzol:         MaterialHandle,
    pub pumpkin:        MaterialHandle,
    pub melon:          MaterialHandle,
    pub red_mushroom:   MaterialHandle,
    pub brown_mushroom: MaterialHandle,
    pub obsidian:       MaterialHandle,
    pub lily_pad:       MaterialHandle,
    pub sugar_cane:     MaterialHandle,
    pub dead_bush:      MaterialHandle,
    pub poppy:          MaterialHandle,
    pub dandelion:      MaterialHandle,
}

impl MaterialPalette {
    /// Create all materials with Faithful 64x textures and register them in the asset registry.
    pub fn new(integration: &mut HelioIntegration) -> Self {
        let m = |int: &mut HelioIntegration, tex_path: &str, roughness: f32| {
            let (tex_rgba, w, h) = load_block_texture(tex_path);
            let g = int.create_material(
                &Material::new()
                    .with_base_color_texture(TextureData::new(tex_rgba, w, h))
                    .with_roughness(roughness),
            );
            int.assets_mut().add_material(g)
        };
        // Tinted material for grayscale textures (grass, leaves)
        let m_tinted = |int: &mut HelioIntegration, tex_path: &str, roughness: f32, tint: [f32; 4]| {
            let (tex_rgba, w, h) = load_block_texture(tex_path);
            let g = int.create_material(
                &Material::new()
                    .with_base_color(tint)
                    .with_base_color_texture(TextureData::new(tex_rgba, w, h))
                    .with_roughness(roughness),
            );
            int.assets_mut().add_material(g)
        };
        Self {
            grass:       m_tinted(integration, "grass_block_top.png", 0.50, [0.52, 0.82, 0.35, 1.0]),
            dirt:        m(integration, "dirt.png", 0.65),
            stone:       m(integration, "stone.png", 0.55),
            wood:        m(integration, "oak_log.png", 0.55),
            leaves:      m_tinted(integration, "oak_leaves.png", 0.60, [0.48, 0.78, 0.32, 1.0]),
            stone_brick: m(integration, "stone_bricks.png", 0.60),
            plank:       m(integration, "oak_planks.png", 0.50),
            glass:       m(integration, "glass.png", 0.05),
            sand:        m(integration, "sand.png", 0.65),
            sandstone:   m(integration, "sandstone_top.png", 0.55),
            snow:        m(integration, "snow.png", 0.40),
            ice:         m(integration, "ice.png", 0.05),
            dark_wood:   m(integration, "dark_oak_log.png", 0.55),
            dark_leaves: m_tinted(integration, "dark_oak_leaves.png", 0.60, [0.38, 0.68, 0.28, 1.0]),
            clay:        m(integration, "clay.png", 0.70),
            cobblestone: m(integration, "cobblestone.png", 0.65),
            mossy_stone: m(integration, "mossy_cobblestone.png", 0.65),
            water:       m_tinted(integration, "water_still.png", 0.02, [0.15, 0.38, 0.85, 0.75]),
            cactus:      m(integration, "cactus_side.png", 0.55),
            thatch:      m(integration, "hay_block_top.png", 0.60),
            // ── New materials ──
            bedrock:      m(integration, "bedrock.png", 0.80),
            gravel:       m(integration, "gravel.png", 0.70),
            coal_ore:     m(integration, "coal_ore.png", 0.60),
            iron_ore:     m(integration, "iron_ore.png", 0.55),
            gold_ore:     m(integration, "gold_ore.png", 0.45),
            diamond_ore:  m(integration, "diamond_ore.png", 0.35),
            birch_log:    m(integration, "birch_log.png", 0.55),
            birch_leaves: m_tinted(integration, "birch_leaves.png", 0.60, [0.55, 0.80, 0.38, 1.0]),
            spruce_log:   m(integration, "spruce_log.png", 0.55),
            spruce_leaves: m_tinted(integration, "spruce_leaves.png", 0.60, [0.28, 0.52, 0.22, 1.0]),
            jungle_log:   m(integration, "jungle_log.png", 0.55),
            jungle_leaves: m_tinted(integration, "jungle_leaves.png", 0.60, [0.30, 0.72, 0.18, 1.0]),
            podzol:       m(integration, "podzol_top.png", 0.65),
            pumpkin:      m(integration, "pumpkin_side.png", 0.55),
            melon:        m(integration, "melon_side.png", 0.55),
            red_mushroom: m(integration, "red_mushroom.png", 0.60),
            brown_mushroom: m(integration, "brown_mushroom.png", 0.60),
            obsidian:     m(integration, "obsidian.png", 0.15),
            lily_pad:     m_tinted(integration, "lily_pad.png", 0.55, [0.30, 0.70, 0.20, 1.0]),
            sugar_cane:   m_tinted(integration, "sugar_cane.png", 0.55, [0.45, 0.80, 0.35, 1.0]),
            dead_bush:    m(integration, "dead_bush.png", 0.65),
            poppy:        m(integration, "poppy.png", 0.55),
            dandelion:    m(integration, "dandelion.png", 0.55),
        }
    }

    /// Map a `Block` to its registered `MaterialHandle`.
    pub fn handle_for(&self, block: Block) -> MaterialHandle {
        match block {
            Block::Grass        => self.grass,
            Block::Dirt         => self.dirt,
            Block::Stone        => self.stone,
            Block::Wood         => self.wood,
            Block::Leaves       => self.leaves,
            Block::StoneBrick   => self.stone_brick,
            Block::Plank        => self.plank,
            Block::Glass        => self.glass,
            Block::Sand         => self.sand,
            Block::Sandstone    => self.sandstone,
            Block::Snow         => self.snow,
            Block::Ice          => self.ice,
            Block::DarkWood     => self.dark_wood,
            Block::DarkLeaves   => self.dark_leaves,
            Block::Clay         => self.clay,
            Block::Cobblestone  => self.cobblestone,
            Block::MossyStone   => self.mossy_stone,
            Block::Water        => self.water,
            Block::Cactus       => self.cactus,
            Block::Thatch       => self.thatch,
            Block::Bedrock      => self.bedrock,
            Block::Gravel       => self.gravel,
            Block::CoalOre      => self.coal_ore,
            Block::IronOre      => self.iron_ore,
            Block::GoldOre      => self.gold_ore,
            Block::DiamondOre   => self.diamond_ore,
            Block::BirchLog     => self.birch_log,
            Block::BirchLeaves  => self.birch_leaves,
            Block::SpruceLog    => self.spruce_log,
            Block::SpruceLeaves => self.spruce_leaves,
            Block::JungleLog    => self.jungle_log,
            Block::JungleLeaves => self.jungle_leaves,
            Block::Podzol       => self.podzol,
            Block::Pumpkin      => self.pumpkin,
            Block::Melon        => self.melon,
            Block::RedMushroom  => self.red_mushroom,
            Block::BrownMushroom => self.brown_mushroom,
            Block::Obsidian     => self.obsidian,
            Block::LilyPad      => self.lily_pad,
            Block::SugarCane    => self.sugar_cane,
            Block::DeadBush     => self.dead_bush,
            Block::Poppy        => self.poppy,
            Block::Dandelion    => self.dandelion,
        }
    }
}
