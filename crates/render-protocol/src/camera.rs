use glam::{
    DVec3, Vec3,
    camera::rh::{proj::directx, view},
};
use thiserror::Error;

/// Canonical orthonormal world-space basis of a validated perspective camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraBasis {
    forward: [f64; 3],
    right: [f64; 3],
    up: [f64; 3],
}

impl CameraBasis {
    /// Returns the normalized direction from the camera eye toward its target.
    #[must_use]
    pub const fn forward(self) -> [f64; 3] {
        self.forward
    }

    /// Returns the normalized right direction.
    #[must_use]
    pub const fn right(self) -> [f64; 3] {
        self.right
    }

    /// Returns the normalized view-up direction orthogonal to forward and right.
    #[must_use]
    pub const fn up(self) -> [f64; 3] {
        self.up
    }
}

/// A validated perspective camera expressed in 64-bit world coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    eye: [f64; 3],
    target: [f64; 3],
    up: [f64; 3],
    world_basis: CameraBasis,
    view_direction: [f32; 3],
    view_up: [f32; 3],
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
    /// field of view is outside `(0, pi)`, the clipping range is invalid, or
    /// otherwise valid parameters would produce a non-finite projection.
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
        if forward == DVec3::ZERO {
            return Err(CameraError::CoincidentEyeAndTarget);
        }
        if up_vector == DVec3::ZERO {
            return Err(CameraError::ZeroUpVector);
        }
        let world_forward =
            normalize_world_direction(forward).ok_or(CameraError::NonFiniteViewDirection)?;
        let world_requested_up =
            normalize_world_direction(up_vector).ok_or(CameraError::ZeroUpVector)?;
        let view_direction = world_forward.as_vec3();
        let requested_up = world_requested_up.as_vec3();
        let view_right = view_direction
            .cross(requested_up)
            .try_normalize()
            .ok_or(CameraError::ParallelUpVector)?;
        let view_up = view_right.cross(view_direction);
        if !view_up.is_finite() {
            return Err(CameraError::ParallelUpVector);
        }
        let world_right = normalize_world_direction(world_forward.cross(world_requested_up))
            .ok_or(CameraError::ParallelUpVector)?;
        let world_up = world_right.cross(world_forward);
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

        let camera = Self {
            eye,
            target,
            up,
            world_basis: CameraBasis {
                forward: world_forward.to_array(),
                right: world_right.to_array(),
                up: world_up.to_array(),
            },
            view_direction: view_direction.to_array(),
            view_up: view_up.to_array(),
            vertical_field_of_view_radians,
            near_distance,
            far_distance,
        };
        camera.view_projection_matrix(1.0)?;
        Ok(camera)
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

    /// Returns the canonical orthonormal basis used for world-space planning.
    #[must_use]
    pub const fn world_basis(&self) -> CameraBasis {
        self.world_basis
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

    /// Builds the right-handed view-projection matrix for an aspect ratio.
    ///
    /// The returned flat array is column-major: each consecutive group of four
    /// values is one matrix column. Its clip-space depth range is `0..=1`.
    ///
    /// # Errors
    ///
    /// Returns [`CameraError::NonFiniteProjection`] when the aspect ratio and
    /// validated camera parameters do not produce a finite matrix.
    pub fn view_projection_matrix(&self, aspect_ratio: f32) -> Result<[f32; 16], CameraError> {
        let view = view::look_at_mat4(
            Vec3::ZERO,
            Vec3::from_array(self.view_direction),
            Vec3::from_array(self.view_up),
        );
        let projection = directx::perspective(
            self.vertical_field_of_view_radians,
            aspect_ratio,
            self.near_distance,
            self.far_distance,
        );
        let view_projection = projection * view;
        if view_projection.is_finite() {
            Ok(view_projection.to_cols_array())
        } else {
            Err(CameraError::NonFiniteProjection)
        }
    }
}

/// A camera construction or projection error.
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
    /// Subtracting the finite eye and target did not produce a finite direction.
    #[error("camera eye-to-target direction must remain finite")]
    NonFiniteViewDirection,
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
    /// The accepted camera parameters cannot produce a finite projection matrix.
    #[error("camera projection matrix must remain finite")]
    NonFiniteProjection,
}

fn validate_finite_vector(name: &'static str, vector: [f64; 3]) -> Result<(), CameraError> {
    if vector.into_iter().all(f64::is_finite) {
        Ok(())
    } else {
        Err(CameraError::NonFiniteVector { name })
    }
}

fn normalize_world_direction(vector: DVec3) -> Option<DVec3> {
    let scale = vector.abs().max_element();
    if !scale.is_finite() || scale == 0.0 {
        return None;
    }

    let normalized = (vector / scale).try_normalize()?;
    normalized.is_finite().then_some(normalized)
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
        let matrix = valid_camera()
            .view_projection_matrix(16.0 / 9.0)
            .expect("the validated camera should produce a finite projection");

        assert!(matrix.into_iter().all(f32::is_finite));
    }

    #[test]
    fn exposes_an_orthonormal_world_basis() {
        let basis = valid_camera().world_basis();
        let forward = DVec3::from_array(basis.forward());
        let right = DVec3::from_array(basis.right());
        let up = DVec3::from_array(basis.up());

        for direction in [forward, right, up] {
            assert!((direction.length() - 1.0).abs() < f64::EPSILON * 4.0);
        }
        assert!(forward.dot(right).abs() < f64::EPSILON * 4.0);
        assert!(forward.dot(up).abs() < f64::EPSILON * 4.0);
        assert!(right.dot(up).abs() < f64::EPSILON * 4.0);
    }

    #[test]
    fn normalizes_view_directions_before_narrowing_to_f32() {
        for distance in [1.0e-50, 1.0e40] {
            let camera = Camera::perspective(
                [0.0; 3],
                [distance, 0.0, 0.0],
                [0.0, 0.0, 1.0],
                1.0,
                0.1,
                100.0,
            )
            .expect("finite view directions should be normalized before narrowing");

            assert!(camera.view_projection_matrix(1.0).is_ok());
        }
    }

    #[test]
    fn rejects_directions_that_become_parallel_after_narrowing() {
        let result = Camera::perspective(
            [0.0; 3],
            [1.0, 0.0, 0.0],
            [1.0, 1.0e-50, 0.0],
            1.0,
            0.1,
            100.0,
        );

        assert_eq!(result, Err(CameraError::ParallelUpVector));
    }

    #[test]
    fn rejects_parameters_that_produce_non_finite_projection_matrices() {
        let tiny_field_of_view = Camera::perspective(
            [0.0, -5.0, 0.0],
            [0.0; 3],
            [0.0, 0.0, 1.0],
            f32::from_bits(1),
            0.1,
            100.0,
        );
        let overflowing_depth_range = Camera::perspective(
            [0.0, -5.0, 0.0],
            [0.0; 3],
            [0.0, 0.0, 1.0],
            1.0,
            1.0e20,
            2.0e20,
        );

        assert_eq!(tiny_field_of_view, Err(CameraError::NonFiniteProjection));
        assert_eq!(
            overflowing_depth_range,
            Err(CameraError::NonFiniteProjection)
        );
    }
}
