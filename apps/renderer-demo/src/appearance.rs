use std::collections::{BTreeMap, BTreeSet};

use point_view::{AvailableNode, NodeKey, NodeStatus, RetainedNode, ViewPlan};
use render_protocol::{
    BatchKey, BatchVersion, RasterTransition, RasterTransitionSide, RenderLimits, RenderUpdate,
    UpdateReport, ViewGenerationKey, Viewport,
};
use render_wgpu::{EyeDomeLighting, PointFootprint, RendererConfig, RendererError, WgpuRenderer};

use crate::scene::Scene;

pub(crate) const CROSS_FADE_PRESENTED_FRAMES: u8 = 8;
pub(crate) const MIN_POINT_SIZE_PIXELS: f32 = 1.0;
pub(crate) const MAX_POINT_SIZE_PIXELS: f32 = 4.0;
pub(crate) const REFERENCE_POINT_SIZE_PIXELS: f32 = 2.4;

pub(crate) fn renderer_appearance_config(
    color_format: wgpu::TextureFormat,
    limits: RenderLimits,
) -> RendererConfig {
    let depth_cue = EyeDomeLighting::new(1.25, 1)
        .expect("the fixed renderer-demo depth cue must stay within render-wgpu bounds");
    RendererConfig::new(color_format, limits)
        .with_eye_dome_lighting(depth_cue)
        .with_point_footprint(PointFootprint::Antialiased)
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ConditionalBatch {
    pub(crate) view_generation: ViewGenerationKey,
    pub(crate) key: BatchKey,
    pub(crate) expected_version: BatchVersion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransitionAction {
    Present {
        batch: ConditionalBatch,
        transition: RasterTransition,
    },
    Retire(ConditionalBatch),
}

impl TransitionAction {
    pub(crate) fn render_update(self) -> RenderUpdate {
        match self {
            Self::Present { batch, transition } => RenderUpdate::SetBatchRasterTransition {
                view_generation: batch.view_generation,
                key: batch.key,
                expected_version: batch.expected_version,
                transition,
            },
            Self::Retire(batch) => RenderUpdate::Remove {
                view_generation: batch.view_generation,
                key: batch.key,
                expected_version: batch.expected_version,
            },
        }
    }

    pub(crate) const fn retiring_batch(self) -> Option<ConditionalBatch> {
        match self {
            Self::Present { .. } => None,
            Self::Retire(batch) => Some(batch),
        }
    }
}

pub(crate) fn apply_transition_action(
    renderer: &mut WgpuRenderer,
    scene: &mut Scene,
    action: TransitionAction,
) -> Result<UpdateReport, RendererError> {
    let report = renderer.apply(&action.render_update())?;
    if let Some(batch) = action.retiring_batch() {
        scene.mark_retired(batch.key, batch.expected_version);
    }
    Ok(report)
}

#[derive(Clone, Debug)]
struct ActiveTransition {
    outgoing: Vec<ConditionalBatch>,
    incoming: Vec<ConditionalBatch>,
    seed: u32,
    presented_frames: u8,
}

impl ActiveTransition {
    fn controls(&self, key: BatchKey) -> bool {
        self.batches().any(|batch| batch.key == key)
    }

    fn batches(&self) -> impl Iterator<Item = ConditionalBatch> + '_ {
        self.outgoing.iter().chain(&self.incoming).copied()
    }

    fn coverage_eighths(&self, key: BatchKey) -> Option<u8> {
        if self.outgoing.iter().any(|batch| batch.key == key) {
            return Some(CROSS_FADE_PRESENTED_FRAMES - self.presented_frames);
        }
        self.incoming
            .iter()
            .any(|batch| batch.key == key)
            .then_some(self.presented_frames)
    }

    fn presentations(&self) -> impl Iterator<Item = TransitionAction> + '_ {
        [
            (self.outgoing.as_slice(), RasterTransitionSide::Outgoing),
            (self.incoming.as_slice(), RasterTransitionSide::Incoming),
        ]
        .into_iter()
        .flat_map(move |(batches, side)| {
            batches
                .iter()
                .copied()
                .map(move |batch| TransitionAction::Present {
                    batch,
                    transition: RasterTransition::new(self.seed, side, self.presented_frames)
                        .expect("host progress stays within eight presented frames"),
                })
        })
    }

    fn matches_plan(&self, hierarchy: &HierarchyIndex<'_>, plan: &ViewPlan) -> bool {
        self.outgoing.iter().all(|batch| {
            plan.retirements().iter().any(|retirement| {
                retirement.batch_key() == batch.key
                    && retirement.expected_version() == batch.expected_version
                    && retirement.view_generation() == batch.view_generation
            }) && resident_batch(hierarchy, plan, batch.key) == Some(*batch)
        }) && self.incoming.iter().all(|batch| {
            plan.retained_nodes()
                .iter()
                .any(|node| conditional_retained(*node) == *batch)
                && resident_batch(hierarchy, plan, batch.key) == Some(*batch)
        })
    }
}

#[derive(Default)]
pub(crate) struct DensityTransitions {
    view_generation: Option<ViewGenerationKey>,
    active: BTreeMap<BatchKey, ActiveTransition>,
    pending_replacements: BTreeSet<BatchKey>,
}

impl DensityTransitions {
    pub(crate) fn reconcile(
        &mut self,
        hierarchy: &[AvailableNode],
        plan: &ViewPlan,
    ) -> Vec<TransitionAction> {
        if self.view_generation != Some(plan.view_generation()) {
            self.active.clear();
            self.pending_replacements.clear();
            self.view_generation = Some(plan.view_generation());
        }
        let hierarchy = HierarchyIndex::new(hierarchy);
        let retained = plan
            .retained_nodes()
            .iter()
            .map(|node| node.node_key())
            .collect();
        let mut next_pending =
            pending_replacement_batches(&hierarchy, &retained, plan.demanded_nodes());
        let mut actions = Vec::new();
        let mut retired = BTreeSet::new();
        self.active.retain(|_, transition| {
            if transition.matches_plan(&hierarchy, plan) {
                return true;
            }
            for batch in transition.batches() {
                if resident_batch(&hierarchy, plan, batch.key) != Some(batch) {
                    continue;
                }
                if plan
                    .retirements()
                    .iter()
                    .any(|node| node.batch_key() == batch.key)
                {
                    actions.push(TransitionAction::Retire(batch));
                    retired.insert(batch.key);
                } else {
                    actions.push(TransitionAction::Present {
                        batch,
                        transition: if next_pending.contains(&batch.key) {
                            RasterTransition::HIDDEN
                        } else {
                            RasterTransition::FULL
                        },
                    });
                }
            }
            false
        });
        self.start_groups(&hierarchy, plan, &next_pending, &mut retired, &mut actions);
        next_pending.retain(|key| !self.controls(*key));
        for key in self
            .pending_replacements
            .symmetric_difference(&next_pending)
        {
            if self.controls(*key) || retired.contains(key) {
                continue;
            }
            if let Some(batch) = resident_batch(&hierarchy, plan, *key) {
                actions.push(TransitionAction::Present {
                    batch,
                    transition: if next_pending.contains(key) {
                        RasterTransition::HIDDEN
                    } else {
                        RasterTransition::FULL
                    },
                });
            }
        }
        self.pending_replacements = next_pending;
        actions
    }

    fn start_groups(
        &mut self,
        hierarchy: &HierarchyIndex<'_>,
        plan: &ViewPlan,
        next_pending: &BTreeSet<BatchKey>,
        retired: &mut BTreeSet<BatchKey>,
        actions: &mut Vec<TransitionAction>,
    ) {
        let mut outgoing = plan
            .retirements()
            .iter()
            .map(|node| ConditionalBatch {
                view_generation: node.view_generation(),
                key: node.batch_key(),
                expected_version: node.expected_version(),
            })
            .filter(|batch| !self.controls(batch.key) && !retired.contains(&batch.key))
            .collect::<Vec<_>>();
        let mut incoming = plan
            .retained_nodes()
            .iter()
            .copied()
            .map(conditional_retained)
            .filter(|batch| !self.controls(batch.key) && !next_pending.contains(&batch.key))
            .collect::<Vec<_>>();
        while let Some(first) = outgoing.pop() {
            if self.pending_replacements.contains(&first.key) {
                actions.push(TransitionAction::Retire(first));
                retired.insert(first.key);
                continue;
            }
            let (group_outgoing, group_incoming) = connected_cut(
                hierarchy,
                first,
                &mut outgoing,
                &mut incoming,
                &self.pending_replacements,
            );
            if group_incoming.is_empty() {
                actions.push(TransitionAction::Retire(first));
                retired.insert(first.key);
                continue;
            }
            let anchor = group_outgoing
                .iter()
                .chain(&group_incoming)
                .map(|batch| batch.key)
                .min()
                .expect("a group contains its first outgoing batch");
            let transition = ActiveTransition {
                outgoing: group_outgoing,
                incoming: group_incoming,
                seed: transition_seed(anchor),
                presented_frames: 0,
            };
            actions.extend(transition.presentations());
            self.active.insert(anchor, transition);
        }
    }

    fn controls(&self, key: BatchKey) -> bool {
        self.active
            .values()
            .any(|transition| transition.controls(key))
    }

    pub(crate) fn uploaded_batch_presentation(
        &self,
        batch: ConditionalBatch,
    ) -> Option<TransitionAction> {
        self.pending_replacements
            .contains(&batch.key)
            .then_some(TransitionAction::Present {
                batch,
                transition: RasterTransition::HIDDEN,
            })
    }

    pub(crate) fn advance_presented_frame(&mut self) -> Vec<TransitionAction> {
        let mut actions = Vec::new();
        self.active.retain(|_, transition| {
            transition.presented_frames += 1;
            actions.extend(transition.presentations());
            if transition.presented_frames < CROSS_FADE_PRESENTED_FRAMES {
                return true;
            }
            actions.extend(
                transition
                    .outgoing
                    .iter()
                    .copied()
                    .map(TransitionAction::Retire),
            );
            actions.extend(transition.incoming.iter().copied().map(|batch| {
                TransitionAction::Present {
                    batch,
                    transition: RasterTransition::FULL,
                }
            }));
            false
        });
        actions
    }

    pub(crate) fn is_active(&self) -> bool {
        !self.active.is_empty()
    }

    pub(crate) fn blocks_new_residency(&self) -> bool {
        self.is_active()
    }

    pub(crate) fn display_density_point_count(&self, scene: &Scene) -> u64 {
        if self.active.is_empty() && self.pending_replacements.is_empty() {
            return scene.metrics().resident_points;
        }
        self.display_density_point_count_in(scene.planning_nodes().as_slice())
    }

    fn display_density_point_count_in(&self, hierarchy: &[AvailableNode]) -> u64 {
        let weighted = hierarchy
            .iter()
            .filter(|node| matches!(node.status(), NodeStatus::Resident { .. }))
            .map(|node| {
                u128::from(node.point_count()) * u128::from(self.coverage_eighths(node.batch_key()))
            })
            .fold(0_u128, u128::saturating_add);
        rounded_weighted_point_count(weighted)
    }

    fn coverage_eighths(&self, key: BatchKey) -> u8 {
        if self.pending_replacements.contains(&key) {
            return 0;
        }
        self.active
            .values()
            .find_map(|transition| transition.coverage_eighths(key))
            .unwrap_or(CROSS_FADE_PRESENTED_FRAMES)
    }
}

fn conditional_retained(node: RetainedNode) -> ConditionalBatch {
    ConditionalBatch {
        view_generation: node.view_generation(),
        key: node.batch_key(),
        expected_version: node.version(),
    }
}

fn transition_seed(anchor: BatchKey) -> u32 {
    let key = anchor.get();
    u32::try_from((key ^ (key >> 32)) & u64::from(u32::MAX)).expect("masked group seed fits u32")
}

fn connected_cut(
    hierarchy: &HierarchyIndex<'_>,
    first: ConditionalBatch,
    outgoing: &mut Vec<ConditionalBatch>,
    incoming: &mut Vec<ConditionalBatch>,
    hidden: &BTreeSet<BatchKey>,
) -> (Vec<ConditionalBatch>, Vec<ConditionalBatch>) {
    let mut group_outgoing = vec![first];
    let mut group_incoming = Vec::new();
    loop {
        let before = group_outgoing.len() + group_incoming.len();
        incoming.retain(|batch| {
            if group_outgoing
                .iter()
                .any(|other| hierarchy.related_batches(batch.key, other.key))
            {
                group_incoming.push(*batch);
                false
            } else {
                true
            }
        });
        outgoing.retain(|batch| {
            if !hidden.contains(&batch.key)
                && group_incoming
                    .iter()
                    .any(|other| hierarchy.related_batches(batch.key, other.key))
            {
                group_outgoing.push(*batch);
                false
            } else {
                true
            }
        });
        if before == group_outgoing.len() + group_incoming.len() {
            break;
        }
    }
    (group_outgoing, group_incoming)
}

pub(crate) fn projected_density_point_size(viewport: Viewport, drawn_points: u64) -> f32 {
    if drawn_points == 0 {
        return MAX_POINT_SIZE_PIXELS;
    }
    let physical_pixels = f64::from(viewport.width()) * f64::from(viewport.height());
    let density_sample = u32::try_from(drawn_points.min(u64::from(u32::MAX)))
        .expect("the point count is bounded to u32");
    let projected_spacing = (physical_pixels / f64::from(density_sample)).sqrt();
    #[allow(clippy::cast_possible_truncation)]
    let diameter = (projected_spacing * 0.55) as f32;
    diameter.clamp(MIN_POINT_SIZE_PIXELS, MAX_POINT_SIZE_PIXELS)
}

fn pending_replacement_batches(
    hierarchy: &HierarchyIndex<'_>,
    retained: &BTreeSet<NodeKey>,
    demanded: &[NodeKey],
) -> BTreeSet<BatchKey> {
    let fallback_ancestors = demanded
        .iter()
        .filter_map(|node| hierarchy.nearest_retained_ancestor(*node, retained))
        .collect::<BTreeSet<_>>();
    let demanded = demanded.iter().copied().collect::<BTreeSet<_>>();

    let coarsening = demanded
        .iter()
        .filter(|ancestor| {
            retained
                .iter()
                .any(|node| hierarchy.is_descendant(*node, **ancestor))
        })
        .copied()
        .collect::<BTreeSet<_>>();
    hierarchy
        .iter()
        .filter(|node| retained.contains(&node.key()) || demanded.contains(&node.key()))
        .filter(|node| {
            coarsening.contains(&node.key())
                || fallback_ancestors
                    .iter()
                    .any(|ancestor| hierarchy.is_descendant(node.key(), *ancestor))
        })
        .map(|node| node.batch_key())
        .collect()
}

fn resident_batch(
    hierarchy: &HierarchyIndex<'_>,
    plan: &ViewPlan,
    key: BatchKey,
) -> Option<ConditionalBatch> {
    let node = hierarchy.iter().find(|node| node.batch_key() == key)?;
    let NodeStatus::Resident { version } = node.status() else {
        return None;
    };
    Some(ConditionalBatch {
        view_generation: plan.view_generation(),
        key,
        expected_version: version,
    })
}

struct HierarchyIndex<'a> {
    hierarchy: &'a [AvailableNode],
    nodes_by_key: BTreeMap<NodeKey, AvailableNode>,
}

impl<'a> HierarchyIndex<'a> {
    fn new(hierarchy: &'a [AvailableNode]) -> Self {
        Self {
            hierarchy,
            nodes_by_key: hierarchy
                .iter()
                .copied()
                .map(|node| (node.key(), node))
                .collect(),
        }
    }

    fn iter(&self) -> impl Iterator<Item = &AvailableNode> {
        self.hierarchy.iter()
    }

    fn related_batches(&self, left: BatchKey, right: BatchKey) -> bool {
        let node = |key| {
            self.iter()
                .find(|node| node.batch_key() == key)
                .map(|node| node.key())
        };
        match (node(left), node(right)) {
            (Some(left), Some(right)) => {
                self.is_descendant(left, right) || self.is_descendant(right, left)
            }
            _ => false,
        }
    }

    fn nearest_retained_ancestor(
        &self,
        node: NodeKey,
        retained: &BTreeSet<NodeKey>,
    ) -> Option<NodeKey> {
        let mut parent = self.nodes_by_key.get(&node).and_then(|node| node.parent());
        while let Some(candidate) = parent {
            if retained.contains(&candidate) {
                return Some(candidate);
            }
            parent = self
                .nodes_by_key
                .get(&candidate)
                .and_then(|node| node.parent());
        }
        None
    }

    fn is_descendant(&self, mut candidate: NodeKey, ancestor: NodeKey) -> bool {
        while let Some(parent) = self
            .nodes_by_key
            .get(&candidate)
            .and_then(|node| node.parent())
        {
            if parent == ancestor {
                return true;
            }
            candidate = parent;
        }
        false
    }
}

fn rounded_weighted_point_count(numerator: u128) -> u64 {
    let denominator = u128::from(CROSS_FADE_PRESENTED_FRAMES);
    let rounded = (numerator + denominator / 2) / denominator;
    u64::try_from(rounded).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use point_view::{AvailableNodes, AxisAlignedBox, PlannerConfig, PlanningBudget, ViewPlanner};

    fn batch(key: u64) -> ConditionalBatch {
        ConditionalBatch {
            view_generation: ViewGenerationKey::new(render_protocol::ViewId::new(1), 1),
            key: BatchKey::new(key),
            expected_version: BatchVersion::new(1),
        }
    }

    fn node(key: u64, parent: Option<u64>, points: u64, status: NodeStatus) -> AvailableNode {
        AvailableNode::new(
            NodeKey::new(key).unwrap(),
            parent.map(|key| NodeKey::new(key).unwrap()),
            AxisAlignedBox::new([0.0; 3], [1.0; 3]).unwrap(),
            1.0,
            points,
            1,
            BatchKey::new(key),
            status,
        )
        .unwrap()
    }

    fn hierarchy() -> [AvailableNode; 3] {
        let resident = NodeStatus::Resident {
            version: BatchVersion::new(1),
        };
        [
            node(1, None, 100, resident),
            node(2, Some(1), 80, resident),
            node(3, Some(1), 120, resident),
        ]
    }

    fn plan(hierarchy: &[AvailableNode], refined: bool) -> ViewPlan {
        let camera = render_protocol::Camera::orthographic(
            [0.5, -5.0, 0.5],
            [0.5; 3],
            [0.0, 0.0, 1.0],
            if refined { 1.0 } else { 1_000.0 },
            0.1,
            100.0,
        )
        .unwrap();
        ViewPlanner::new(PlannerConfig::new(2.0, 0.25).unwrap())
            .plan(
                &camera,
                Viewport::new(320, 240).unwrap(),
                AvailableNodes::new(batch(1).view_generation, hierarchy),
                PlanningBudget::new(1_000, 1_000, 16),
            )
            .unwrap()
    }

    #[test]
    fn refinement_and_many_to_one_coarsening_finish_after_eight_presentations() {
        for refined in [true, false] {
            let mut hierarchy = hierarchy();
            let plan = plan(&hierarchy, refined);
            let mut transitions = DensityTransitions::default();
            let initial = transitions.reconcile(&hierarchy, &plan);
            assert_eq!(initial.len(), 3);
            assert!(
                initial
                    .iter()
                    .all(|action| action.retiring_batch().is_none())
            );
            assert_eq!(transitions.active.len(), 1);
            let expected_outgoing = if refined {
                vec![batch(1)]
            } else {
                vec![batch(2), batch(3)]
            };
            let expected_incoming = if refined {
                vec![batch(2), batch(3)]
            } else {
                vec![batch(1)]
            };
            let transition = transitions.active.values().next().unwrap();
            assert_eq!(
                transition.outgoing.iter().copied().collect::<BTreeSet<_>>(),
                expected_outgoing.iter().copied().collect()
            );
            assert_eq!(transition.incoming, expected_incoming);
            let start = if refined { 100 } else { 200 };
            let finish = if refined { 200 } else { 100 };
            assert_eq!(
                transitions.display_density_point_count_in(&hierarchy),
                start
            );
            for frame in 1..CROSS_FADE_PRESENTED_FRAMES {
                assert!(transitions.reconcile(&hierarchy, &plan).is_empty());
                assert!(transitions.blocks_new_residency());
                assert_eq!(transitions.advance_presented_frame().len(), 3);
                let expected = (start * u64::from(8 - frame) + finish * u64::from(frame) + 4) / 8;
                assert_eq!(
                    transitions.display_density_point_count_in(&hierarchy),
                    expected
                );
            }
            let final_actions = transitions.advance_presented_frame();
            assert!(!transitions.blocks_new_residency());
            for batch in expected_outgoing {
                assert!(final_actions.contains(&TransitionAction::Retire(batch)));
                for node in &mut hierarchy {
                    if node.batch_key() == batch.key {
                        *node = node.with_status(NodeStatus::Missing);
                    }
                }
            }
            for batch in expected_incoming {
                assert!(final_actions.contains(&TransitionAction::Present {
                    batch,
                    transition: RasterTransition::FULL
                }));
            }
            assert_eq!(
                transitions.display_density_point_count_in(&hierarchy),
                finish
            );
        }
    }

    #[test]
    fn interruption_returns_only_the_current_retained_cut() {
        let hierarchy = hierarchy();
        let mut transitions = DensityTransitions::default();
        transitions.reconcile(&hierarchy, &plan(&hierarchy, true));
        for _ in 0..3 {
            transitions.advance_presented_frame();
        }
        let actions = transitions.reconcile(&hierarchy, &plan(&hierarchy, false));
        assert!(!transitions.is_active());
        assert!(actions.contains(&TransitionAction::Present {
            batch: batch(1),
            transition: RasterTransition::FULL
        }));
        assert!(actions.contains(&TransitionAction::Retire(batch(2))));
        assert!(actions.contains(&TransitionAction::Retire(batch(3))));
        assert!(!actions.iter().any(|action| matches!(action, TransitionAction::Present { batch, .. } if batch.key != BatchKey::new(1))));
        assert!(transitions.advance_presented_frame().is_empty());
    }

    #[test]
    fn partial_refinement_hides_replacements_before_upload_without_density_pulse() {
        let mut hierarchy = hierarchy();
        hierarchy[2] = hierarchy[2].with_status(NodeStatus::Missing);
        let mut transitions = DensityTransitions::default();
        let actions = transitions.reconcile(&hierarchy, &plan(&hierarchy, true));
        assert!(!transitions.is_active());
        assert!(actions.contains(&TransitionAction::Present {
            batch: batch(2),
            transition: RasterTransition::HIDDEN
        }));
        assert_eq!(
            transitions.uploaded_batch_presentation(batch(3)),
            Some(TransitionAction::Present {
                batch: batch(3),
                transition: RasterTransition::HIDDEN
            })
        );
        assert_eq!(transitions.display_density_point_count_in(&hierarchy), 100);
        hierarchy[2] = hierarchy[2].with_status(NodeStatus::Resident {
            version: BatchVersion::new(1),
        });
        transitions.reconcile(&hierarchy, &plan(&hierarchy, true));
        assert!(transitions.is_active());
        assert_eq!(transitions.display_density_point_count_in(&hierarchy), 100);
    }

    #[test]
    fn missing_coarse_ancestor_is_hidden_before_its_first_frame() {
        let mut hierarchy = hierarchy();
        hierarchy[0] = hierarchy[0].with_status(NodeStatus::Missing);
        let mut transitions = DensityTransitions::default();
        transitions.reconcile(&hierarchy, &plan(&hierarchy, false));
        assert_eq!(
            transitions.uploaded_batch_presentation(batch(1)),
            Some(TransitionAction::Present {
                batch: batch(1),
                transition: RasterTransition::HIDDEN
            })
        );
        assert_eq!(transitions.display_density_point_count_in(&hierarchy), 200);
    }

    #[test]
    fn paused_presentations_and_generation_reset_cannot_advance_old_groups() {
        let hierarchy = hierarchy();
        let plan = plan(&hierarchy, true);
        let mut transitions = DensityTransitions::default();
        transitions.reconcile(&hierarchy, &plan);
        for _ in 0..30 {
            assert!(transitions.reconcile(&hierarchy, &plan).is_empty());
        }
        assert_eq!(
            transitions.active.values().next().unwrap().presented_frames,
            0
        );
        transitions.view_generation =
            Some(ViewGenerationKey::new(render_protocol::ViewId::new(1), 99));
        let restarted = transitions.reconcile(&hierarchy, &plan);
        assert_eq!(restarted.len(), 3);
        assert_eq!(transitions.active.len(), 1);
        assert_eq!(
            transitions.active.values().next().unwrap().presented_frames,
            0
        );
    }

    #[test]
    fn transition_actions_own_their_exact_conditional_protocol_mapping() {
        let batch = batch(7);
        let transition = RasterTransition::new(31, RasterTransitionSide::Incoming, 4).unwrap();
        let presentation = TransitionAction::Present { batch, transition };
        assert_eq!(
            presentation.render_update(),
            RenderUpdate::SetBatchRasterTransition {
                view_generation: batch.view_generation,
                key: batch.key,
                expected_version: batch.expected_version,
                transition
            }
        );
        assert_eq!(presentation.retiring_batch(), None);
        let retirement = TransitionAction::Retire(batch);
        assert_eq!(
            retirement.render_update(),
            RenderUpdate::Remove {
                view_generation: batch.view_generation,
                key: batch.key,
                expected_version: batch.expected_version
            }
        );
        assert_eq!(retirement.retiring_batch(), Some(batch));
    }

    #[test]
    fn renderer_appearance_and_projected_density_remain_bounded() {
        assert_eq!(
            renderer_appearance_config(
                wgpu::TextureFormat::Rgba8Unorm,
                RenderLimits::new(24, 1, 1)
            )
            .point_footprint(),
            PointFootprint::Antialiased
        );
        let viewport = Viewport::new(2_560, 1_664).unwrap();
        assert_eq!(
            projected_density_point_size(viewport, 0).to_bits(),
            4.0_f32.to_bits()
        );
        assert_eq!(
            projected_density_point_size(viewport, u64::MAX).to_bits(),
            1.0_f32.to_bits()
        );
        assert!(
            projected_density_point_size(viewport, 100_000)
                > projected_density_point_size(viewport, 600_000)
        );
    }
}

#[cfg(test)]
mod gpu_tests;
