//! Deterministic noise primitives — no external dependencies.

/// Fast integer hash → [0, 1).
pub fn hash(x: i32, z: i32, seed: u64) -> f32 {
    let mut v = (x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
              ^ (z as u64).wrapping_mul(0x6c62_272e_07bb_0142)
              ^ seed;
    v ^= v >> 30; v = v.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    v ^= v >> 27; v = v.wrapping_mul(0x94d0_49bb_1331_11eb);
    v ^= v >> 31;
    (v as f32) / (u64::MAX as f32)
}

/// Hermite-interpolated value noise on a unit grid.
pub fn smooth_noise(x: f32, z: f32, seed: u64) -> f32 {
    let xi = x.floor() as i32;
    let zi = z.floor() as i32;
    let ux = { let f = x - xi as f32; f * f * (3.0 - 2.0 * f) };
    let uz = { let f = z - zi as f32; f * f * (3.0 - 2.0 * f) };
    let a = hash(xi,   zi,   seed);
    let b = hash(xi+1, zi,   seed);
    let c = hash(xi,   zi+1, seed);
    let d = hash(xi+1, zi+1, seed);
    (a + ux * (b - a)) + uz * ((c + ux * (d - c)) - (a + ux * (b - a)))
}

/// Fractional Brownian motion — layered smooth noise.
pub fn fbm(x: f32, z: f32, seed: u64, octaves: u32, freq: f32, lacunarity: f32, gain: f32) -> f32 {
    let mut value   = 0.0_f32;
    let mut amp     = 1.0_f32;
    let mut f       = freq;
    let mut max_amp = 0.0_f32;
    for i in 0..octaves {
        value   += smooth_noise(x * f, z * f, seed + i as u64) * amp;
        max_amp += amp;
        f       *= lacunarity;
        amp     *= gain;
    }
    value / max_amp
}
