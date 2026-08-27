use point_view::{
    AvailableNode, AvailableNodes, AxisAlignedBox, NodeKey, NodeStatus, PlanningBudget, ViewPlanner,
};
use render_protocol::{
    BatchKey, BatchVersion, Camera, CameraError, ESTIMATED_GPU_BYTES_PER_POINT, PointBatch,
    PointId, ProtocolError, RenderLimits, RenderPoint, RenderUpdate, SourceId, ViewGenerationKey,
    ViewId, Viewport,
};
use render_wgpu::{Frame, FrameError, PointStyle};
use serde::Serialize;
use thiserror::Error;

use crate::diagnostics::serialize_required_source_identity;

pub(crate) const VIEW_GENERATION: ViewGenerationKey = ViewGenerationKey::new(ViewId::new(15), 1);
pub(crate) const BATCH_KEY: BatchKey = BatchKey::new(1);
pub(crate) const BATCH_VERSION: BatchVersion = BatchVersion::new(1);
pub(crate) const SCENE_SIDE: u64 = 33;
pub(crate) const SCENE_POINT_COUNT: u64 = SCENE_SIDE * SCENE_SIDE;
pub(crate) const CENTRE_POINT_ORDINAL: u64 = SCENE_POINT_COUNT / 2;
pub(crate) const MAX_RESIDENT_POINTS: u64 = crate::streaming::MAX_STREAM_POINTS;
pub(crate) const MAX_RESIDENT_BYTES: u64 = MAX_RESIDENT_POINTS * ESTIMATED_GPU_BYTES_PER_POINT;
pub(crate) const MAX_RESIDENT_BATCHES: u64 = crate::streaming::MAX_TRANSFER_BATCHES;
pub(crate) const MAX_HIGHLIGHT_POINTS: u64 = 32;

const SOURCE_ID: SourceId = SourceId::new([0x15; 32]);
const NODE_KEY: NodeKey = match NodeKey::new(1) {
    Ok(key) => key,
    Err(_) => panic!("one is a valid Node key"),
};
const WORLD_ORIGIN: [f64; 3] = [500_000.0, 4_600_000.0, 100.0];

pub(crate) struct PreparedScene {
    batch: PointBatch,
    camera: Camera,
    facts: SceneFacts,
    planner: ViewPlanner,
}

impl PreparedScene {
    pub(crate) fn new() -> Result<Self, SceneError> {
        let points = generated_points()?;
        let batch = PointBatch::new(
            VIEW_GENERATION,
            BATCH_KEY,
            BATCH_VERSION,
            WORLD_ORIGIN,
            points,
        )?;
        let camera = scene_camera()?;
        let planner = plan_missing_root(&batch, &camera)?;
        let facts = SceneFacts {
            source_identity: SOURCE_ID,
            point_count: batch.point_count(),
            estimated_gpu_bytes: batch.estimated_gpu_bytes(),
            world_origin: WORLD_ORIGIN,
            initial_requests: 1,
            retained_nodes: 0,
            view_id: VIEW_GENERATION.view().get(),
            generation: VIEW_GENERATION.generation(),
            batch_key: BATCH_KEY.get(),
            batch_version: BATCH_VERSION.get(),
            centre_point_ordinal: CENTRE_POINT_ORDINAL,
            progressive_coverage: true,
            cpu_authoritative: false,
        };
        Ok(Self {
            batch,
            camera,
            facts,
            planner,
        })
    }

    pub(crate) fn settle_after_publication(&mut self) -> Result<(), SceneError> {
        if self.facts.retained_nodes != 0 {
            return Err(SceneError::PlanningInvariant);
        }
        let resident = available_node(
            &self.batch,
            scene_bounds()?,
            NodeStatus::Resident {
                version: BATCH_VERSION,
            },
        )?;
        let settled = self.planner.plan(
            &self.camera,
            planning_viewport()?,
            AvailableNodes::new(VIEW_GENERATION, &[resident]),
            planning_budget(),
        )?;
        validate_settled_plan_contract(&settled)?;
        self.facts.retained_nodes = 1;
        Ok(())
    }

    pub(crate) const fn reset_update() -> RenderUpdate {
        RenderUpdate::Reset {
            view_generation: VIEW_GENERATION,
        }
    }

    pub(crate) fn batch_update(&self) -> RenderUpdate {
        RenderUpdate::Upsert {
            batch: self.batch.clone(),
        }
    }

    pub(crate) fn frame(
        viewport: Viewport,
        view_generation: ViewGenerationKey,
        camera: Camera,
    ) -> Result<Frame, FrameError> {
        let style = PointStyle::new(7.0, [0.78, 0.66, 0.2], [0.075, 0.078, 0.075, 1.0])?;
        Ok(Frame::new(view_generation, camera, viewport)?.with_style(style))
    }

    pub(crate) const fn camera(&self) -> Camera {
        self.camera
    }

    pub(crate) const fn facts(&self) -> SceneFacts {
        self.facts
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(crate) struct SceneFacts {
    #[serde(serialize_with = "serialize_required_source_identity")]
    pub(crate) source_identity: SourceId,
    pub(crate) point_count: u64,
    pub(crate) estimated_gpu_bytes: u64,
    pub(crate) world_origin: [f64; 3],
    pub(crate) initial_requests: u64,
    pub(crate) retained_nodes: u64,
    pub(crate) view_id: u64,
    pub(crate) generation: u64,
    pub(crate) batch_key: u64,
    pub(crate) batch_version: u64,
    pub(crate) centre_point_ordinal: u64,
    pub(crate) progressive_coverage: bool,
    pub(crate) cpu_authoritative: bool,
}

#[derive(Debug, Error)]
pub(crate) enum SceneError {
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error(transparent)]
    Camera(#[from] CameraError),
    #[error(transparent)]
    Planning(#[from] point_view::PlanError),
    #[error("deterministic scene planning did not preserve its fixed request/retention contract")]
    PlanningInvariant,
}

pub(crate) const fn render_limits() -> RenderLimits {
    RenderLimits::new(
        MAX_RESIDENT_BYTES,
        MAX_RESIDENT_POINTS,
        MAX_RESIDENT_BATCHES,
    )
    .with_max_highlight_points(MAX_HIGHLIGHT_POINTS)
}

pub(crate) const fn centre_point_id() -> PointId {
    PointId::new(SOURCE_ID, CENTRE_POINT_ORDINAL)
}

fn generated_points() -> Result<Vec<RenderPoint>, ProtocolError> {
    let capacity = usize::try_from(SCENE_POINT_COUNT).map_err(|_| ProtocolError::SizeOverflow)?;
    let mut points = Vec::with_capacity(capacity);
    for row in 0..SCENE_SIDE {
        for column in 0..SCENE_SIDE {
            let ordinal = row * SCENE_SIDE + column;
            points.push(generated_point(row, column, ordinal)?);
        }
    }
    Ok(points)
}

fn generated_point(row: u64, column: u64, ordinal: u64) -> Result<RenderPoint, ProtocolError> {
    #[allow(clippy::cast_precision_loss)]
    let x = (column as f32 - 16.0) * 0.75;
    #[allow(clippy::cast_precision_loss)]
    let y = (row as f32 - 16.0) * 0.75;
    let z = (x.mul_add(x, -(y * y))) * 0.0125;
    let shade = u8::try_from((row + column) % 9).unwrap_or(0) * 7;
    let color = if ordinal == CENTRE_POINT_ORDINAL {
        [224, 184, 40, 255]
    } else {
        [105 + shade, 113 + shade, 108 + shade, 255]
    };
    RenderPoint::new([x, y, z], color, PointId::new(SOURCE_ID, ordinal))
}

fn scene_camera() -> Result<Camera, CameraError> {
    Camera::perspective(
        [
            WORLD_ORIGIN[0],
            WORLD_ORIGIN[1] - 31.0,
            WORLD_ORIGIN[2] + 22.0,
        ],
        WORLD_ORIGIN,
        [0.0, 0.0, 1.0],
        std::f32::consts::FRAC_PI_3,
        0.1,
        250.0,
    )
}

fn plan_missing_root(batch: &PointBatch, camera: &Camera) -> Result<ViewPlanner, SceneError> {
    let missing = available_node(batch, scene_bounds()?, NodeStatus::Missing)?;
    let mut planner = ViewPlanner::default();
    let initial = planner.plan(
        camera,
        planning_viewport()?,
        AvailableNodes::new(VIEW_GENERATION, &[missing]),
        planning_budget(),
    )?;
    validate_initial_plan_contract(&initial)?;
    Ok(planner)
}

fn scene_bounds() -> Result<AxisAlignedBox, point_view::PlanError> {
    AxisAlignedBox::new(
        [
            WORLD_ORIGIN[0] - 12.0,
            WORLD_ORIGIN[1] - 12.0,
            WORLD_ORIGIN[2] - 4.0,
        ],
        [
            WORLD_ORIGIN[0] + 12.0,
            WORLD_ORIGIN[1] + 12.0,
            WORLD_ORIGIN[2] + 4.0,
        ],
    )
}

fn planning_viewport() -> Result<Viewport, SceneError> {
    Viewport::new(960, 600).map_err(|_| SceneError::PlanningInvariant)
}

const fn planning_budget() -> PlanningBudget {
    PlanningBudget::new(
        MAX_RESIDENT_POINTS,
        MAX_RESIDENT_BYTES,
        MAX_RESIDENT_BATCHES,
    )
}

fn available_node(
    batch: &PointBatch,
    bounds: AxisAlignedBox,
    status: NodeStatus,
) -> Result<AvailableNode, point_view::PlanError> {
    AvailableNode::new(
        NODE_KEY,
        None,
        bounds,
        4.0,
        batch.point_count(),
        batch.estimated_gpu_bytes(),
        BATCH_KEY,
        status,
    )
}

fn validate_initial_plan_contract(initial: &point_view::ViewPlan) -> Result<(), SceneError> {
    let initial_is_exact = initial.requests().len() == 1
        && initial.requests()[0].node() == NODE_KEY
        && initial.retained_nodes().is_empty()
        && initial.retirements().is_empty();
    if initial_is_exact {
        Ok(())
    } else {
        Err(SceneError::PlanningInvariant)
    }
}

fn validate_settled_plan_contract(settled: &point_view::ViewPlan) -> Result<(), SceneError> {
    let settled_is_exact = settled.requests().is_empty()
        && settled.retained_nodes().len() == 1
        && settled.retained_nodes()[0].node_key() == NODE_KEY
        && settled.retirements().is_empty();
    if settled_is_exact {
        Ok(())
    } else {
        Err(SceneError::PlanningInvariant)
    }
}

#[cfg(test)]
mod tests {
    use render_protocol::RenderStateModel;

    use super::*;

    #[test]
    fn generated_scene_has_fixed_identity_planning_and_resource_facts() {
        let mut scene = PreparedScene::new().unwrap();
        let initial_facts = scene.facts();
        let frame = PreparedScene::frame(
            Viewport::new(960, 600).unwrap(),
            VIEW_GENERATION,
            scene.camera(),
        )
        .unwrap();

        assert_eq!(initial_facts.point_count, 1_089);
        assert_eq!(initial_facts.estimated_gpu_bytes, 26_136);
        assert_eq!(initial_facts.initial_requests, 1);
        assert_eq!(initial_facts.retained_nodes, 0);
        assert_eq!(initial_facts.centre_point_ordinal, 544);
        assert!(!initial_facts.cpu_authoritative);
        assert_eq!(frame.view_generation(), VIEW_GENERATION);

        scene.settle_after_publication().unwrap();
        assert_eq!(scene.facts().retained_nodes, 1);
        assert!(matches!(
            scene.settle_after_publication(),
            Err(SceneError::PlanningInvariant)
        ));
    }

    #[test]
    fn protocol_preserves_generation_version_and_atomic_limit_rules() {
        let scene = PreparedScene::new().unwrap();
        let mut state = RenderStateModel::new(render_limits());
        state.apply(&PreparedScene::reset_update()).unwrap();
        state.apply(&scene.batch_update()).unwrap();
        let before = state.snapshot();

        assert!(matches!(
            state.apply(&scene.batch_update()),
            Err(ProtocolError::BatchVersionNotIncreasing { .. })
        ));
        assert_eq!(state.snapshot(), before);
        assert_eq!(before.active_view_generation(), Some(VIEW_GENERATION));
        assert_eq!(before.resident().point_count(), SCENE_POINT_COUNT);
    }

    #[test]
    fn centre_identity_is_source_aware_and_stable() {
        let scene = PreparedScene::new().unwrap();
        let RenderUpdate::Upsert { batch } = scene.batch_update() else {
            panic!("scene publishes one batch");
        };
        let centre = &batch.points()[usize::try_from(CENTRE_POINT_ORDINAL).unwrap()];

        assert_eq!(centre.point_id(), centre_point_id());
        assert!(
            centre
                .relative_position()
                .into_iter()
                .all(|value| value.abs() <= f32::EPSILON)
        );
    }
}
