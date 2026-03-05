//! Biome classification via temperature / moisture noise.

use crate::noise::smooth_noise;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Biome {
    Plains,
    Forest,
    Taiga,
    Desert,
    Swamp,
    Mountains,
    Jungle,
}

/// Temperature 0..1 (0 = cold, 1 = hot).
pub fn temperature_at(wx: f32, wz: f32) -> f32 {
    let t = smooth_noise(wx / 200.0, wz / 200.0, 100)
          + smooth_noise(wx / 100.0, wz / 100.0, 101) * 0.5;
    (t / 1.5).clamp(0.0, 1.0)
}

/// Moisture 0..1 (0 = dry, 1 = wet).
pub fn moisture_at(wx: f32, wz: f32) -> f32 {
    let m = smooth_noise(wx / 180.0, wz / 180.0, 110)
          + smooth_noise(wx /  90.0, wz /  90.0, 111) * 0.5;
    (m / 1.5).clamp(0.0, 1.0)
}

/// Classify the biome at a world-space XZ position.
pub fn biome_at(wx: i32, wz: i32, mountain_factor: f32) -> Biome {
    if mountain_factor > 0.15 { return Biome::Mountains; }

    let temp  = temperature_at(wx as f32, wz as f32);
    let moist = moisture_at(wx as f32, wz as f32);

    if temp > 0.62 && moist < 0.35 { return Biome::Desert;  }
    if temp > 0.62 && moist > 0.58 { return Biome::Jungle;  }
    if temp < 0.38 && moist > 0.35 { return Biome::Taiga;   }
    if moist > 0.68                { return Biome::Swamp;    }
    if moist > 0.48                { return Biome::Forest;   }
    Biome::Plains
}
