use std::error::Error;

use point_view::{AvailableNode, NodeKey, NodeRequest};
use render_protocol::{BatchKey, BatchVersion, PointBatch, PointId, ViewGenerationKey};

use crate::{
    real_cloud::RealCloudScene,
    synthetic::{LOGICAL_POINT_COUNT, SCENE_RADIUS, SCENE_TARGET, SyntheticScene},
};

pub(crate) type SceneResult<T> = Result<T, Box<dyn Error>>;

const NEW_REQUESTS_PER_PUMP: usize = 1;

pub(crate) enum PlanningNodes<'scene> {
    Synthetic(Vec<AvailableNode>),
    Real(&'scene [AvailableNode]),
}

impl PlanningNodes<'_> {
    pub(crate) fn as_slice(&self) -> &[AvailableNode] {
        match self {
            Self::Synthetic(nodes) => nodes,
            Self::Real(nodes) => nodes,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SceneMetrics {
    pub(crate) logical_points: u64,
    pub(crate) hierarchy_nodes: u64,
    pub(crate) missing_nodes: u64,
    pub(crate) requested_nodes: u64,
    pub(crate) resident_batches: u64,
    pub(crate) resident_points: u64,
    pub(crate) sampled_resident_batches: u64,
    pub(crate) sampled_resident_points: u64,
    pub(crate) complete_resident_batches: u64,
    pub(crate) complete_resident_points: u64,
    pub(crate) authored_resident_batches: u64,
    pub(crate) authored_resident_points: u64,
    pub(crate) queued_batches: u64,
    pub(crate) staged_points: u64,
    pub(crate) staged_bytes: u64,
    pub(crate) peak_queued_batches: u64,
    pub(crate) peak_queued_host_bytes: u64,
    pub(crate) peak_staged_points: u64,
    pub(crate) peak_staged_bytes: u64,
    pub(crate) cancelled_requests: u64,
    pub(crate) retired_batches: u64,
    pub(crate) rejected_batches: u64,
}

trait SceneBackend: std::fmt::Debug {
    fn planning_nodes(&self) -> PlanningNodes<'_>;
    fn reconcile_requests(
        &mut self,
        demanded_nodes: &[NodeKey],
        requests: &[NodeRequest],
    ) -> SceneResult<u64>;
    fn next_batch(&mut self) -> SceneResult<Option<PointBatch>>;
    fn mark_resident(&mut self, key: BatchKey, version: BatchVersion);
    fn mark_retired(&mut self, key: BatchKey, version: BatchVersion);
    fn mark_rejected(&mut self, key: BatchKey, version: BatchVersion);
    fn camera_target(&self) -> [f64; 3];
    fn camera_radius(&self) -> f64;
    fn highlight_ids(&self) -> Vec<PointId>;
    fn metrics(&self) -> SceneMetrics;
    fn label(&self) -> &'static str;
}

impl SceneBackend for SyntheticScene {
    fn planning_nodes(&self) -> PlanningNodes<'_> {
        PlanningNodes::Synthetic(SyntheticScene::planning_nodes(self))
    }

    fn reconcile_requests(
        &mut self,
        demanded_nodes: &[NodeKey],
        requests: &[NodeRequest],
    ) -> SceneResult<u64> {
        Ok(SyntheticScene::reconcile_requests(
            self,
            demanded_nodes,
            requests,
        ))
    }

    fn next_batch(&mut self) -> SceneResult<Option<PointBatch>> {
        Ok(SyntheticScene::next_batch(self)?)
    }

    fn mark_resident(&mut self, key: BatchKey, version: BatchVersion) {
        SyntheticScene::mark_resident(self, key, version);
    }

    fn mark_retired(&mut self, key: BatchKey, version: BatchVersion) {
        SyntheticScene::mark_retired(self, key, version);
    }

    fn mark_rejected(&mut self, key: BatchKey, version: BatchVersion) {
        SyntheticScene::mark_rejected(self, key, version);
    }

    fn camera_target(&self) -> [f64; 3] {
        SCENE_TARGET
    }

    fn camera_radius(&self) -> f64 {
        SCENE_RADIUS
    }

    fn highlight_ids(&self) -> Vec<PointId> {
        SyntheticScene::highlight_ids()
    }

    fn metrics(&self) -> SceneMetrics {
        let status = SyntheticScene::status_facts(self);
        SceneMetrics {
            logical_points: LOGICAL_POINT_COUNT,
            hierarchy_nodes: status.hierarchy_nodes,
            missing_nodes: status.missing_nodes,
            requested_nodes: status.requested_nodes,
            resident_batches: status.resident_batches,
            resident_points: status.resident_points,
            authored_resident_batches: status.resident_batches,
            authored_resident_points: status.resident_points,
            queued_batches: SyntheticScene::pending_batches(self),
            cancelled_requests: SyntheticScene::cancelled_requests(self),
            retired_batches: SyntheticScene::retired_batches(self),
            rejected_batches: SyntheticScene::rejected_batches(self),
            ..SceneMetrics::default()
        }
    }

    fn label(&self) -> &'static str {
        "synthetic"
    }
}

impl SceneBackend for RealCloudScene {
    fn planning_nodes(&self) -> PlanningNodes<'_> {
        PlanningNodes::Real(RealCloudScene::planning_nodes(self))
    }

    fn reconcile_requests(
        &mut self,
        demanded_nodes: &[NodeKey],
        requests: &[NodeRequest],
    ) -> SceneResult<u64> {
        RealCloudScene::reconcile_requests(self, demanded_nodes, requests)
    }

    fn next_batch(&mut self) -> SceneResult<Option<PointBatch>> {
        RealCloudScene::next_batch(self)
    }

    fn mark_resident(&mut self, key: BatchKey, version: BatchVersion) {
        RealCloudScene::mark_resident(self, key, version);
    }

    fn mark_retired(&mut self, key: BatchKey, version: BatchVersion) {
        RealCloudScene::mark_retired(self, key, version);
    }

    fn mark_rejected(&mut self, key: BatchKey, version: BatchVersion) {
        RealCloudScene::mark_rejected(self, key, version);
    }

    fn camera_target(&self) -> [f64; 3] {
        RealCloudScene::camera_target(self)
    }

    fn camera_radius(&self) -> f64 {
        RealCloudScene::camera_radius(self)
    }

    fn highlight_ids(&self) -> Vec<PointId> {
        RealCloudScene::highlight_ids(self)
    }

    fn metrics(&self) -> SceneMetrics {
        RealCloudScene::metrics(self)
    }

    fn label(&self) -> &'static str {
        "verified LAS/LAZ"
    }
}

/// Private type-erased choice between the original fixture and a prepared Source.
#[derive(Debug)]
pub(crate) struct Scene(Box<dyn SceneBackend>);

impl Scene {
    pub(crate) fn synthetic(generation: ViewGenerationKey) -> SceneResult<Self> {
        Ok(Self(Box::new(SyntheticScene::new(generation)?)))
    }

    pub(crate) fn real(scene: RealCloudScene) -> Self {
        Self(Box::new(scene))
    }

    pub(crate) fn planning_nodes(&self) -> PlanningNodes<'_> {
        self.0.planning_nodes()
    }

    pub(crate) fn reconcile_requests(
        &mut self,
        demanded_nodes: &[NodeKey],
        requests: &[NodeRequest],
    ) -> SceneResult<u64> {
        let admitted_requests = &requests[..requests.len().min(NEW_REQUESTS_PER_PUMP)];
        self.0.reconcile_requests(demanded_nodes, admitted_requests)
    }

    pub(crate) fn next_batch(&mut self) -> SceneResult<Option<PointBatch>> {
        self.0.next_batch()
    }

    pub(crate) fn mark_resident(&mut self, key: BatchKey, version: BatchVersion) {
        self.0.mark_resident(key, version);
    }

    pub(crate) fn mark_retired(&mut self, key: BatchKey, version: BatchVersion) {
        self.0.mark_retired(key, version);
    }

    pub(crate) fn mark_rejected(&mut self, key: BatchKey, version: BatchVersion) {
        self.0.mark_rejected(key, version);
    }

    pub(crate) fn camera_target(&self) -> [f64; 3] {
        self.0.camera_target()
    }

    pub(crate) fn camera_radius(&self) -> f64 {
        self.0.camera_radius()
    }

    pub(crate) fn highlight_ids(&self) -> Vec<PointId> {
        self.0.highlight_ids()
    }

    pub(crate) fn metrics(&self) -> SceneMetrics {
        self.0.metrics()
    }

    pub(crate) fn label(&self) -> &'static str {
        self.0.label()
    }
}

#[cfg(test)]
mod tests {
    use point_view::{AvailableNodes, PlannerConfig, ViewPlanner};
    use render_protocol::{RenderLimits, RenderStateModel, RenderUpdate, Viewport};

    use crate::{
        PLANNING_BUDGET, VIEW_GENERATION,
        orbit_camera::OrbitCamera,
        synthetic::{RESIDENT_BATCH_BUDGET, RESIDENT_BYTE_BUDGET, RESIDENT_POINT_BUDGET},
    };

    use super::*;

    const SETTLEMENT_FRAME_CEILING: u64 = 1_024;
    const SETTLED_OBSERVATION_FRAMES: u64 = 300;

    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    struct FrameActivity {
        demanded: u64,
        requested: u64,
        issued: u64,
        uploaded: u64,
        retired: u64,
    }

    impl FrameActivity {
        const fn is_quiet(self) -> bool {
            self.demanded == 0
                && self.requested == 0
                && self.issued == 0
                && self.uploaded == 0
                && self.retired == 0
        }
    }

    #[test]
    fn stationary_default_view_converges_and_remains_quiet() {
        let mut scene = Scene::synthetic(VIEW_GENERATION).unwrap();
        let camera = OrbitCamera::new(scene.camera_target(), scene.camera_radius())
            .as_render_camera()
            .unwrap();
        let viewport = Viewport::new(2_560, 1_664).unwrap();
        let (mut planner, mut renderer) = reset_convergence_runtime();

        settle_and_observe(
            &mut scene,
            &mut planner,
            &mut renderer,
            &camera,
            viewport,
            SETTLED_OBSERVATION_FRAMES,
            "stationary default View",
        );
    }

    #[test]
    fn view_state_changes_reconverge_without_stale_work() {
        const TRANSITION_OBSERVATION_FRAMES: u64 = 16;

        let mut scene = Scene::synthetic(VIEW_GENERATION).unwrap();
        let mut orbit = OrbitCamera::new(scene.camera_target(), scene.camera_radius());
        let mut viewport = Viewport::new(1_280, 800).unwrap();
        let (mut planner, mut renderer) = reset_convergence_runtime();

        settle_and_observe(
            &mut scene,
            &mut planner,
            &mut renderer,
            &orbit.as_render_camera().unwrap(),
            viewport,
            TRANSITION_OBSERVATION_FRAMES,
            "initial View",
        );

        orbit.orbit(120.0, -45.0);
        settle_and_observe(
            &mut scene,
            &mut planner,
            &mut renderer,
            &orbit.as_render_camera().unwrap(),
            viewport,
            TRANSITION_OBSERVATION_FRAMES,
            "moved camera",
        );

        orbit.toggle_projection();
        settle_and_observe(
            &mut scene,
            &mut planner,
            &mut renderer,
            &orbit.as_render_camera().unwrap(),
            viewport,
            TRANSITION_OBSERVATION_FRAMES,
            "projection switch",
        );

        viewport = Viewport::new(1_920, 1_080).unwrap();
        settle_and_observe(
            &mut scene,
            &mut planner,
            &mut renderer,
            &orbit.as_render_camera().unwrap(),
            viewport,
            TRANSITION_OBSERVATION_FRAMES,
            "resized View",
        );

        orbit.zoom(4.0);
        settle_and_observe(
            &mut scene,
            &mut planner,
            &mut renderer,
            &orbit.as_render_camera().unwrap(),
            viewport,
            TRANSITION_OBSERVATION_FRAMES,
            "refined View",
        );

        orbit.zoom(-8.0);
        settle_and_observe(
            &mut scene,
            &mut planner,
            &mut renderer,
            &orbit.as_render_camera().unwrap(),
            viewport,
            TRANSITION_OBSERVATION_FRAMES,
            "coarsened View",
        );

        orbit.orbit(-80.0, 20.0);
        for _ in 0..16 {
            let activity = pump_frame(
                &mut scene,
                &mut planner,
                &mut renderer,
                &orbit.as_render_camera().unwrap(),
                viewport,
                true,
            );
            assert_eq!(activity.issued, 0);
            assert_eq!(activity.uploaded, 0);
            assert!(scene.metrics().resident_points > 0);
        }
        settle_and_observe(
            &mut scene,
            &mut planner,
            &mut renderer,
            &orbit.as_render_camera().unwrap(),
            viewport,
            TRANSITION_OBSERVATION_FRAMES,
            "resumed View",
        );

        orbit.reset(scene.camera_radius());
        settle_and_observe(
            &mut scene,
            &mut planner,
            &mut renderer,
            &orbit.as_render_camera().unwrap(),
            viewport,
            TRANSITION_OBSERVATION_FRAMES,
            "reset View",
        );
    }

    fn reset_convergence_runtime() -> (ViewPlanner, RenderStateModel) {
        let planner = ViewPlanner::new(PlannerConfig::new(2.0, 0.25).unwrap());
        let mut renderer = RenderStateModel::new(RenderLimits::new(
            RESIDENT_BYTE_BUDGET,
            RESIDENT_POINT_BUDGET,
            RESIDENT_BATCH_BUDGET,
        ));
        renderer
            .apply(&RenderUpdate::Reset {
                view_generation: VIEW_GENERATION,
            })
            .unwrap();
        (planner, renderer)
    }

    #[allow(clippy::too_many_arguments)]
    fn settle_and_observe(
        scene: &mut Scene,
        planner: &mut ViewPlanner,
        renderer: &mut RenderStateModel,
        camera: &render_protocol::Camera,
        viewport: Viewport,
        observation_frames: u64,
        context: &str,
    ) {
        let mut settlement_frame = None;
        let mut latest_activity = FrameActivity::default();
        for frame in 1..=SETTLEMENT_FRAME_CEILING {
            latest_activity = pump_frame(scene, planner, renderer, camera, viewport, false);
            if latest_activity.is_quiet() && scene.metrics().queued_batches == 0 {
                settlement_frame = Some(frame);
                break;
            }
        }

        let settlement_frame = settlement_frame.unwrap_or_else(|| {
            panic!(
                "{context} did not settle within {SETTLEMENT_FRAME_CEILING} frames; latest activity: {latest_activity:?}; metrics: {:?}",
                scene.metrics()
            )
        });
        let settled_metrics = scene.metrics();
        let settled_nodes = scene.planning_nodes().as_slice().to_vec();

        for observation_frame in 1..=observation_frames {
            let activity = pump_frame(scene, planner, renderer, camera, viewport, false);
            assert!(
                activity.is_quiet(),
                "{context} frame {observation_frame} after settlement frame {settlement_frame} produced work: {activity:?}"
            );
            assert_eq!(scene.metrics(), settled_metrics);
            assert_eq!(scene.planning_nodes().as_slice(), settled_nodes);
        }
    }

    fn pump_frame(
        scene: &mut Scene,
        planner: &mut ViewPlanner,
        renderer: &mut RenderStateModel,
        camera: &render_protocol::Camera,
        viewport: Viewport,
        loads_paused: bool,
    ) -> FrameActivity {
        let plan = {
            let nodes = scene.planning_nodes();
            planner
                .plan(
                    camera,
                    viewport,
                    AvailableNodes::new(VIEW_GENERATION, nodes.as_slice()),
                    PLANNING_BUDGET,
                )
                .unwrap()
        };
        let activity = FrameActivity {
            demanded: u64::try_from(plan.demanded_nodes().len()).unwrap(),
            requested: u64::try_from(plan.requests().len()).unwrap(),
            retired: u64::try_from(plan.retirements().len()).unwrap(),
            ..FrameActivity::default()
        };

        for retirement in plan.retirements().iter().copied() {
            renderer.apply(&retirement.render_update()).unwrap();
            scene.mark_retired(retirement.batch_key(), retirement.expected_version());
        }
        let requests = if loads_paused {
            &[][..]
        } else {
            plan.requests()
        };
        let issued = scene
            .reconcile_requests(plan.demanded_nodes(), requests)
            .unwrap();
        if loads_paused {
            return FrameActivity { issued, ..activity };
        }
        let Some(batch) = scene.next_batch().unwrap() else {
            return FrameActivity { issued, ..activity };
        };
        let key = batch.key();
        let version = batch.version();
        renderer.apply(&RenderUpdate::Upsert { batch }).unwrap();
        scene.mark_resident(key, version);
        FrameActivity {
            issued,
            uploaded: 1,
            ..activity
        }
    }
}
