//! Block types for the voxel world.

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum Block {
    Air = 0,
    Grass = 1,
    Dirt = 2,
    Stone = 3,
    Cobblestone = 4,
    Sand = 5,
    Sandstone = 6,
    Water = 7,
    Wood = 8,
    Leaves = 9,
    Glass = 10,
    Bedrock = 11,
    Snow = 12,
    Ice = 13,
    Gravel = 14,
    CoalOre = 15,
    IronOre = 16,
    GoldOre = 17,
    DiamondOre = 18,
}

impl Block {
    /// Returns true if this block is transparent (doesn't cull adjacent faces).
    pub fn is_transparent(self) -> bool {
        matches!(
            self,
            Block::Air | Block::Water | Block::Glass | Block::Ice | Block::Leaves
        )
    }

    /// Returns true if this block is completely see-through (renders no faces).
    pub fn is_air(self) -> bool {
        matches!(self, Block::Air)
    }

    /// Get the texture name for this block (corresponds to PNG files in assets).
    pub fn texture_name(self) -> &'static str {
        match self {
            Block::Air => "air",
            Block::Grass => "grass_block_top",
            Block::Dirt => "dirt",
            Block::Stone => "stone",
            Block::Cobblestone => "cobblestone",
            Block::Sand => "sand",
            Block::Sandstone => "sandstone_top",
            Block::Water => "water_still",
            Block::Wood => "oak_log",
            Block::Leaves => "oak_leaves",
            Block::Glass => "glass",
            Block::Bedrock => "bedrock",
            Block::Snow => "snow",
            Block::Ice => "ice",
            Block::Gravel => "gravel",
            Block::CoalOre => "coal_ore",
            Block::IronOre => "iron_ore",
            Block::GoldOre => "gold_ore",
            Block::DiamondOre => "diamond_ore",
        }
    }

    /// Convert from u8 ID.
    pub fn from_id(id: u8) -> Self {
        match id {
            0 => Block::Air,
            1 => Block::Grass,
            2 => Block::Dirt,
            3 => Block::Stone,
            4 => Block::Cobblestone,
            5 => Block::Sand,
            6 => Block::Sandstone,
            7 => Block::Water,
            8 => Block::Wood,
            9 => Block::Leaves,
            10 => Block::Glass,
            11 => Block::Bedrock,
            12 => Block::Snow,
            13 => Block::Ice,
            14 => Block::Gravel,
            15 => Block::CoalOre,
            16 => Block::IronOre,
            17 => Block::GoldOre,
            18 => Block::DiamondOre,
            _ => Block::Air,
        }
    }
}
