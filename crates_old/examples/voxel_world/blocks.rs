//! Block types for the voxel world.
//!
//! Each variant maps to a unique material index stored in chunk entity records.
//! Indices 1–41 are used; 0 is reserved for air (not stored).

/// Block discriminant stored in chunk JSON as `material` field index.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Block {
    // ── Original 8 ──
    Grass,
    Dirt,
    Stone,
    Wood,
    Leaves,
    StoneBrick,
    Plank,
    Glass,
    // ── Biome additions ──
    Sand,
    Sandstone,
    Snow,
    Ice,
    DarkWood,
    DarkLeaves,
    Clay,
    Cobblestone,
    MossyStone,
    Water,
    Cactus,
    Thatch,
    // ── Geology ──
    Bedrock,
    Gravel,
    CoalOre,
    IronOre,
    GoldOre,
    DiamondOre,
    // ── Biome-specific wood/leaves ──
    BirchLog,
    BirchLeaves,
    SpruceLog,
    SpruceLeaves,
    JungleLog,
    JungleLeaves,
    // ── Surface & decoration ──
    Podzol,
    Pumpkin,
    Melon,
    RedMushroom,
    BrownMushroom,
    Obsidian,
    LilyPad,
    SugarCane,
    DeadBush,
    Poppy,
    Dandelion,
}

impl Block {
    /// Returns `true` for blocks that can be seen through (water, glass, ice,
    /// leaves, vegetation, etc.).  The face culler uses this to keep faces
    /// between an opaque block and a transparent neighbour.
    pub fn is_transparent(self) -> bool {
        matches!(
            self,
            Block::Water
            | Block::Glass
            | Block::Ice
            | Block::Leaves
            | Block::DarkLeaves
            | Block::BirchLeaves
            | Block::SpruceLeaves
            | Block::JungleLeaves
            | Block::LilyPad
            | Block::SugarCane
            | Block::DeadBush
            | Block::Poppy
            | Block::Dandelion
            | Block::RedMushroom
            | Block::BrownMushroom
        )
    }

    pub fn mat_index(self) -> u64 {
        match self {
            Block::Grass       => 1,
            Block::Dirt        => 2,
            Block::Stone       => 3,
            Block::Wood        => 4,
            Block::Leaves      => 5,
            Block::StoneBrick  => 6,
            Block::Plank       => 7,
            Block::Glass       => 8,
            Block::Sand        => 9,
            Block::Sandstone   => 10,
            Block::Snow        => 11,
            Block::Ice         => 12,
            Block::DarkWood    => 13,
            Block::DarkLeaves  => 14,
            Block::Clay        => 15,
            Block::Cobblestone => 16,
            Block::MossyStone  => 17,
            Block::Water       => 18,
            Block::Cactus      => 19,
            Block::Thatch      => 20,
            Block::Bedrock     => 21,
            Block::Gravel      => 22,
            Block::CoalOre     => 23,
            Block::IronOre     => 24,
            Block::GoldOre     => 25,
            Block::DiamondOre  => 26,
            Block::BirchLog    => 27,
            Block::BirchLeaves => 28,
            Block::SpruceLog   => 29,
            Block::SpruceLeaves => 30,
            Block::JungleLog   => 31,
            Block::JungleLeaves => 32,
            Block::Podzol      => 33,
            Block::Pumpkin     => 34,
            Block::Melon       => 35,
            Block::RedMushroom => 36,
            Block::BrownMushroom => 37,
            Block::Obsidian    => 38,
            Block::LilyPad    => 39,
            Block::SugarCane  => 40,
            Block::DeadBush   => 41,
            Block::Poppy     => 42,
            Block::Dandelion => 43,
        }
    }

    pub fn from_mat_index(n: u64) -> Option<Self> {
        match n {
            1  => Some(Block::Grass),
            2  => Some(Block::Dirt),
            3  => Some(Block::Stone),
            4  => Some(Block::Wood),
            5  => Some(Block::Leaves),
            6  => Some(Block::StoneBrick),
            7  => Some(Block::Plank),
            8  => Some(Block::Glass),
            9  => Some(Block::Sand),
            10 => Some(Block::Sandstone),
            11 => Some(Block::Snow),
            12 => Some(Block::Ice),
            13 => Some(Block::DarkWood),
            14 => Some(Block::DarkLeaves),
            15 => Some(Block::Clay),
            16 => Some(Block::Cobblestone),
            17 => Some(Block::MossyStone),
            18 => Some(Block::Water),
            19 => Some(Block::Cactus),
            20 => Some(Block::Thatch),
            21 => Some(Block::Bedrock),
            22 => Some(Block::Gravel),
            23 => Some(Block::CoalOre),
            24 => Some(Block::IronOre),
            25 => Some(Block::GoldOre),
            26 => Some(Block::DiamondOre),
            27 => Some(Block::BirchLog),
            28 => Some(Block::BirchLeaves),
            29 => Some(Block::SpruceLog),
            30 => Some(Block::SpruceLeaves),
            31 => Some(Block::JungleLog),
            32 => Some(Block::JungleLeaves),
            33 => Some(Block::Podzol),
            34 => Some(Block::Pumpkin),
            35 => Some(Block::Melon),
            36 => Some(Block::RedMushroom),
            37 => Some(Block::BrownMushroom),
            38 => Some(Block::Obsidian),
            39 => Some(Block::LilyPad),
            40 => Some(Block::SugarCane),
            41 => Some(Block::DeadBush),
            42 => Some(Block::Poppy),
            43 => Some(Block::Dandelion),
            _  => None,
        }
    }
}
