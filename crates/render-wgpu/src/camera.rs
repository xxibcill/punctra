use glam::{
    DVec3, Mat4, Vec3,
    camera::rh::{proj::directx, view},
};
use thiserror::Error;

/// A validated perspective camera expressed in 64-bit world coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    eye: [f64; 3],
    target: [f64; 3],
    up: [f64; 3],
    vertical_field_of_view_radians: f32,
    near_distance: f32,
    far_distance: f32,
}

impl Camera {
    /// Constructs a perspective camera after validating its numeric model.
    ///
    /// # Errors
    ///
    /// Returns [`CameraError`] when a vector is non-finite or degenerate, the
    /// field of view is outside `(0, pi)`, or the clipping range is invalid.
    pub fn perspective(
        eye: [f64; 3],
        target: [f64; 3],
        up: [f64; 3],
        vertical_field_of_view_radians: f32,
        near_distance: f32,
        far_distance: f32,
    ) -> Result<Self, CameraError> {
        validate_finite_vector("eye", eye)?;
        validate_finite_vector("target", target)?;
        validate_finite_vector("up", up)?;

        let forward = DVec3::from_array(target) - DVec3::from_array(eye);
        let up_vector = DVec3::from_array(up);
        if forward.length_squared() == 0.0 {
            return Err(CameraError::CoincidentEyeAndTarget);
        }
        if up_vector.length_squared() == 0.0 {
            return Err(CameraError::ZeroUpVector);
        }
        if forward.cross(up_vector).length_squared() == 0.0 {
            return Err(CameraError::ParallelUpVector);
        }
        if !vertical_field_of_view_radians.is_finite()
            || !(0.0..std::f32::consts::PI).contains(&vertical_field_of_view_radians)
        {
            return Err(CameraError::InvalidFieldOfView(
                vertical_field_of_view_radians,
            ));
        }
        if !near_distance.is_finite() || near_distance <= 0.0 {
            return Err(CameraError::InvalidNearDistance(near_distance));
        }
        if !far_distance.is_finite() || far_distance <= near_distance {
            return Err(CameraError::InvalidFarDistance {
                near: near_distance,
                far: far_distance,
            });
        }

        Ok(Self {
            eye,
            target,
            up,
            vertical_field_of_view_radians,
            near_distance,
            far_distance,
        })
    }

    /// Returns the camera position in world coordinates.
    #[must_use]
    pub const fn eye(&self) -> [f64; 3] {
        self.eye
    }

    /// Returns the look-at target in world coordinates.
    #[must_use]
    pub const fn target(&self) -> [f64; 3] {
        self.target
    }

    /// Returns the camera up direction.
    #[must_use]
    pub const fn up(&self) -> [f64; 3] {
        self.up
    }

    /// Returns the vertical field of view in radians.
    #[must_use]
    pub const fn vertical_field_of_view_radians(&self) -> f32 {
        self.vertical_field_of_view_radians
    }

    /// Returns the near clipping distance.
    #[must_use]
    pub const fn near_distance(&self) -> f32 {
        self.near_distance
    }

    /// Returns the far clipping distance.
    #[must_use]
    pub const fn far_distance(&self) -> f32 {
        self.far_distance
    }

    pub(crate) fn view_projection(&self, aspect_ratio: f32) -> Mat4 {
        let eye = DVec3::from_array(self.eye);
        let relative_target = (DVec3::from_array(self.target) - eye).as_vec3();
        let relative_up = DVec3::from_array(self.up).normalize().as_vec3();
        let view = view::look_at_mat4(Vec3::ZERO, relative_target, relative_up);
        let projection = directx::perspective(
            self.vertical_field_of_view_radians,
            aspect_ratio,
            self.near_distance,
            self.far_distance,
        );
        projection * view
    }
}

/// A camera construction error.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum CameraError {
    /// A world-coordinate vector contains NaN or infinity.
    #[error("camera {name} must contain only finite values")]
    NonFiniteVector {
        /// The invalid vector's name.
        name: &'static str,
    },
    /// The camera position and target are identical.
    #[error("camera eye and target must differ")]
    CoincidentEyeAndTarget,
    /// The up vector has zero length.
    #[error("camera up vector must be non-zero")]
    ZeroUpVector,
    /// The up vector is parallel to the viewing direction.
    #[error("camera up vector must not be parallel to its viewing direction")]
    ParallelUpVector,
    /// The vertical field of view is outside `(0, pi)`.
    #[error("camera field of view must be finite and inside (0, pi), got {0}")]
    InvalidFieldOfView(f32),
    /// The near clipping distance is not positive and finite.
    #[error("camera near distance must be positive and finite, got {0}")]
    InvalidNearDistance(f32),
    /// The far clipping distance does not exceed the near distance.
    #[error("camera far distance {far} must be finite and greater than near distance {near}")]
    InvalidFarDistance {
        /// The configured near distance.
        near: f32,
        /// The invalid far distance.
        far: f32,
    },
}

fn validate_finite_vector(name: &'static str, vector: [f64; 3]) -> Result<(), CameraError> {
    if vector.into_iter().all(f64::is_finite) {
        Ok(())
    } else {
        Err(CameraError::NonFiniteVector { name })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_camera() -> Camera {
        Camera::perspective(
            [1_000_000.0, 2_000_000.0, 100.0],
            [1_000_001.0, 2_000_000.0, 99.0],
            [0.0, 0.0, 1.0],
            std::f32::consts::FRAC_PI_3,
            0.1,
            10_000.0,
        )
        .expect("fixture camera should be valid")
    }

    #[test]
    fn rejects_a_non_finite_world_position() {
        let result = Camera::perspective(
            [f64::NAN, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            1.0,
            0.1,
            100.0,
        );

        assert_eq!(result, Err(CameraError::NonFiniteVector { name: "eye" }));
    }

    #[test]
    fn rejects_degenerate_look_at_vectors() {
        let coincident = Camera::perspective([0.0; 3], [0.0; 3], [0.0, 0.0, 1.0], 1.0, 0.1, 100.0);
        let parallel =
            Camera::perspective([0.0; 3], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0], 1.0, 0.1, 100.0);

        assert_eq!(coincident, Err(CameraError::CoincidentEyeAndTarget));
        assert_eq!(parallel, Err(CameraError::ParallelUpVector));
    }

    #[test]
    fn builds_a_finite_large_world_matrix() {
        let matrix = valid_camera().view_projection(16.0 / 9.0);

        assert!(matrix.to_cols_array().into_iter().all(f32::is_finite));
    }
}
