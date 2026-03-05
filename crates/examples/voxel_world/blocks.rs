//! Block types for the voxel world.
//!
//! Each variant maps to a unique material index stored in chunk entity records.
//! Indices 1–20 are used; 0 is reserved for air (not stored).

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
}

impl Block {
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
            _  => None,
        }
    }
}
