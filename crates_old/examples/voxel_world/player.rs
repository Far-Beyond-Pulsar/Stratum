//! Player physics — walking, gravity, AABB collision vs voxels.

use glam::Vec3;
use std::collections::HashSet;
use winit::keyboard::KeyCode;

use crate::blocks::Block;
use crate::chunks::VoxelChunkManager;
use crate::terrain::{terrain_height, CHUNK_SIZE, VOXELS_PER_CHUNK, WATER_LEVEL};
use crate::biomes::Biome;

// ── Player constants ────────────────────────────────────────────────────────

pub const PLAYER_WALK_SPEED: f32   = 4.5;
pub const PLAYER_SPRINT_SPEED: f32 = 8.5;
pub const PLAYER_CROUCH_SPEED: f32 = 2.0;
pub const PLAYER_AIR_ACCEL: f32    = 0.8;
pub const PLAYER_GRAVITY: f32      = 22.0;
pub const PLAYER_JUMP_VELOCITY: f32 = 8.0;
pub const PLAYER_HEIGHT: f32       = 1.8;
pub const PLAYER_CROUCH_HEIGHT: f32 = 1.2;
pub const PLAYER_RADIUS: f32       = 0.35;
pub const GROUND_STICK_DIST: f32   = 0.1;
pub const DOUBLE_TAP_WINDOW: f32   = 0.3; // seconds

// ── Player state ────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct Player {
    pub position: Vec3,
    pub velocity: Vec3,
    pub yaw:      f32,
    pub pitch:    f32,
    pub grounded: bool,
    pub sprinting: bool,
    pub crouching: bool,
    last_w_press: f32, // time of last W key press for double-tap detection
    prev_w_state: bool, // previous frame's W key state (for edge detection)
}

impl Player {
    pub fn new(position: Vec3) -> Self {
        Self {
            position,
            velocity: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            grounded: false,
            sprinting: false,
            crouching: false,
            last_w_press: -999.0,
            prev_w_state: false,
        }
    }

    pub fn spawn_at_surface(wx: f32, wz: f32) -> Self {
        let h = terrain_height(wx as i32, wz as i32);
        let spawn_y = (h + 2) as f32;
        Self::new(Vec3::new(wx, spawn_y, wz))
    }

    pub fn forward(&self) -> Vec3 {
        // Match camera convention: yaw=0 looks down -Z, yaw increases CCW
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(sy, 0.0, -cy).normalize()
    }

    pub fn right(&self) -> Vec3 {
        // Perpendicular to forward in XZ plane, matching camera convention
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(cy, 0.0, sy).normalize()
    }

    /// Camera eye position (head height)
    pub fn eye_pos(&self) -> Vec3 {
        let height = if self.crouching { PLAYER_CROUCH_HEIGHT } else { PLAYER_HEIGHT };
        self.position + Vec3::new(0.0, height * 0.85, 0.0)
    }
    
    /// Current player height (affected by crouching)
    pub fn height(&self) -> f32 {
        if self.crouching { PLAYER_CROUCH_HEIGHT } else { PLAYER_HEIGHT }
    }
    
    /// Current movement speed (affected by sprint/crouch)
    pub fn move_speed(&self) -> f32 {
        if self.sprinting {
            PLAYER_SPRINT_SPEED
        } else if self.crouching {
            PLAYER_CROUCH_SPEED
        } else {
            PLAYER_WALK_SPEED
        }
    }
}

// ── Collision detection ─────────────────────────────────────────────────────

/// Check if a voxel block exists at exact world position.
fn is_solid_at(wx: i32, wy: i32, wz: i32, chunks: &VoxelChunkManager) -> bool {
    // Out of bounds → treat as solid below y=0, empty above world
    if wy < 0 { return true; }
    if wy > 63 { return false; }

    // Check if chunk is loaded
    let cx = wx.div_euclid(VOXELS_PER_CHUNK);
    let cy = wy.div_euclid(VOXELS_PER_CHUNK);
    let cz = wz.div_euclid(VOXELS_PER_CHUNK);
    let coord = stratum::chunk::ChunkCoord::new(cx, cy, cz);

    // If chunk loaded, check the solid voxel data (includes structures!)
    if let Some(solid_map) = chunks.solids.get(&coord) {
        if let Some(&block) = solid_map.get(&(wx, wy, wz)) {
            // Water is not solid for player
            if matches!(block, Block::Water) {
                return false;
            }
            return true;
        }
        return false; // Chunk loaded but this voxel is air
    }

    // If chunk not loaded, fall back to procedural terrain height
    let surface_h = terrain_height(wx, wz);
    wy <= surface_h && wy != WATER_LEVEL
}

/// AABB collision sweep: returns first hit time t ∈ [0,1] along direction `dir`.
fn sweep_aabb_voxels(
    center: Vec3,
    radius: f32,
    height: f32,
    dir: Vec3,
    chunks: &VoxelChunkManager,
) -> Option<f32> {
    if dir.length_squared() < 1e-6 { return None; }
    
    let steps = (dir.length() * 4.0).ceil() as i32; // 0.25m substeps
    let step_dir = dir / steps as f32;
    
    for i in 1..=steps {
        let test_pos = center + step_dir * i as f32;
        if collides_aabb_voxels(test_pos, radius, height, chunks) {
            return Some((i - 1) as f32 / steps as f32);
        }
    }
    None
}

/// Check if player capsule at `center` overlaps any solid voxels.
/// Uses proper capsule-AABB intersection to avoid asymmetric hitboxes.
fn collides_aabb_voxels(
    center: Vec3,
    radius: f32,
    height: f32,
    chunks: &VoxelChunkManager,
) -> bool {
    // Expand search region conservatively (add extra margin for safety)
    let margin = 1.0;
    let min_x = (center.x - radius - margin).floor() as i32;
    let max_x = (center.x + radius + margin).ceil() as i32;
    let min_y = (center.y - margin).floor() as i32;
    let max_y = (center.y + height + margin).ceil() as i32;
    let min_z = (center.z - radius - margin).floor() as i32;
    let max_z = (center.z + radius + margin).ceil() as i32;

    for bx in min_x..=max_x {
        for by in min_y..=max_y {
            for bz in min_z..=max_z {
                if !is_solid_at(bx, by, bz, chunks) {
                    continue;
                }
                
                // Check if this voxel's AABB actually intersects the capsule
                if voxel_intersects_capsule(bx, by, bz, center, radius, height) {
                    return true;
                }
            }
        }
    }
    false
}

/// Test if a voxel cube [bx..bx+1, by..by+1, bz..bz+1] intersects a capsule.
/// Capsule is a vertical cylinder: center at `center`, horizontal radius `radius`, height `height`.
fn voxel_intersects_capsule(
    bx: i32,
    by: i32,
    bz: i32,
    center: Vec3,
    radius: f32,
    height: f32,
) -> bool {
    // Voxel AABB bounds
    let voxel_min = Vec3::new(bx as f32, by as f32, bz as f32);
    let voxel_max = voxel_min + Vec3::ONE;

    // Capsule vertical range
    let capsule_y_min = center.y;
    let capsule_y_max = center.y + height;

    // Quick Y-axis reject
    if voxel_max.y < capsule_y_min || voxel_min.y > capsule_y_max {
        return false;
    }

    // Find closest point on voxel AABB to capsule axis (vertical line at center.x, center.z)
    let closest_x = center.x.clamp(voxel_min.x, voxel_max.x);
    let closest_z = center.z.clamp(voxel_min.z, voxel_max.z);

    // Horizontal distance from capsule axis to closest point
    let dx = closest_x - center.x;
    let dz = closest_z - center.z;
    let dist_sq = dx * dx + dz * dz;

    // If closest point is within capsule radius, we have a collision
    dist_sq <= radius * radius
}

// ── Player physics update ───────────────────────────────────────────────────

pub fn update_player(
    player: &mut Player,
    keys: &HashSet<KeyCode>,
    chunks: &VoxelChunkManager,
    dt: f32,
    time: f32, // game time in seconds for double-tap detection
) {
    let dt = dt.min(0.1); // Cap to avoid huge timesteps

    // -- Sprint/Crouch state --
    // Crouch: hold Shift
    player.crouching = keys.contains(&KeyCode::ShiftLeft) || keys.contains(&KeyCode::ShiftRight);
    
    // Sprint: double-tap W (detect rising edge of W key)
    let w_pressed = keys.contains(&KeyCode::KeyW);
    let w_just_pressed = w_pressed && !player.prev_w_state;
    
    if w_just_pressed && !player.crouching {
        // Check if this is a double-tap (W pressed within double-tap window)
        if time - player.last_w_press < DOUBLE_TAP_WINDOW {
            player.sprinting = true;
        }
        player.last_w_press = time;
    }
    player.prev_w_state = w_pressed;
    
    // Cancel sprint if not moving forward or if crouching
    if !w_pressed || player.crouching {
        player.sprinting = false;
    }

    // -- Input velocities --
    let fwd = player.forward();
    let right = player.right();
    
    let mut wish_dir = Vec3::ZERO;
    if keys.contains(&KeyCode::KeyW) { wish_dir += fwd; }
    if keys.contains(&KeyCode::KeyS) { wish_dir -= fwd; }
    if keys.contains(&KeyCode::KeyA) { wish_dir -= right; }
    if keys.contains(&KeyCode::KeyD) { wish_dir += right; }
    
    if wish_dir.length_squared() > 0.01 {
        wish_dir = wish_dir.normalize();
    }

    // -- Grounded check (raycast down slightly) --
    let feet_pos = player.position;
    let check_below = feet_pos - Vec3::Y * (GROUND_STICK_DIST + 0.01);
    player.grounded = collides_aabb_voxels(
        check_below, 
        PLAYER_RADIUS, 
        player.height(), 
        chunks
    );

    // -- Vertical movement --
    if player.grounded {
        player.velocity.y = -1.0; // Small downward to stick to ground
        if keys.contains(&KeyCode::Space) && !player.crouching {
            player.velocity.y = PLAYER_JUMP_VELOCITY;
            player.grounded = false;
        }
    } else {
        player.velocity.y -= PLAYER_GRAVITY * dt;
    }

    // -- Horizontal movement --
    let move_speed = if player.grounded { 
        player.move_speed()
    } else { 
        PLAYER_AIR_ACCEL 
    };
    
    let target_xz = wish_dir * move_speed;
    if player.grounded {
        // Instant ground movement (optional: add acceleration)
        player.velocity.x = target_xz.x;
        player.velocity.z = target_xz.z;
    } else {
        // Air control
        player.velocity.x += (target_xz.x - player.velocity.x) * PLAYER_AIR_ACCEL * dt;
        player.velocity.z += (target_xz.z - player.velocity.z) * PLAYER_AIR_ACCEL * dt;
    }

    // -- Collision resolution (axis-aligned slide) --
    let mut displacement = player.velocity * dt;
    let player_height = player.height();
    
    // X axis
    let test_x = player.position + Vec3::new(displacement.x, 0.0, 0.0);
    if collides_aabb_voxels(test_x, PLAYER_RADIUS, player_height, chunks) {
        displacement.x = 0.0;
        player.velocity.x = 0.0;
    }

    // Y axis
    let test_y = player.position + Vec3::new(0.0, displacement.y, 0.0);
    if collides_aabb_voxels(test_y, PLAYER_RADIUS, player_height, chunks) {
        if player.velocity.y < 0.0 {
            player.grounded = true;
        }
        displacement.y = 0.0;
        player.velocity.y = 0.0;
    }

    // Z axis
    let test_z = player.position + Vec3::new(0.0, 0.0, displacement.z);
    if collides_aabb_voxels(test_z, PLAYER_RADIUS, player_height, chunks) {
        displacement.z = 0.0;
        player.velocity.z = 0.0;
    }

    player.position += displacement;

    // Safety: prevent falling through world
    if player.position.y < -10.0 {
        let h = terrain_height(player.position.x as i32, player.position.z as i32);
        player.position.y = (h + 3) as f32;
        player.velocity = Vec3::ZERO;
    }
}
