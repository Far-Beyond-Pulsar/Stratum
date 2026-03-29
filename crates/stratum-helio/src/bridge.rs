use glam::Vec3;

/// Convert a Stratum world-space camera to an abstract external camera.
/// This is a thin shim: actual Helio API may differ, but this is the expected
/// conversion layer for rendering integration.
pub fn stratum_camera_to_external<P>(position: Vec3, view_matrix: P, projection_matrix: P) -> (Vec3, P, P) {
    (position, view_matrix, projection_matrix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Vec3};

    #[test]
    fn bridge_camera_conversion_roundtrip() {
        let pos = Vec3::new(1.0, 2.0, 3.0);
        let view = Mat4::IDENTITY;
        let proj = Mat4::IDENTITY;
        let (p, v, pr) = stratum_camera_to_external(pos, view, proj);
        assert_eq!(p, pos);
        assert_eq!(v, Mat4::IDENTITY);
        assert_eq!(pr, Mat4::IDENTITY);
    }
}
