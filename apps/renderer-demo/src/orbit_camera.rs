use std::{
    f64::consts::{FRAC_PI_4, FRAC_PI_6},
    ffi::OsStr,
    fmt,
};

use render_wgpu::{Camera, CameraError};

const MIN_ELEVATION: f64 = 0.08;
const MAX_ELEVATION: f64 = 1.48;
const MIN_RADIUS: f64 = 20.0;
const MAX_RADIUS: f64 = 10_000.0;
const FAR_DISTANCE: f32 = 20_000.0;
const PERSPECTIVE_FIELD_OF_VIEW: f32 = std::f32::consts::FRAC_PI_4;
const ORBIT_RADIANS_PER_PIXEL: f64 = 0.006;
const ZOOM_EXPONENT_PER_LINE: f64 = 0.12;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ProjectionMode {
    #[default]
    Perspective,
    Orthographic,
}

impl ProjectionMode {
    pub(crate) fn parse(value: &OsStr) -> Option<Self> {
        if value == OsStr::new("perspective") {
            Some(Self::Perspective)
        } else if value == OsStr::new("orthographic") {
            Some(Self::Orthographic)
        } else {
            None
        }
    }
}

impl fmt::Display for ProjectionMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Perspective => formatter.write_str("perspective"),
            Self::Orthographic => formatter.write_str("orthographic"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NorthOrientation {
    Up,
    Down,
    Right,
    Left,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct OrbitCamera {
    home_target: [f64; 3],
    target: [f64; 3],
    azimuth: f64,
    elevation: f64,
    radius: f64,
    min_radius: f64,
    max_radius: f64,
    far_distance: f32,
    projection: ProjectionMode,
}

impl OrbitCamera {
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn new(target: [f64; 3], radius: f64) -> Self {
        let min_radius = MIN_RADIUS.min((radius * 0.01).max(0.01));
        let max_radius = MAX_RADIUS.max(radius * 4.0);
        let far_distance = FAR_DISTANCE.max((max_radius * 2.0) as f32);
        Self {
            home_target: target,
            target,
            azimuth: -FRAC_PI_4,
            elevation: FRAC_PI_6,
            radius,
            min_radius,
            max_radius,
            far_distance,
            projection: ProjectionMode::Perspective,
        }
    }

    pub(crate) fn with_projection(mut self, projection: ProjectionMode) -> Self {
        self.projection = projection;
        self
    }

    pub(crate) fn orbit(&mut self, horizontal_pixels: f64, vertical_pixels: f64) {
        self.azimuth -= horizontal_pixels * ORBIT_RADIANS_PER_PIXEL;
        self.elevation = (self.elevation + vertical_pixels * ORBIT_RADIANS_PER_PIXEL)
            .clamp(MIN_ELEVATION, MAX_ELEVATION);
    }

    pub(crate) fn zoom(&mut self, lines: f64) {
        let zoom_factor = (-lines * ZOOM_EXPONENT_PER_LINE).exp();
        self.radius = (self.radius * zoom_factor).clamp(self.min_radius, self.max_radius);
    }

    pub(crate) fn pan(
        &mut self,
        horizontal_pixels: f64,
        vertical_pixels: f64,
        viewport_height: u32,
    ) -> Result<(), CameraError> {
        if viewport_height == 0 {
            return Ok(());
        }
        let basis = self.as_render_camera()?.world_basis();
        let world_per_pixel = self.vertical_world_height() / f64::from(viewport_height);
        for (axis, target) in self.target.iter_mut().enumerate() {
            *target += -basis.right()[axis] * horizontal_pixels * world_per_pixel
                + basis.up()[axis] * vertical_pixels * world_per_pixel;
        }
        Ok(())
    }

    pub(crate) fn toggle_projection(&mut self) {
        self.projection = match self.projection {
            ProjectionMode::Perspective => ProjectionMode::Orthographic,
            ProjectionMode::Orthographic => ProjectionMode::Perspective,
        };
    }

    pub(crate) const fn projection(self) -> ProjectionMode {
        self.projection
    }

    pub(crate) fn reset(&mut self, radius: f64) {
        *self = Self::new(self.home_target, radius).with_projection(self.projection);
    }

    pub(crate) fn as_render_camera(self) -> Result<Camera, CameraError> {
        let horizontal_radius = self.radius * self.elevation.cos();
        let eye = [
            self.target[0] + horizontal_radius * self.azimuth.cos(),
            self.target[1] + horizontal_radius * self.azimuth.sin(),
            self.target[2] + self.radius * self.elevation.sin(),
        ];

        match self.projection {
            ProjectionMode::Perspective => Camera::perspective(
                eye,
                self.target,
                [0.0, 0.0, 1.0],
                PERSPECTIVE_FIELD_OF_VIEW,
                0.5,
                self.far_distance,
            ),
            ProjectionMode::Orthographic => Camera::orthographic(
                eye,
                self.target,
                [0.0, 0.0, 1.0],
                self.vertical_world_height(),
                0.5,
                self.far_distance,
            ),
        }
    }

    pub(crate) fn target_plane_world(
        self,
        pixel: [f64; 2],
        viewport: [u32; 2],
    ) -> Result<Option<[f64; 3]>, CameraError> {
        if viewport[0] == 0
            || viewport[1] == 0
            || pixel[0] < 0.0
            || pixel[1] < 0.0
            || pixel[0] > f64::from(viewport[0])
            || pixel[1] > f64::from(viewport[1])
        {
            return Ok(None);
        }
        let basis = self.as_render_camera()?.world_basis();
        let world_per_pixel = self.vertical_world_height() / f64::from(viewport[1]);
        let horizontal = pixel[0] - f64::from(viewport[0]) * 0.5;
        let vertical = f64::from(viewport[1]) * 0.5 - pixel[1];
        let mut world = self.target;
        for (axis, coordinate) in world.iter_mut().enumerate() {
            *coordinate += basis.right()[axis] * horizontal * world_per_pixel
                + basis.up()[axis] * vertical * world_per_pixel;
        }
        Ok(Some(world))
    }

    pub(crate) fn world_units_for_pixels(self, pixels: u32, viewport_height: u32) -> f64 {
        if viewport_height == 0 {
            return 0.0;
        }
        self.vertical_world_height() * f64::from(pixels) / f64::from(viewport_height)
    }

    pub(crate) fn north_orientation(self) -> Result<NorthOrientation, CameraError> {
        let basis = self.as_render_camera()?.world_basis();
        let right = basis.right()[1];
        let up = basis.up()[1];
        Ok(if up.abs() >= right.abs() {
            if up >= 0.0 {
                NorthOrientation::Up
            } else {
                NorthOrientation::Down
            }
        } else if right >= 0.0 {
            NorthOrientation::Right
        } else {
            NorthOrientation::Left
        })
    }

    fn vertical_world_height(self) -> f64 {
        2.0 * self.radius * (f64::from(PERSPECTIVE_FIELD_OF_VIEW) * 0.5).tan()
    }
}

#[cfg(test)]
mod tests {
    use point_view::{AvailableNodes, PlanningBudget, ViewPlanner};
    use render_protocol::{ViewGenerationKey, ViewId, Viewport};

    use crate::synthetic::{SCENE_RADIUS, SCENE_TARGET, SyntheticScene};

    use super::*;

    const TARGET: [f64; 3] = [6_378_137.125, 13_756_432.625, 120.0];

    #[test]
    fn orbit_camera_remains_valid_at_input_extremes() {
        let mut camera = OrbitCamera::new(TARGET, 700.0);

        camera.orbit(1_000_000.0, 1_000_000.0);
        camera.zoom(1_000_000.0);
        camera.orbit(-1_000_000.0, -1_000_000.0);
        camera.zoom(-1_000_000.0);

        assert!(camera.as_render_camera().is_ok());
    }

    #[test]
    fn reset_restores_the_initial_view() {
        let initial = OrbitCamera::new(TARGET, 700.0);
        let mut changed = initial;
        changed.orbit(30.0, -20.0);
        changed.zoom(2.0);

        changed.reset(700.0);

        assert_eq!(changed.as_render_camera(), initial.as_render_camera());
    }

    #[test]
    fn projection_toggle_preserves_target_plane_scale() {
        let mut camera = OrbitCamera::new(TARGET, 700.0);
        let expected_height = camera.vertical_world_height();

        camera.toggle_projection();
        let rendered = camera.as_render_camera().unwrap();

        assert_eq!(camera.projection(), ProjectionMode::Orthographic);
        assert!(matches!(
            rendered.projection(),
            render_protocol::CameraProjection::Orthographic { vertical_world_height }
                if vertical_world_height.to_bits() == expected_height.to_bits()
        ));
    }

    #[test]
    fn status_geometry_reports_target_plane_cursor_scale_and_north() {
        let camera = OrbitCamera::new(TARGET, 700.0);
        let center = camera
            .target_plane_world([640.0, 400.0], [1_280, 800])
            .unwrap()
            .unwrap();

        assert_eq!(center.map(f64::to_bits), TARGET.map(f64::to_bits));
        assert!(camera.world_units_for_pixels(100, 800).is_finite());
        assert!(camera.world_units_for_pixels(100, 800) > 0.0);
        assert!(matches!(
            camera.north_orientation().unwrap(),
            NorthOrientation::Up
                | NorthOrientation::Down
                | NorthOrientation::Left
                | NorthOrientation::Right
        ));
        assert_eq!(
            camera
                .target_plane_world([-1.0, 400.0], [1_280, 800])
                .unwrap(),
            None
        );
    }

    #[test]
    fn large_world_pan_is_finite_and_resettable() {
        let initial = OrbitCamera::new(TARGET, 700.0).with_projection(ProjectionMode::Orthographic);
        let mut camera = initial;

        camera.pan(120.0, -45.0, 800).unwrap();
        assert_ne!(
            camera.as_render_camera().unwrap(),
            initial.as_render_camera().unwrap()
        );
        camera.reset(700.0);
        assert_eq!(
            camera.as_render_camera().unwrap(),
            initial.as_render_camera().unwrap()
        );
    }

    #[test]
    fn maximum_radius_keeps_the_scene_root_requestable() {
        let view_generation = ViewGenerationKey::new(ViewId::new(1), 1);
        let scene = SyntheticScene::new(view_generation).unwrap();
        let planning_nodes = scene.planning_nodes();
        let mut orbit = OrbitCamera::new(SCENE_TARGET, SCENE_RADIUS);
        orbit.zoom(-1_000_000.0);

        for projection in [ProjectionMode::Perspective, ProjectionMode::Orthographic] {
            let plan = ViewPlanner::default()
                .plan(
                    &orbit
                        .with_projection(projection)
                        .as_render_camera()
                        .unwrap(),
                    Viewport::new(1_280, 800).unwrap(),
                    AvailableNodes::new(view_generation, &planning_nodes),
                    PlanningBudget::new(u64::MAX, u64::MAX, u64::MAX),
                )
                .unwrap();

            assert_eq!(plan.requests().len(), 1);
            assert_eq!(plan.requests()[0].node().get(), 1);
        }
    }
}
