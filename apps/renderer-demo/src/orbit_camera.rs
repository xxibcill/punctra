use std::f64::consts::{FRAC_PI_4, FRAC_PI_6};

use render_wgpu::{Camera, CameraError};

const MIN_ELEVATION: f64 = 0.08;
const MAX_ELEVATION: f64 = 1.48;
const MIN_RADIUS: f64 = 20.0;
const MAX_RADIUS: f64 = 10_000.0;
const ORBIT_RADIANS_PER_PIXEL: f64 = 0.006;
const ZOOM_EXPONENT_PER_LINE: f64 = 0.12;

#[derive(Clone, Copy, Debug)]
pub(crate) struct OrbitCamera {
    target: [f64; 3],
    azimuth: f64,
    elevation: f64,
    radius: f64,
}

impl OrbitCamera {
    pub(crate) const fn new(target: [f64; 3], radius: f64) -> Self {
        Self {
            target,
            azimuth: -FRAC_PI_4,
            elevation: FRAC_PI_6,
            radius,
        }
    }

    pub(crate) fn orbit(&mut self, horizontal_pixels: f64, vertical_pixels: f64) {
        self.azimuth -= horizontal_pixels * ORBIT_RADIANS_PER_PIXEL;
        self.elevation = (self.elevation + vertical_pixels * ORBIT_RADIANS_PER_PIXEL)
            .clamp(MIN_ELEVATION, MAX_ELEVATION);
    }

    pub(crate) fn zoom(&mut self, lines: f64) {
        let zoom_factor = (-lines * ZOOM_EXPONENT_PER_LINE).exp();
        self.radius = (self.radius * zoom_factor).clamp(MIN_RADIUS, MAX_RADIUS);
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
            5_000.0,
        )
    }
}

#[cfg(test)]
mod tests {
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
}
