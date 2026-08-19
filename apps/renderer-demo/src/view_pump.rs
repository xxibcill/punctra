//! Shared planning, streaming, and density-transition lifecycle for one view.

use std::error::Error;

use point_view::{AvailableNodes, PlanError, PlanningBudget, ViewPlan, ViewPlanner};
use render_protocol::{UpdateReport, ViewGenerationKey, Viewport};
use render_wgpu::{Camera, RendererError, WgpuRenderer};

use crate::{
    appearance::{ConditionalBatch, DensityTransitions, TransitionAction, apply_transition_action},
    diagnostic::{ViewFailure, ViewPhase, classify_renderer_failure},
    scene::Scene,
};

#[derive(Debug)]
pub(crate) enum ViewPumpError {
    Planning(PlanError),
    RequestReconciliation(Box<dyn Error>),
    NodeRead(Box<dyn Error>),
    Renderer(RendererError),
}

impl ViewPumpError {
    pub(crate) fn into_view_failure(self) -> ViewFailure {
        match self {
            Self::Planning(error) => ViewFailure::internal(ViewPhase::Planning, error),
            Self::RequestReconciliation(error) => {
                preserve_view_failure(error, ViewPhase::Planning, "request reconciliation")
            }
            Self::NodeRead(error) => preserve_view_failure(error, ViewPhase::NodeRead, "node read"),
            Self::Renderer(error) => classify_renderer_failure(ViewPhase::GpuUpload, error),
        }
    }
}

fn preserve_view_failure(
    error: Box<dyn Error>,
    phase: ViewPhase,
    context: &'static str,
) -> ViewFailure {
    match error.downcast::<ViewFailure>() {
        Ok(failure) => *failure,
        Err(error) => ViewFailure::internal(phase, format_args!("{context} failed: {error}")),
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ViewSpec<'view> {
    camera: &'view Camera,
    viewport: Viewport,
    view_generation: ViewGenerationKey,
    budget: PlanningBudget,
}

impl<'view> ViewSpec<'view> {
    pub(crate) const fn new(
        camera: &'view Camera,
        viewport: Viewport,
        view_generation: ViewGenerationKey,
        budget: PlanningBudget,
    ) -> Self {
        Self {
            camera,
            viewport,
            view_generation,
            budget,
        }
    }
}

#[derive(Debug)]
pub(crate) struct PlannedView {
    plan: ViewPlan,
    issued_requests: u64,
    transitions: TransitionActivity,
}

impl PlannedView {
    pub(crate) fn plan(&self) -> &ViewPlan {
        &self.plan
    }

    pub(crate) const fn issued_requests(&self) -> u64 {
        self.issued_requests
    }

    pub(crate) fn into_parts(self) -> (ViewPlan, u64, TransitionActivity) {
        (self.plan, self.issued_requests, self.transitions)
    }
}

#[derive(Debug)]
pub(crate) struct AcceptedBatch {
    upload: UpdateReport,
    transitions: TransitionActivity,
}

impl AcceptedBatch {
    pub(crate) const fn upload(&self) -> UpdateReport {
        self.upload
    }

    pub(crate) fn into_parts(self) -> (UpdateReport, TransitionActivity) {
        (self.upload, self.transitions)
    }
}

#[derive(Debug, Default)]
pub(crate) struct TransitionActivity {
    reports: Vec<UpdateReport>,
    presentations: u64,
    retired: u64,
}

impl TransitionActivity {
    pub(crate) fn reports(&self) -> &[UpdateReport] {
        &self.reports
    }

    pub(crate) const fn presentations(&self) -> u64 {
        self.presentations
    }

    pub(crate) const fn retired(&self) -> u64 {
        self.retired
    }

    pub(crate) fn add(&mut self, other: Self) {
        self.reports.extend(other.reports);
        self.presentations = self.presentations.saturating_add(other.presentations);
        self.retired = self.retired.saturating_add(other.retired);
    }
}

pub(crate) struct ViewLifecycle<'state> {
    scene: &'state mut Scene,
    renderer: &'state mut WgpuRenderer,
    density_transitions: &'state mut DensityTransitions,
}

impl<'state> ViewLifecycle<'state> {
    pub(crate) fn new(
        scene: &'state mut Scene,
        renderer: &'state mut WgpuRenderer,
        density_transitions: &'state mut DensityTransitions,
    ) -> Self {
        Self {
            scene,
            renderer,
            density_transitions,
        }
    }

    pub(crate) fn reconcile_view(
        &mut self,
        planner: &mut ViewPlanner,
        view: ViewSpec<'_>,
    ) -> Result<PlannedView, ViewPumpError> {
        let (hierarchy, plan) = {
            let planning_nodes = self.scene.planning_nodes();
            let hierarchy = planning_nodes.as_slice().to_vec();
            let plan = planner
                .plan(
                    view.camera,
                    view.viewport,
                    AvailableNodes::new(view.view_generation, planning_nodes.as_slice()),
                    view.budget,
                )
                .map_err(ViewPumpError::Planning)?;
            (hierarchy, plan)
        };

        let actions = self.density_transitions.reconcile(&hierarchy, &plan);
        let transitions = self.apply_transition_actions(actions)?;
        let requests = if self.density_transitions.blocks_new_residency() {
            &[]
        } else {
            plan.requests()
        };
        let issued_requests = self
            .scene
            .reconcile_requests(plan.demanded_nodes(), requests)
            .map_err(ViewPumpError::RequestReconciliation)?;
        Ok(PlannedView {
            plan,
            issued_requests,
            transitions,
        })
    }

    pub(crate) fn accept_next_batch(
        &mut self,
        view_generation: ViewGenerationKey,
    ) -> Result<Option<AcceptedBatch>, ViewPumpError> {
        if self.density_transitions.blocks_new_residency() {
            return Ok(None);
        }
        let Some(batch) = self.scene.next_batch().map_err(ViewPumpError::NodeRead)? else {
            return Ok(None);
        };

        let key = batch.key();
        let version = batch.version();
        let upload = match self
            .renderer
            .apply(&render_protocol::RenderUpdate::Upsert { batch })
        {
            Ok(report) => report,
            Err(error) => {
                self.scene.mark_rejected(key, version);
                return Err(ViewPumpError::Renderer(error));
            }
        };
        let presentation = self
            .density_transitions
            .uploaded_batch_presentation(ConditionalBatch {
                view_generation,
                key,
                expected_version: version,
            });
        let transitions = presentation.map_or_else(
            || Ok(TransitionActivity::default()),
            |action| self.apply_transition_actions(vec![action]),
        )?;
        self.scene.mark_resident(key, version);
        Ok(Some(AcceptedBatch {
            upload,
            transitions,
        }))
    }

    pub(crate) fn advance_presented_frame(&mut self) -> Result<TransitionActivity, ViewPumpError> {
        let actions = self.density_transitions.advance_presented_frame();
        self.apply_transition_actions(actions)
    }

    pub(crate) fn display_density_point_count(&self) -> u64 {
        self.density_transitions
            .display_density_point_count(self.scene)
    }

    pub(crate) fn renderer_mut(&mut self) -> &mut WgpuRenderer {
        self.renderer
    }

    fn apply_transition_actions(
        &mut self,
        actions: Vec<TransitionAction>,
    ) -> Result<TransitionActivity, ViewPumpError> {
        let mut activity = TransitionActivity::default();
        for action in actions {
            let retiring = action.retiring_batch().is_some();
            let report = apply_transition_action(self.renderer, self.scene, action)
                .map_err(ViewPumpError::Renderer)?;
            activity.reports.push(report);
            if retiring {
                activity.retired = activity.retired.saturating_add(1);
            } else {
                activity.presentations = activity.presentations.saturating_add(1);
            }
        }
        Ok(activity)
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;

    #[test]
    fn pump_errors_own_their_failure_phase() {
        let request = ViewPumpError::RequestReconciliation(Box::new(io::Error::other("request")))
            .into_view_failure();
        let node = ViewPumpError::NodeRead(Box::new(io::Error::other("read"))).into_view_failure();

        assert_eq!(request.phase(), ViewPhase::Planning);
        assert_eq!(node.phase(), ViewPhase::NodeRead);
    }
}
