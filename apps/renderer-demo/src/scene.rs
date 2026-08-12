use std::error::Error;

use point_view::{AvailableNode, NodeKey, NodeRequest};
use render_protocol::{BatchKey, BatchVersion, PointBatch, PointId, ViewGenerationKey};

use crate::{
    real_cloud::RealCloudScene,
    synthetic::{LOGICAL_POINT_COUNT, SCENE_RADIUS, SCENE_TARGET, SyntheticScene},
};

pub(crate) type SceneResult<T> = Result<T, Box<dyn Error>>;

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
    pub(crate) resident_batches: u64,
    pub(crate) queued_batches: u64,
    pub(crate) staged_points: u64,
    pub(crate) staged_bytes: u64,
    pub(crate) peak_queued_batches: u64,
    pub(crate) peak_staged_points: u64,
    pub(crate) peak_staged_bytes: u64,
    pub(crate) cancelled_requests: u64,
}

trait SceneBackend: std::fmt::Debug {
    fn planning_nodes(&self) -> PlanningNodes<'_>;
    fn reconcile_requests(
        &mut self,
        demanded_nodes: &[NodeKey],
        requests: &[NodeRequest],
    ) -> SceneResult<()>;
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
    ) -> SceneResult<()> {
        SyntheticScene::reconcile_requests(self, demanded_nodes, requests);
        Ok(())
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
        SceneMetrics {
            logical_points: LOGICAL_POINT_COUNT,
            resident_batches: SyntheticScene::resident_batches(self),
            queued_batches: SyntheticScene::pending_batches(self),
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
    ) -> SceneResult<()> {
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
        Vec::new()
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
    ) -> SceneResult<()> {
        self.0.reconcile_requests(demanded_nodes, requests)
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
