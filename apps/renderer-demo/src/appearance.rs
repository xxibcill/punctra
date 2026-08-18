use std::collections::{BTreeMap, BTreeSet};

use point_view::{AvailableNode, NodeKey, NodeStatus, RetainedNode, ViewPlan};
use render_protocol::{
    BatchKey, BatchVersion, PresentationWeight, RenderLimits, RenderUpdate, UpdateReport,
    ViewGenerationKey, Viewport,
};
use render_wgpu::{EyeDomeLighting, RendererConfig, RendererError, WgpuRenderer};

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
    RendererConfig::new(color_format, limits).with_eye_dome_lighting(depth_cue)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ConditionalBatch {
    pub(crate) view_generation: ViewGenerationKey,
    pub(crate) key: BatchKey,
    pub(crate) expected_version: BatchVersion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransitionAction {
    Present {
        batch: ConditionalBatch,
        weight: PresentationWeight,
    },
    Retire(ConditionalBatch),
}

impl TransitionAction {
    pub(crate) fn render_update(self) -> RenderUpdate {
        match self {
            Self::Present { batch, weight } => RenderUpdate::SetBatchPresentation {
                view_generation: batch.view_generation,
                key: batch.key,
                expected_version: batch.expected_version,
                weight,
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
    retiring: ConditionalBatch,
    replacements: Vec<ConditionalBatch>,
    presented_frames: u8,
}

impl ActiveTransition {
    fn controls(&self, key: BatchKey) -> bool {
        self.retiring.key == key
            || self
                .replacements
                .iter()
                .any(|replacement| replacement.key == key)
    }
}

#[derive(Default)]
pub(crate) struct DensityTransitions {
    active: BTreeMap<BatchKey, ActiveTransition>,
    pending_replacements: BTreeSet<BatchKey>,
}

impl DensityTransitions {
    pub(crate) fn reconcile(
        &mut self,
        hierarchy: &[AvailableNode],
        plan: &ViewPlan,
    ) -> Vec<TransitionAction> {
        let planned_retirements = plan
            .retirements()
            .iter()
            .map(|retirement| retirement.batch_key())
            .collect::<BTreeSet<_>>();
        let mut actions = Vec::new();

        self.active.retain(|key, transition| {
            if planned_retirements.contains(key) {
                true
            } else {
                actions.push(TransitionAction::Present {
                    batch: transition.retiring,
                    weight: PresentationWeight::OPAQUE,
                });
                actions.extend(transition.replacements.iter().copied().map(|batch| {
                    TransitionAction::Present {
                        batch,
                        weight: PresentationWeight::OPAQUE,
                    }
                }));
                false
            }
        });
        actions.extend(self.reconcile_pending_replacements(hierarchy, plan));

        for retirement in plan.retirements().iter().copied() {
            if self.active.contains_key(&retirement.batch_key()) {
                continue;
            }
            let retiring = ConditionalBatch {
                view_generation: retirement.view_generation(),
                key: retirement.batch_key(),
                expected_version: retirement.expected_version(),
            };
            let replacements = replacement_batches(hierarchy, plan.retained_nodes(), retiring.key);
            if !self.transition_is_disjoint(retiring, &replacements) {
                continue;
            }
            if replacements.is_empty() {
                actions.push(TransitionAction::Retire(retiring));
                continue;
            }
            actions.push(TransitionAction::Present {
                batch: retiring,
                weight: PresentationWeight::OPAQUE,
            });
            actions.extend(
                replacements
                    .iter()
                    .copied()
                    .map(|batch| TransitionAction::Present {
                        batch,
                        weight: PresentationWeight::TRANSPARENT,
                    }),
            );
            self.active.insert(
                retiring.key,
                ActiveTransition {
                    retiring,
                    replacements,
                    presented_frames: 0,
                },
            );
        }
        actions
    }

    pub(crate) fn uploaded_batch_presentation(
        &self,
        batch: ConditionalBatch,
    ) -> Option<TransitionAction> {
        self.pending_replacements
            .contains(&batch.key)
            .then_some(TransitionAction::Present {
                batch,
                weight: PresentationWeight::TRANSPARENT,
            })
    }

    pub(crate) fn advance_presented_frame(&mut self) -> Vec<TransitionAction> {
        let mut actions = Vec::new();
        let mut completed = Vec::new();
        for (key, transition) in &mut self.active {
            transition.presented_frames = transition.presented_frames.saturating_add(1);
            let replacement_weight = weight_for_step(transition.presented_frames);
            let retiring_weight = weight_for_step(
                CROSS_FADE_PRESENTED_FRAMES.saturating_sub(transition.presented_frames),
            );
            actions.push(TransitionAction::Present {
                batch: transition.retiring,
                weight: retiring_weight,
            });
            actions.extend(transition.replacements.iter().copied().map(|batch| {
                TransitionAction::Present {
                    batch,
                    weight: replacement_weight,
                }
            }));
            if transition.presented_frames == CROSS_FADE_PRESENTED_FRAMES {
                actions.push(TransitionAction::Retire(transition.retiring));
                completed.push(*key);
            }
        }
        for key in completed {
            self.active.remove(&key);
        }
        actions
    }

    pub(crate) fn is_active(&self) -> bool {
        !self.active.is_empty()
    }

    pub(crate) fn blocks_new_residency(&self) -> bool {
        self.is_active()
    }

    fn reconcile_pending_replacements(
        &mut self,
        hierarchy: &[AvailableNode],
        plan: &ViewPlan,
    ) -> Vec<TransitionAction> {
        let retained = plan
            .retained_nodes()
            .iter()
            .map(|node| node.node_key())
            .collect();
        let next = pending_replacement_batches(hierarchy, &retained, plan.demanded_nodes());
        let mut actions = Vec::new();
        for key in self.pending_replacements.difference(&next) {
            if let Some(batch) = resident_batch(hierarchy, plan, *key) {
                actions.push(TransitionAction::Present {
                    batch,
                    weight: PresentationWeight::OPAQUE,
                });
            }
        }
        for key in next.difference(&self.pending_replacements) {
            if let Some(batch) = resident_batch(hierarchy, plan, *key) {
                actions.push(TransitionAction::Present {
                    batch,
                    weight: PresentationWeight::TRANSPARENT,
                });
            }
        }
        self.pending_replacements = next;
        actions
    }

    fn transition_is_disjoint(
        &self,
        retiring: ConditionalBatch,
        replacements: &[ConditionalBatch],
    ) -> bool {
        self.active.values().all(|active| {
            !active.controls(retiring.key)
                && replacements
                    .iter()
                    .all(|replacement| !active.controls(replacement.key))
        })
    }
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

fn replacement_batches(
    hierarchy: &[AvailableNode],
    retained: &[RetainedNode],
    retiring_batch: BatchKey,
) -> Vec<ConditionalBatch> {
    let nodes_by_key = hierarchy
        .iter()
        .copied()
        .map(|node| (node.key(), node))
        .collect::<BTreeMap<_, _>>();
    let Some(retiring_node) = hierarchy
        .iter()
        .find(|node| node.batch_key() == retiring_batch)
        .map(|node| node.key())
    else {
        return Vec::new();
    };
    retained
        .iter()
        .copied()
        .filter(|retained| is_descendant(retained.node_key(), retiring_node, &nodes_by_key))
        .map(|retained| ConditionalBatch {
            view_generation: retained.view_generation(),
            key: retained.batch_key(),
            expected_version: retained.version(),
        })
        .collect()
}

fn pending_replacement_batches(
    hierarchy: &[AvailableNode],
    retained: &BTreeSet<NodeKey>,
    demanded: &[NodeKey],
) -> BTreeSet<BatchKey> {
    let nodes = hierarchy
        .iter()
        .copied()
        .map(|node| (node.key(), node))
        .collect::<BTreeMap<_, _>>();
    let fallback_ancestors = demanded
        .iter()
        .filter_map(|node| nearest_retained_ancestor(*node, retained, &nodes))
        .collect::<BTreeSet<_>>();
    let demanded = demanded.iter().copied().collect::<BTreeSet<_>>();

    hierarchy
        .iter()
        .filter(|node| retained.contains(&node.key()) || demanded.contains(&node.key()))
        .filter(|node| {
            fallback_ancestors
                .iter()
                .any(|ancestor| is_descendant(node.key(), *ancestor, &nodes))
        })
        .map(|node| node.batch_key())
        .collect()
}

fn nearest_retained_ancestor(
    node: NodeKey,
    retained: &BTreeSet<NodeKey>,
    nodes: &BTreeMap<NodeKey, AvailableNode>,
) -> Option<NodeKey> {
    let mut parent = nodes.get(&node).and_then(|node| node.parent());
    while let Some(candidate) = parent {
        if retained.contains(&candidate) {
            return Some(candidate);
        }
        parent = nodes.get(&candidate).and_then(|node| node.parent());
    }
    None
}

fn resident_batch(
    hierarchy: &[AvailableNode],
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

fn is_descendant(
    mut candidate: NodeKey,
    ancestor: NodeKey,
    nodes: &BTreeMap<NodeKey, AvailableNode>,
) -> bool {
    while let Some(parent) = nodes.get(&candidate).and_then(|node| node.parent()) {
        if parent == ancestor {
            return true;
        }
        candidate = parent;
    }
    false
}

fn weight_for_step(step: u8) -> PresentationWeight {
    let numerator = u16::from(step.min(CROSS_FADE_PRESENTED_FRAMES));
    let denominator = u16::from(CROSS_FADE_PRESENTED_FRAMES);
    let value = (numerator * u16::from(u8::MAX) + denominator / 2) / denominator;
    PresentationWeight::new(u8::try_from(value).expect("an eighth weight fits in u8"))
}

#[cfg(test)]
mod tests {
    use point_view::AxisAlignedBox;
    use render_protocol::{BatchKey, BatchVersion, ViewGenerationKey, ViewId};

    use super::*;

    fn batch(key: u64) -> ConditionalBatch {
        ConditionalBatch {
            view_generation: ViewGenerationKey::new(ViewId::new(1), 1),
            key: BatchKey::new(key),
            expected_version: BatchVersion::new(1),
        }
    }

    fn node_key(key: u64) -> NodeKey {
        NodeKey::new(key).unwrap()
    }

    fn node(key: u64, parent: Option<u64>) -> AvailableNode {
        AvailableNode::new(
            node_key(key),
            parent.map(node_key),
            AxisAlignedBox::new([0.0; 3], [1.0; 3]).unwrap(),
            1.0,
            1,
            1,
            BatchKey::new(key),
            NodeStatus::Missing,
        )
        .unwrap()
    }

    #[test]
    fn cross_fade_uses_exactly_eight_presented_frames() {
        let retiring = batch(1);
        let replacement = batch(2);
        let mut transitions = DensityTransitions::default();
        transitions.active.insert(
            retiring.key,
            ActiveTransition {
                retiring,
                replacements: vec![replacement],
                presented_frames: 0,
            },
        );

        for frame in 1..CROSS_FADE_PRESENTED_FRAMES {
            let actions = transitions.advance_presented_frame();
            assert!(transitions.is_active());
            assert!(!actions.contains(&TransitionAction::Retire(retiring)));
            assert!(actions.contains(&TransitionAction::Present {
                batch: replacement,
                weight: weight_for_step(frame),
            }));
        }
        let final_actions = transitions.advance_presented_frame();
        assert!(!transitions.is_active());
        assert!(final_actions.contains(&TransitionAction::Present {
            batch: replacement,
            weight: PresentationWeight::OPAQUE,
        }));
        assert!(final_actions.contains(&TransitionAction::Retire(retiring)));
    }

    #[test]
    fn active_cross_fade_blocks_new_residency_until_retirement() {
        let retiring = batch(1);
        let mut transitions = DensityTransitions::default();
        transitions.active.insert(
            retiring.key,
            ActiveTransition {
                retiring,
                replacements: vec![batch(2)],
                presented_frames: 0,
            },
        );

        assert!(transitions.blocks_new_residency());
        for _ in 0..CROSS_FADE_PRESENTED_FRAMES {
            transitions.advance_presented_frame();
        }
        assert!(!transitions.blocks_new_residency());
    }

    #[test]
    fn incomplete_replacement_coverage_stays_transparent() {
        let hierarchy = [node(1, None), node(2, Some(1)), node(3, Some(1))];
        let retained = [node_key(1), node_key(2)].into_iter().collect();
        let pending = pending_replacement_batches(&hierarchy, &retained, &[node_key(3)]);
        assert_eq!(
            pending,
            [BatchKey::new(2), BatchKey::new(3)].into_iter().collect()
        );

        let transitions = DensityTransitions {
            pending_replacements: pending,
            ..DensityTransitions::default()
        };
        assert_eq!(
            transitions.uploaded_batch_presentation(batch(3)),
            Some(TransitionAction::Present {
                batch: batch(3),
                weight: PresentationWeight::TRANSPARENT,
            })
        );
    }

    #[test]
    fn nested_transition_waits_for_its_ancestor_transition() {
        let mut transitions = DensityTransitions::default();
        transitions.active.insert(
            BatchKey::new(1),
            ActiveTransition {
                retiring: batch(1),
                replacements: vec![batch(2)],
                presented_frames: 3,
            },
        );

        assert!(!transitions.transition_is_disjoint(batch(2), &[batch(3)]));
        assert!(!transitions.transition_is_disjoint(batch(4), &[batch(2)]));
        assert!(transitions.transition_is_disjoint(batch(4), &[batch(5)]));
    }

    #[test]
    fn transition_actions_own_their_protocol_mapping() {
        let conditional = batch(7);
        let weight = PresentationWeight::new(91);
        let presentation = TransitionAction::Present {
            batch: conditional,
            weight,
        };
        assert_eq!(
            presentation.render_update(),
            RenderUpdate::SetBatchPresentation {
                view_generation: conditional.view_generation,
                key: conditional.key,
                expected_version: conditional.expected_version,
                weight,
            }
        );
        assert_eq!(presentation.retiring_batch(), None);

        let retirement = TransitionAction::Retire(conditional);
        assert_eq!(
            retirement.render_update(),
            RenderUpdate::Remove {
                view_generation: conditional.view_generation,
                key: conditional.key,
                expected_version: conditional.expected_version,
            }
        );
        assert_eq!(retirement.retiring_batch(), Some(conditional));
    }

    #[test]
    fn projected_density_policy_is_bounded_and_density_sensitive() {
        let viewport = Viewport::new(2_560, 1_664).unwrap();
        assert!((projected_density_point_size(viewport, 0) - 4.0).abs() < f32::EPSILON);
        assert!((projected_density_point_size(viewport, 1) - 4.0).abs() < f32::EPSILON);
        assert!((projected_density_point_size(viewport, u64::MAX) - 1.0).abs() < f32::EPSILON);
        assert!(
            projected_density_point_size(viewport, 100_000)
                > projected_density_point_size(viewport, 600_000)
        );
    }
}

#[cfg(test)]
mod gpu_tests;
