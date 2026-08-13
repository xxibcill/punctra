use glam::{
    DVec3, Vec3,
    camera::rh::{proj::directx, view},
};
use thiserror::Error;

/// Canonical orthonormal world-space basis of a validated camera.
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

/// The projection model owned by a [`Camera`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CameraProjection {
    /// Perspective projection with a vertical angular field of view.
    Perspective {
        /// Vertical field of view in radians inside `(0, pi)`.
        vertical_field_of_view_radians: f32,
    },
    /// Orthographic projection with a vertical extent in world units.
    Orthographic {
        /// Positive finite world height visible before aspect-ratio scaling.
        vertical_world_height: f64,
    },
}

/// A validated perspective or orthographic camera in 64-bit world coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    eye: [f64; 3],
    target: [f64; 3],
    up: [f64; 3],
    world_basis: CameraBasis,
    projection: CameraProjection,
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
        Self::new(
            eye,
            target,
            up,
            CameraProjection::Perspective {
                vertical_field_of_view_radians,
            },
            near_distance,
            far_distance,
        )
    }

    /// Constructs an orthographic camera after validating its numeric model.
    ///
    /// `vertical_world_height` is the world-space height visible in the
    /// viewport. The horizontal extent is that height multiplied by the
    /// viewport aspect ratio.
    ///
    /// # Errors
    ///
    /// Returns [`CameraError`] when a vector is non-finite or degenerate, the
    /// vertical world height is not positive and finite, the clipping range is
    /// invalid, or the parameters cannot produce a finite projection.
    pub fn orthographic(
        eye: [f64; 3],
        target: [f64; 3],
        up: [f64; 3],
        vertical_world_height: f64,
        near_distance: f32,
        far_distance: f32,
    ) -> Result<Self, CameraError> {
        Self::new(
            eye,
            target,
            up,
            CameraProjection::Orthographic {
                vertical_world_height,
            },
            near_distance,
            far_distance,
        )
    }

    fn new(
        eye: [f64; 3],
        target: [f64; 3],
        up: [f64; 3],
        projection: CameraProjection,
        near_distance: f32,
        far_distance: f32,
    ) -> Result<Self, CameraError> {
        let world_basis = validated_world_basis(eye, target, up)?;
        validate_projection(projection)?;
        validate_clipping_distances(near_distance, far_distance)?;

        let camera = Self {
            eye,
            target,
            up,
            world_basis,
            projection,
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

    /// Returns the explicit perspective or orthographic projection model.
    #[must_use]
    pub const fn projection(&self) -> CameraProjection {
        self.projection
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
        let forward = DVec3::from_array(self.world_basis.forward).as_vec3();
        let up = DVec3::from_array(self.world_basis.up).as_vec3();
        let view = view::look_at_mat4(Vec3::ZERO, forward, up);
        let projection = self.projection_matrix(aspect_ratio);
        let view_projection = projection * view;
        if view_projection.is_finite() {
            Ok(view_projection.to_cols_array())
        } else {
            Err(CameraError::NonFiniteProjection)
        }
    }

    fn projection_matrix(&self, aspect_ratio: f32) -> glam::Mat4 {
        match self.projection {
            CameraProjection::Perspective {
                vertical_field_of_view_radians,
            } => directx::perspective(
                vertical_field_of_view_radians,
                aspect_ratio,
                self.near_distance,
                self.far_distance,
            ),
            CameraProjection::Orthographic {
                vertical_world_height,
            } => {
                #[allow(clippy::cast_possible_truncation)]
                let half_vertical = (vertical_world_height * 0.5) as f32;
                let half_horizontal = half_vertical * aspect_ratio;
                directx::orthographic(
                    -half_horizontal,
                    half_horizontal,
                    -half_vertical,
                    half_vertical,
                    self.near_distance,
                    self.far_distance,
                )
            }
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
    /// The orthographic vertical world height is not positive and finite.
    #[error("camera orthographic world height must be positive and finite, got {0}")]
    InvalidOrthographicWorldHeight(f64),
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

fn validated_world_basis(
    eye: [f64; 3],
    target: [f64; 3],
    up: [f64; 3],
) -> Result<CameraBasis, CameraError> {
    validate_finite_vector("eye", eye)?;
    validate_finite_vector("target", target)?;
    validate_finite_vector("up", up)?;

    let forward = DVec3::from_array(target) - DVec3::from_array(eye);
    let requested_up = DVec3::from_array(up);
    if forward == DVec3::ZERO {
        return Err(CameraError::CoincidentEyeAndTarget);
    }
    if requested_up == DVec3::ZERO {
        return Err(CameraError::ZeroUpVector);
    }

    let forward = normalize_world_direction(forward).ok_or(CameraError::NonFiniteViewDirection)?;
    let requested_up = normalize_world_direction(requested_up).ok_or(CameraError::ZeroUpVector)?;
    validate_narrowed_basis(forward, requested_up)?;
    let right = normalize_world_direction(forward.cross(requested_up))
        .ok_or(CameraError::ParallelUpVector)?;
    let up = right.cross(forward);
    Ok(CameraBasis {
        forward: forward.to_array(),
        right: right.to_array(),
        up: up.to_array(),
    })
}

fn validate_narrowed_basis(forward: DVec3, requested_up: DVec3) -> Result<(), CameraError> {
    let narrowed_forward = forward.as_vec3();
    let narrowed_up = requested_up.as_vec3();
    let narrowed_right = narrowed_forward
        .cross(narrowed_up)
        .try_normalize()
        .ok_or(CameraError::ParallelUpVector)?;
    if narrowed_right.cross(narrowed_forward).is_finite() {
        Ok(())
    } else {
        Err(CameraError::ParallelUpVector)
    }
}

fn validate_projection(projection: CameraProjection) -> Result<(), CameraError> {
    match projection {
        CameraProjection::Perspective {
            vertical_field_of_view_radians,
        } if !vertical_field_of_view_radians.is_finite()
            || !(0.0..std::f32::consts::PI).contains(&vertical_field_of_view_radians) =>
        {
            Err(CameraError::InvalidFieldOfView(
                vertical_field_of_view_radians,
            ))
        }
        CameraProjection::Orthographic {
            vertical_world_height,
        } if !vertical_world_height.is_finite() || vertical_world_height <= 0.0 => Err(
            CameraError::InvalidOrthographicWorldHeight(vertical_world_height),
        ),
        _ => Ok(()),
    }
}

fn validate_clipping_distances(near: f32, far: f32) -> Result<(), CameraError> {
    if !near.is_finite() || near <= 0.0 {
        return Err(CameraError::InvalidNearDistance(near));
    }
    if !far.is_finite() || far <= near {
        return Err(CameraError::InvalidFarDistance { near, far });
    }
    Ok(())
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
    fn orthographic_projection_validates_its_world_height() {
        for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let result = Camera::orthographic(
                [0.0, 0.0, 10.0],
                [0.0; 3],
                [0.0, 1.0, 0.0],
                invalid,
                0.1,
                100.0,
            );

            assert!(matches!(
                result,
                Err(CameraError::InvalidOrthographicWorldHeight(actual))
                    if actual.to_bits() == invalid.to_bits()
            ));
        }
    }

    #[test]
    fn orthographic_projection_maps_world_extents_and_depth() {
        let camera =
            Camera::orthographic([0.0, 0.0, 10.0], [0.0; 3], [0.0, 1.0, 0.0], 10.0, 1.0, 21.0)
                .unwrap();
        let matrix = glam::Mat4::from_cols_array(&camera.view_projection_matrix(2.0).unwrap());

        let right_edge = matrix * glam::Vec4::new(10.0, 0.0, -1.0, 1.0);
        let top_edge = matrix * glam::Vec4::new(0.0, 5.0, -1.0, 1.0);
        let near = matrix * glam::Vec4::new(0.0, 0.0, -1.0, 1.0);
        let far = matrix * glam::Vec4::new(0.0, 0.0, -21.0, 1.0);

        assert!((right_edge.x - 1.0).abs() < f32::EPSILON * 4.0);
        assert!((top_edge.y - 1.0).abs() < f32::EPSILON * 4.0);
        assert!(near.z.abs() < f32::EPSILON * 4.0);
        assert!((far.z - 1.0).abs() < f32::EPSILON * 4.0);
        assert!((right_edge.w - 1.0).abs() < f32::EPSILON * 4.0);
        assert!((top_edge.w - 1.0).abs() < f32::EPSILON * 4.0);
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
    fn projection_uses_the_canonical_basis_for_near_parallel_up_vectors() {
        let camera = Camera::perspective(
            [0.0; 3],
            [1.0, 1.0, 1.0],
            [0.999_999_95, 1.000_000_04, 1.000_000_05],
            1.0,
            1.0,
            100.0,
        )
        .unwrap();
        let point = DVec3::new(
            7.267_795_159_728_823_5,
            0.196_727_468_879_519_3,
            9.855_985_447_080_432,
        );
        let basis = camera.world_basis();
        let forward = DVec3::from_array(basis.forward());
        let right = DVec3::from_array(basis.right());
        let depth = forward.dot(point);
        let CameraProjection::Perspective {
            vertical_field_of_view_radians,
        } = camera.projection()
        else {
            panic!("the fixture camera should be perspective");
        };
        let half_field_of_view = 0.5 * f64::from(vertical_field_of_view_radians);
        let horizontal_limit = depth * half_field_of_view.tan();
        assert!(right.dot(point).abs() > horizontal_limit);

        let matrix = glam::Mat4::from_cols_array(&camera.view_projection_matrix(1.0).unwrap());
        let clip = matrix * point.as_vec3().extend(1.0);
        assert!(clip.x.abs() > clip.w);
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
