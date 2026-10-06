//! Actual planner, materializer, and renderer lifecycle acceptance.

use point_view::{PlannerConfig, ViewPlanner};

use super::{ColorTarget, FORMAT, GpuContext, point_style, with_gpu};
use crate::{
    PLANNING_BUDGET,
    appearance::{DensityTransitions, projected_density_point_size, renderer_appearance_config},
    orbit_camera::OrbitCamera,
    scene::Scene,
    synthetic::{RESIDENT_BATCH_BUDGET, RESIDENT_BYTE_BUDGET, RESIDENT_POINT_BUDGET},
    view_pump::{ViewLifecycle, ViewSpec},
};
use render_protocol::{RenderLimits, RenderUpdate, ViewGenerationKey, ViewId, Viewport};
use render_wgpu::{Frame, WgpuRenderer};

#[test]
fn actual_lifecycle_settles_refinement_coarsening_interruption_and_generation_changes() {
    with_gpu(|gpu| {
        let mut subject = LifecycleHarness::new(gpu, 1);
        subject.settle("initial");
        subject.orbit.orbit(120.0, -45.0);
        subject.settle("motion then stop");
        subject.orbit.toggle_projection();
        subject.settle("projection");
        subject.viewport = Viewport::new(800, 480).unwrap();
        subject.settle("resize");
        subject.orbit.zoom(4.0);
        subject.settle("refinement");
        subject.orbit.zoom(-8.0);
        subject.settle("coarsening");
        subject.orbit.zoom(8.0);
        for _ in 0..1_024 {
            subject.present(false);
            if subject.transitions.is_active() {
                break;
            }
        }
        assert!(
            subject.transitions.is_active(),
            "the interruption must hit a real active group"
        );
        let paused_nodes = subject.scene.planning_nodes().as_slice().to_vec();
        let paused_metrics = subject.scene.metrics();
        let paused_density = subject
            .transitions
            .display_density_point_count(&subject.scene);
        for _ in 0..8 {
            assert!(subject.present(true));
        }
        assert_eq!(subject.scene.planning_nodes().as_slice(), paused_nodes);
        assert_eq!(subject.scene.metrics(), paused_metrics);
        assert_eq!(
            subject
                .transitions
                .display_density_point_count(&subject.scene),
            paused_density
        );
        subject.orbit.zoom(-8.0);
        subject.orbit.orbit(-80.0, 20.0);
        subject.present(false);
        subject.orbit.zoom(4.0);
        subject.present(false);
        subject.orbit.zoom(-4.0);
        subject.settle("rapid interruption then resume");
        subject.orbit.reset(subject.scene.camera_radius());
        subject.settle("reset");
        subject.reset_generation(2);
        subject.settle("generation replacement");
        gpu.wait();
    });
}

struct LifecycleHarness<'gpu> {
    gpu: &'gpu GpuContext,
    scene: Scene,
    renderer: WgpuRenderer,
    transitions: DensityTransitions,
    planner: ViewPlanner,
    orbit: OrbitCamera,
    viewport: Viewport,
    generation: ViewGenerationKey,
}

impl<'gpu> LifecycleHarness<'gpu> {
    fn new(gpu: &'gpu GpuContext, generation: u64) -> Self {
        let generation = ViewGenerationKey::new(ViewId::new(94), generation);
        let scene = Scene::synthetic(generation).unwrap();
        let orbit = OrbitCamera::new(scene.camera_target(), scene.camera_radius());
        let mut renderer = WgpuRenderer::new(
            &gpu.device,
            renderer_appearance_config(
                FORMAT,
                RenderLimits::new(
                    RESIDENT_BYTE_BUDGET,
                    RESIDENT_POINT_BUDGET,
                    RESIDENT_BATCH_BUDGET,
                ),
            ),
        )
        .unwrap();
        renderer
            .apply(&RenderUpdate::Reset {
                view_generation: generation,
            })
            .unwrap();
        Self {
            gpu,
            scene,
            renderer,
            transitions: DensityTransitions::default(),
            planner: ViewPlanner::new(PlannerConfig::new(2.0, 0.25).unwrap()),
            orbit,
            viewport: Viewport::new(640, 480).unwrap(),
            generation,
        }
    }

    fn reset_generation(&mut self, generation: u64) {
        self.generation = ViewGenerationKey::new(ViewId::new(94), generation);
        self.renderer
            .apply(&RenderUpdate::Reset {
                view_generation: self.generation,
            })
            .unwrap();
        self.scene = Scene::synthetic(self.generation).unwrap();
        self.planner = ViewPlanner::new(PlannerConfig::new(2.0, 0.25).unwrap());
        self.orbit = OrbitCamera::new(self.scene.camera_target(), self.scene.camera_radius());
    }

    fn settle(&mut self, context: &str) {
        let settled = (1..=1_024)
            .find(|_| {
                self.present(false)
                    && self.scene.metrics().queued_batches == 0
                    && !self.transitions.is_active()
            })
            .unwrap_or_else(|| {
                panic!(
                    "{context} exceeded 1,024 frames: {:?}",
                    self.scene.metrics()
                )
            });
        let metrics = self.scene.metrics();
        let nodes = self.scene.planning_nodes().as_slice().to_vec();
        for _ in 0..300 {
            assert!(
                self.present(false),
                "{context} produced work after settling"
            );
            assert_eq!(self.scene.metrics(), metrics);
            assert_eq!(self.scene.planning_nodes().as_slice(), nodes);
        }
        eprintln!(
            "v0.23 actual lifecycle {context}: settled={settled}, resident_points={}, resident_batches={}, retired={}, cancelled={}",
            metrics.resident_points,
            metrics.resident_batches,
            metrics.retired_batches,
            metrics.cancelled_requests
        );
    }

    fn present(&mut self, paused: bool) -> bool {
        let camera = self.orbit.as_render_camera().unwrap();
        let mut lifecycle =
            ViewLifecycle::new(&mut self.scene, &mut self.renderer, &mut self.transitions);
        let mut quiet = true;
        if !paused {
            let (_, issued, actions) = lifecycle
                .reconcile_view(
                    &mut self.planner,
                    ViewSpec::new(&camera, self.viewport, self.generation, PLANNING_BUDGET),
                )
                .unwrap()
                .into_parts();
            quiet = issued == 0 && actions.reports().is_empty();
            quiet &= lifecycle
                .accept_next_batch(self.generation)
                .unwrap()
                .is_none();
        }
        let style = point_style(7.0)
            .with_display_size_pixels(projected_density_point_size(
                self.viewport,
                lifecycle.display_density_point_count(),
            ))
            .unwrap();
        let frame = Frame::new(self.generation, camera, self.viewport)
            .unwrap()
            .with_style(style);
        let target = ColorTarget::new(
            &self.gpu.device,
            self.viewport.dimensions(),
            FORMAT,
            "actual lifecycle acceptance",
        );
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("actual lifecycle acceptance"),
            });
        let recorded = lifecycle
            .renderer_mut()
            .render(&mut encoder, &target.view, &frame)
            .unwrap();
        let report = recorded.report();
        assert!(report.resident_bytes() <= RESIDENT_BYTE_BUDGET);
        assert!(report.drawn_points() <= RESIDENT_POINT_BUDGET);
        assert!(report.draw_calls() <= RESIDENT_BATCH_BUDGET);
        assert!(report.transient_texture_bytes() <= 64 * 1_024 * 1_024);
        self.gpu.queue.submit([encoder.finish()]);
        if !paused {
            quiet &= lifecycle
                .advance_presented_frame()
                .unwrap()
                .reports()
                .is_empty();
            if quiet {
                assert_eq!(report.raster_transition_batches(), 0);
            }
        }
        self.gpu.wait();
        quiet
    }
}
