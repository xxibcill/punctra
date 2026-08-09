use std::f64::consts::{FRAC_PI_4, FRAC_PI_6};

use render_wgpu::{Camera, CameraError};

const MIN_ELEVATION: f64 = 0.08;
const MAX_ELEVATION: f64 = 1.48;
const MIN_RADIUS: f64 = 20.0;
const MAX_RADIUS: f64 = 10_000.0;
const FAR_DISTANCE: f32 = 20_000.0;
const ORBIT_RADIANS_PER_PIXEL: f64 = 0.006;
const ZOOM_EXPONENT_PER_LINE: f64 = 0.12;

#[derive(Clone, Copy, Debug)]
pub(crate) struct OrbitCamera {
    target: [f64; 3],
    azimuth: f64,
    elevation: f64,
    radius: f64,
    min_radius: f64,
    max_radius: f64,
    far_distance: f32,
}

impl OrbitCamera {
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn new(target: [f64; 3], radius: f64) -> Self {
        let min_radius = MIN_RADIUS.min((radius * 0.01).max(0.01));
        let max_radius = MAX_RADIUS.max(radius * 4.0);
        let far_distance = FAR_DISTANCE.max((max_radius * 2.0) as f32);
        Self {
            target,
            azimuth: -FRAC_PI_4,
            elevation: FRAC_PI_6,
            radius,
            min_radius,
            max_radius,
            far_distance,
        }
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

    pub(crate) fn reset(&mut self, radius: f64) {
        *self = Self::new(self.target, radius);
    }

    pub(crate) fn as_render_camera(self) -> Result<Camera, CameraError> {
        let horizontal_radius = self.radius * self.elevation.cos();
        let eye = [
            self.target[0] + horizontal_radius * self.azimuth.cos(),
            self.target[1] + horizontal_radius * self.azimuth.sin(),
            self.target[2] + self.radius * self.elevation.sin(),
        ];

        Camera::perspective(
            eye,
            self.target,
            [0.0, 0.0, 1.0],
            std::f32::consts::FRAC_PI_4,
            0.5,
            self.far_distance,
        )
    }
}

#[cfg(test)]
mod tests {
    use point_view::{AvailableNodes, PlanningBudget, ViewPlanner};
    use render_protocol::{ViewGenerationKey, ViewId};

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
    fn maximum_radius_keeps_the_scene_root_requestable() {
        let view_generation = ViewGenerationKey::new(ViewId::new(1), 1);
        let scene = SyntheticScene::new(view_generation).unwrap();
        let planning_nodes = scene.planning_nodes();
        let mut orbit = OrbitCamera::new(SCENE_TARGET, SCENE_RADIUS);
        orbit.zoom(-1_000_000.0);

        let plan = ViewPlanner::default()
            .plan(
                &orbit.as_render_camera().unwrap(),
                [1_280, 800],
                AvailableNodes::new(view_generation, &planning_nodes),
                PlanningBudget::new(u64::MAX, u64::MAX, u64::MAX),
            )
            .unwrap();

        assert_eq!(plan.requests().len(), 1);
        assert_eq!(plan.requests()[0].node().get(), 1);
    }
}
