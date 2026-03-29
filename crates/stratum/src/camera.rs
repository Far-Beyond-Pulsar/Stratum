use crate::types::Frustum;
use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Unique identifier for a camera in the multi-camera system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CameraId(pub u64);

impl CameraId {
    /// Create a new camera ID from a simple index.
    pub fn new(index: u64) -> Self {
        Self(index)
    }
}

/// Camera configuration and transform for world partition streaming.
#[derive(Debug, Clone)]
pub struct Camera {
    /// Unique identifier for this camera.
    pub id: CameraId,
    /// Position in world space.
    pub position: Vec3,
    /// Rotation as a quaternion.
    pub rotation: Quat,
    /// Forward direction vector (cached).
    pub forward: Vec3,
    /// Field of view in radians.
    pub fov: f32,
    /// Aspect ratio (width / height).
    pub aspect_ratio: f32,
    /// Near clipping plane distance.
    pub near: f32,
    /// Far clipping plane distance.
    pub far: f32,
    /// View matrix (cached).
    pub view_matrix: Mat4,
    /// Projection matrix (cached).
    pub projection_matrix: Mat4,
    /// Combined view-projection matrix (cached).
    pub view_proj_matrix: Mat4,
    /// Frustum for culling (cached).
    pub frustum: Frustum,
    /// Velocity vector for predictive loading.
    pub velocity: Vec3,
    /// Priority weight for this camera (higher = more important for loading decisions).
    pub priority: f32,
}

impl Camera {
    /// Create a new camera with default settings.
    pub fn new(id: CameraId, position: Vec3, rotation: Quat) -> Self {
        let mut camera = Self {
            id,
            position,
            rotation,
            forward: Vec3::NEG_Z,
            fov: std::f32::consts::FRAC_PI_4, // 45 degrees
            aspect_ratio: 16.0 / 9.0,
            near: 0.1,
            far: 1000.0,
            view_matrix: Mat4::IDENTITY,
            projection_matrix: Mat4::IDENTITY,
            view_proj_matrix: Mat4::IDENTITY,
            frustum: Frustum { planes: [glam::Vec4::ZERO; 6] },
            velocity: Vec3::ZERO,
            priority: 1.0,
        };

        camera.update_matrices();
        camera
    }

    /// Update the camera's position and rotation.
    pub fn set_transform(&mut self, position: Vec3, rotation: Quat) {
        self.position = position;
        self.rotation = rotation;
        self.update_matrices();
    }

    /// Set the camera's projection parameters.
    pub fn set_projection(&mut self, fov: f32, aspect_ratio: f32, near: f32, far: f32) {
        self.fov = fov;
        self.aspect_ratio = aspect_ratio;
        self.near = near;
        self.far = far;
        self.update_matrices();
    }

    /// Update velocity for predictive loading (call each frame with dt).
    pub fn update_velocity(&mut self, new_position: Vec3, dt: f32) {
        if dt > 0.0 {
            self.velocity = (new_position - self.position) / dt;
        }
        self.position = new_position;
        self.update_matrices();
    }

    /// Predict future position based on current velocity.
    pub fn predict_position(&self, time_ahead: f32) -> Vec3 {
        self.position + self.velocity * time_ahead
    }

    /// Update all cached matrices and frustum.
    fn update_matrices(&mut self) {
        // Update forward vector
        self.forward = self.rotation * Vec3::NEG_Z;

        // Compute view matrix
        let up = self.rotation * Vec3::Y;
        self.view_matrix = Mat4::look_at_rh(
            self.position,
            self.position + self.forward,
            up,
        );

        // Compute projection matrix
        self.projection_matrix = Mat4::perspective_rh(
            self.fov,
            self.aspect_ratio,
            self.near,
            self.far,
        );

        // Compute combined view-projection matrix
        self.view_proj_matrix = self.projection_matrix * self.view_matrix;

        // Update frustum
        self.frustum = Frustum::from_matrix(self.view_proj_matrix);
    }

    /// Get the right direction vector.
    pub fn right(&self) -> Vec3 {
        self.rotation * Vec3::X
    }

    /// Get the up direction vector.
    pub fn up(&self) -> Vec3 {
        self.rotation * Vec3::Y
    }
}

/// Registry for managing multiple cameras in the world partition system.
pub struct CameraRegistry {
    /// Map of camera ID to camera instance.
    cameras: HashMap<CameraId, Camera>,
    /// Next camera ID to assign.
    next_id: u64,
    /// Primary camera ID (used for prioritized loading).
    primary_camera: Option<CameraId>,
}

impl CameraRegistry {
    /// Create a new empty camera registry.
    pub fn new() -> Self {
        Self {
            cameras: HashMap::new(),
            next_id: 0,
            primary_camera: None,
        }
    }

    /// Register a new camera and return its ID.
    pub fn register_camera(&mut self, position: Vec3, rotation: Quat) -> CameraId {
        let id = CameraId::new(self.next_id);
        self.next_id += 1;

        let camera = Camera::new(id, position, rotation);
        self.cameras.insert(id, camera);

        // Set as primary if this is the first camera
        if self.primary_camera.is_none() {
            self.primary_camera = Some(id);
        }

        id
    }

    /// Unregister a camera.
    pub fn unregister_camera(&mut self, id: CameraId) -> Option<Camera> {
        let camera = self.cameras.remove(&id);

        // Clear primary if it was the removed camera
        if self.primary_camera == Some(id) {
            self.primary_camera = self.cameras.keys().next().copied();
        }

        camera
    }

    /// Get a camera by ID.
    pub fn get_camera(&self, id: CameraId) -> Option<&Camera> {
        self.cameras.get(&id)
    }

    /// Get a mutable reference to a camera by ID.
    pub fn get_camera_mut(&mut self, id: CameraId) -> Option<&mut Camera> {
        self.cameras.get_mut(&id)
    }

    /// Get the primary camera.
    pub fn primary_camera(&self) -> Option<&Camera> {
        self.primary_camera.and_then(|id| self.cameras.get(&id))
    }

    /// Get a mutable reference to the primary camera.
    pub fn primary_camera_mut(&mut self) -> Option<&mut Camera> {
        self.primary_camera.and_then(|id| self.cameras.get_mut(&id))
    }

    /// Set the primary camera.
    pub fn set_primary_camera(&mut self, id: CameraId) -> Result<(), CameraError> {
        if self.cameras.contains_key(&id) {
            self.primary_camera = Some(id);
            Ok(())
        } else {
            Err(CameraError::CameraNotFound(id))
        }
    }

    /// Iterate over all cameras.
    pub fn cameras(&self) -> impl Iterator<Item = &Camera> {
        self.cameras.values()
    }

    /// Iterate over all cameras mutably.
    pub fn cameras_mut(&mut self) -> impl Iterator<Item = &mut Camera> {
        self.cameras.values_mut()
    }

    /// Get the number of registered cameras.
    pub fn camera_count(&self) -> usize {
        self.cameras.len()
    }

    /// Get all camera IDs.
    pub fn camera_ids(&self) -> Vec<CameraId> {
        self.cameras.keys().copied().collect()
    }
}

impl Default for CameraRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors related to camera operations.
#[derive(Debug, thiserror::Error)]
pub enum CameraError {
    #[error("Camera not found: {0:?}")]
    CameraNotFound(CameraId),

    #[error("No primary camera set")]
    NoPrimaryCamera,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_camera_creation() {
        let camera = Camera::new(
            CameraId::new(0),
            Vec3::new(0.0, 10.0, 0.0),
            Quat::IDENTITY,
        );

        assert_eq!(camera.position, Vec3::new(0.0, 10.0, 0.0));
        assert_eq!(camera.priority, 1.0);
    }

    #[test]
    fn test_camera_velocity_tracking() {
        let mut camera = Camera::new(
            CameraId::new(0),
            Vec3::ZERO,
            Quat::IDENTITY,
        );

        camera.update_velocity(Vec3::new(10.0, 0.0, 0.0), 1.0);
        assert_eq!(camera.velocity, Vec3::new(10.0, 0.0, 0.0));

        let predicted = camera.predict_position(2.0);
        assert_eq!(predicted, Vec3::new(30.0, 0.0, 0.0)); // 10 + 10*2
    }

    #[test]
    fn test_camera_registry() {
        let mut registry = CameraRegistry::new();

        let id1 = registry.register_camera(Vec3::ZERO, Quat::IDENTITY);
        let id2 = registry.register_camera(Vec3::new(10.0, 0.0, 0.0), Quat::IDENTITY);

        assert_eq!(registry.camera_count(), 2);
        assert_eq!(registry.primary_camera().unwrap().id, id1);

        registry.set_primary_camera(id2).unwrap();
        assert_eq!(registry.primary_camera().unwrap().id, id2);

        registry.unregister_camera(id1);
        assert_eq!(registry.camera_count(), 1);
    }
}
