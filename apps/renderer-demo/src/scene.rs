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

/// Private concrete choice between the original fixture and a prepared Source.
#[derive(Debug)]
pub(crate) enum Scene {
    Synthetic(SyntheticScene),
    Real(Box<RealCloudScene>),
}

impl Scene {
    pub(crate) fn synthetic(generation: ViewGenerationKey) -> SceneResult<Self> {
        Ok(Self::Synthetic(SyntheticScene::new(generation)?))
    }

    pub(crate) fn real(scene: RealCloudScene) -> Self {
        Self::Real(Box::new(scene))
    }

    pub(crate) fn planning_nodes(&self) -> PlanningNodes<'_> {
        match self {
            Self::Synthetic(scene) => PlanningNodes::Synthetic(scene.planning_nodes()),
            Self::Real(scene) => PlanningNodes::Real(scene.planning_nodes()),
        }
    }

    pub(crate) fn reconcile_requests(
        &mut self,
        demanded_nodes: &[NodeKey],
        requests: &[NodeRequest],
    ) {
        match self {
            Self::Synthetic(scene) => scene.reconcile_requests(demanded_nodes, requests),
            Self::Real(scene) => scene.reconcile_requests(demanded_nodes, requests),
        }
    }

    pub(crate) fn next_batch(&mut self) -> SceneResult<Option<PointBatch>> {
        match self {
            Self::Synthetic(scene) => Ok(scene.next_batch()?),
            Self::Real(scene) => scene.next_batch(),
        }
    }

    pub(crate) fn mark_resident(&mut self, key: BatchKey, version: BatchVersion) {
        match self {
            Self::Synthetic(scene) => scene.mark_resident(key, version),
            Self::Real(scene) => scene.mark_resident(key, version),
        }
    }

    pub(crate) fn mark_retired(&mut self, key: BatchKey, version: BatchVersion) {
        match self {
            Self::Synthetic(scene) => scene.mark_retired(key, version),
            Self::Real(scene) => scene.mark_retired(key, version),
        }
    }

    pub(crate) fn mark_rejected(&mut self, key: BatchKey, version: BatchVersion) {
        match self {
            Self::Synthetic(scene) => scene.mark_rejected(key, version),
            Self::Real(scene) => scene.mark_rejected(key, version),
        }
    }

    pub(crate) fn camera_target(&self) -> [f64; 3] {
        match self {
            Self::Synthetic(_) => SCENE_TARGET,
            Self::Real(scene) => scene.camera_target(),
        }
    }

    pub(crate) fn camera_radius(&self) -> f64 {
        match self {
            Self::Synthetic(_) => SCENE_RADIUS,
            Self::Real(scene) => scene.camera_radius(),
        }
    }

    pub(crate) fn highlight_ids(&self) -> Vec<PointId> {
        match self {
            Self::Synthetic(_) => SyntheticScene::highlight_ids(),
            Self::Real(_) => Vec::new(),
        }
    }

    pub(crate) fn metrics(&self) -> SceneMetrics {
        match self {
            Self::Synthetic(scene) => SceneMetrics {
                logical_points: LOGICAL_POINT_COUNT,
                resident_batches: scene.resident_batches(),
                queued_batches: scene.pending_batches(),
                ..SceneMetrics::default()
            },
            Self::Real(scene) => scene.metrics(),
        }
    }

    pub(crate) const fn label(&self) -> &'static str {
        match self {
            Self::Synthetic(_) => "synthetic",
            Self::Real(_) => "verified LAS/LAZ",
        }
    }
}
