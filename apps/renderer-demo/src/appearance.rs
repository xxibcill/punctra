use std::collections::{BTreeMap, BTreeSet};

use point_view::{AvailableNode, NodeKey, RetainedNode, ViewPlan};
use render_protocol::{BatchKey, BatchVersion, PresentationWeight, ViewGenerationKey, Viewport};

pub(crate) const CROSS_FADE_PRESENTED_FRAMES: u8 = 8;
pub(crate) const MIN_POINT_SIZE_PIXELS: f32 = 1.0;
pub(crate) const MAX_POINT_SIZE_PIXELS: f32 = 4.0;
pub(crate) const REFERENCE_POINT_SIZE_PIXELS: f32 = 2.4;

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

#[derive(Clone, Debug)]
struct ActiveTransition {
    retiring: ConditionalBatch,
    replacements: Vec<ConditionalBatch>,
    presented_frames: u8,
}

#[derive(Default)]
pub(crate) struct DensityTransitions {
    active: BTreeMap<BatchKey, ActiveTransition>,
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

    #[cfg(test)]
    fn is_active(&self) -> bool {
        !self.active.is_empty()
    }
}

pub(crate) fn projected_spacing_point_size(viewport: Viewport, drawn_points: u64) -> f32 {
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
    use render_protocol::{BatchKey, BatchVersion, ViewGenerationKey, ViewId};

    use super::*;

    fn batch(key: u64) -> ConditionalBatch {
        ConditionalBatch {
            view_generation: ViewGenerationKey::new(ViewId::new(1), 1),
            key: BatchKey::new(key),
            expected_version: BatchVersion::new(1),
        }
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
    fn projected_spacing_policy_is_bounded_and_density_sensitive() {
        let viewport = Viewport::new(2_560, 1_664).unwrap();
        assert!((projected_spacing_point_size(viewport, 0) - 4.0).abs() < f32::EPSILON);
        assert!((projected_spacing_point_size(viewport, 1) - 4.0).abs() < f32::EPSILON);
        assert!((projected_spacing_point_size(viewport, u64::MAX) - 1.0).abs() < f32::EPSILON);
        assert!(
            projected_spacing_point_size(viewport, 100_000)
                > projected_spacing_point_size(viewport, 600_000)
        );
    }
}

#[cfg(test)]
mod gpu_tests;
