use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

use glam::DVec3;
use render_protocol::{Camera, CameraProjection, Viewport};

use crate::{
    AvailableNode, AvailableNodes, AxisAlignedBox, NodeKey, NodeRequest, NodeStatus, PlanError,
    PlanningBudget, ResourceUsage, RetainedNode, Retirement, ViewPlan, ViewPlanner,
};

pub(super) fn plan(
    planner: &mut ViewPlanner,
    camera: &Camera,
    viewport: Viewport,
    available_nodes: AvailableNodes<'_>,
    budget: PlanningBudget,
) -> Result<ViewPlan, PlanError> {
    let projection = ProjectedCamera::new(camera, viewport);
    let hierarchy = ProjectedHierarchy::new(available_nodes.nodes, &projection)?;
    let previous_refinements = if planner.active_generation == Some(available_nodes.view_generation)
    {
        &planner.refined_nodes
    } else {
        &BTreeSet::new()
    };

    let (target_cut, next_refinements) =
        select_target_cut(&hierarchy, previous_refinements, planner.config, budget)?;
    let retained_mask = required_residents(&hierarchy, &target_cut);
    let demanded_nodes = demanded_nodes(&hierarchy, &target_cut);
    let requests = select_requests(
        &hierarchy,
        &target_cut,
        &retained_mask,
        budget,
        available_nodes.view_generation,
    )?;
    let resource_usage =
        planned_resource_usage(&hierarchy, &retained_mask, &demanded_nodes, &requests)?;
    let retained = retained_nodes(&hierarchy, &retained_mask, available_nodes.view_generation);
    let retirements = retirements(&hierarchy, &retained_mask, available_nodes.view_generation);

    planner.active_generation = Some(available_nodes.view_generation);
    planner.refined_nodes = next_refinements;

    Ok(ViewPlan {
        view_generation: available_nodes.view_generation,
        demanded_nodes,
        requests,
        retained,
        retirements,
        resource_usage,
    })
}

#[derive(Debug)]
struct ProjectedHierarchy {
    nodes: Vec<AvailableNode>,
    node_projections: Vec<NodeProjection>,
    parents: Vec<Option<usize>>,
    children: Vec<Vec<usize>>,
    roots: Vec<usize>,
}

impl ProjectedHierarchy {
    fn new(input_nodes: &[AvailableNode], projection: &ProjectedCamera) -> Result<Self, PlanError> {
        let mut nodes = input_nodes.to_vec();
        nodes.sort_by_key(|node| node.key);
        validate_unique_keys(&nodes)?;

        let index_by_key = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.key, index))
            .collect::<BTreeMap<_, _>>();
        let parents = resolve_parents(&nodes, &index_by_key)?;
        validate_acyclic(&nodes, &parents)?;
        validate_child_bounds(&nodes, &parents)?;

        let mut children = vec![Vec::new(); nodes.len()];
        let mut roots = Vec::new();
        for (index, parent) in parents.iter().copied().enumerate() {
            if let Some(parent) = parent {
                children[parent].push(index);
            } else {
                roots.push(index);
            }
        }
        let node_projections = nodes
            .iter()
            .map(|node| projection.node_projection(node))
            .collect();

        Ok(Self {
            nodes,
            node_projections,
            parents,
            children,
            roots,
        })
    }

    fn visible_children(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        self.children[index]
            .iter()
            .copied()
            .filter(|child| self.node_projections[*child].visible)
    }
}

fn validate_unique_keys(nodes: &[AvailableNode]) -> Result<(), PlanError> {
    for duplicate in nodes.windows(2) {
        if duplicate[0].key == duplicate[1].key {
            return Err(PlanError::DuplicateNodeKey {
                key: duplicate[0].key,
            });
        }
    }

    let mut batch_keys = BTreeSet::new();
    for node in nodes {
        if !batch_keys.insert(node.batch_key) {
            return Err(PlanError::DuplicateBatchKey {
                key: node.batch_key,
            });
        }
    }
    Ok(())
}

fn resolve_parents(
    nodes: &[AvailableNode],
    index_by_key: &BTreeMap<NodeKey, usize>,
) -> Result<Vec<Option<usize>>, PlanError> {
    nodes
        .iter()
        .map(|node| {
            node.parent
                .map(|parent| {
                    index_by_key
                        .get(&parent)
                        .copied()
                        .ok_or(PlanError::MissingParent {
                            key: node.key,
                            parent,
                        })
                })
                .transpose()
        })
        .collect()
}

fn validate_acyclic(nodes: &[AvailableNode], parents: &[Option<usize>]) -> Result<(), PlanError> {
    let mut finished = vec![false; nodes.len()];
    let mut path_positions = vec![None; nodes.len()];
    for start in 0..nodes.len() {
        if finished[start] {
            continue;
        }

        let mut path: Vec<usize> = Vec::new();
        let mut cursor = Some(start);
        while let Some(index) = cursor {
            if finished[index] {
                break;
            }
            if let Some(cycle_start) = path_positions[index] {
                let key = path[cycle_start..]
                    .iter()
                    .map(|cycle_index| nodes[*cycle_index].key)
                    .min()
                    .expect("a repeated path index creates a non-empty cycle");
                return Err(PlanError::ParentCycle { key });
            }
            path_positions[index] = Some(path.len());
            path.push(index);
            cursor = parents[index];
        }
        for index in path {
            path_positions[index] = None;
            finished[index] = true;
        }
    }
    Ok(())
}

fn validate_child_bounds(
    nodes: &[AvailableNode],
    parents: &[Option<usize>],
) -> Result<(), PlanError> {
    for (child_index, parent_index) in parents.iter().copied().enumerate() {
        let Some(parent_index) = parent_index else {
            continue;
        };
        let child = &nodes[child_index];
        let parent = &nodes[parent_index];
        if !contains(parent.bounds, child.bounds) {
            return Err(PlanError::ChildOutsideParent {
                key: child.key,
                parent: parent.key,
            });
        }
    }
    Ok(())
}

fn contains(outer: AxisAlignedBox, inner: AxisAlignedBox) -> bool {
    (0..3).all(|axis| outer.min[axis] <= inner.min[axis] && inner.max[axis] <= outer.max[axis])
}

#[derive(Clone, Copy, Debug)]
struct NodeProjection {
    visible: bool,
    screen_error: f64,
}

#[derive(Clone, Copy, Debug)]
struct ProjectedCamera {
    eye: DVec3,
    forward: DVec3,
    screen_error_projection: ScreenErrorProjection,
    frustum: Frustum,
}

impl ProjectedCamera {
    fn new(camera: &Camera, viewport: Viewport) -> Self {
        let geometry = ViewGeometry::new(camera);
        let aspect_ratio = f64::from(viewport.aspect_ratio());
        let near_distance = f64::from(camera.near_distance());
        let far_distance = f64::from(camera.far_distance());
        let (screen_error_projection, frustum) = match camera.projection() {
            CameraProjection::Perspective {
                vertical_field_of_view_radians,
            } => {
                let half_vertical_tangent = (f64::from(vertical_field_of_view_radians) * 0.5).tan();
                let pixel_scale = f64::from(viewport.height()) / (2.0 * half_vertical_tangent);
                (
                    ScreenErrorProjection::Perspective {
                        pixel_scale,
                        near_distance,
                    },
                    Frustum::perspective(
                        geometry,
                        half_vertical_tangent,
                        aspect_ratio,
                        near_distance,
                        far_distance,
                    ),
                )
            }
            CameraProjection::Orthographic {
                vertical_world_height,
            } => (
                ScreenErrorProjection::Orthographic {
                    viewport_height: f64::from(viewport.height()),
                    vertical_world_height,
                },
                Frustum::orthographic(
                    geometry,
                    vertical_world_height * 0.5,
                    aspect_ratio,
                    near_distance,
                    far_distance,
                ),
            ),
        };
        Self {
            eye: geometry.eye,
            forward: geometry.forward,
            screen_error_projection,
            frustum,
        }
    }

    fn node_projection(&self, node: &AvailableNode) -> NodeProjection {
        NodeProjection {
            visible: self.frustum.intersects(node.bounds),
            screen_error: self.screen_error(node),
        }
    }

    fn screen_error(&self, node: &AvailableNode) -> f64 {
        match self.screen_error_projection {
            ScreenErrorProjection::Perspective {
                pixel_scale,
                near_distance,
            } => self.perspective_screen_error(node, pixel_scale, near_distance),
            ScreenErrorProjection::Orthographic {
                viewport_height,
                vertical_world_height,
            } => multiply_divide(node.geometric_error, viewport_height, vertical_world_height),
        }
    }

    fn perspective_screen_error(
        &self,
        node: &AvailableNode,
        pixel_scale: f64,
        near_distance: f64,
    ) -> f64 {
        let min = node.bounds.min;
        let max = node.bounds.max;
        let center = DVec3::new(
            min[0].midpoint(max[0]),
            min[1].midpoint(max[1]),
            min[2].midpoint(max[2]),
        );
        let half_extent = DVec3::new(
            axis_half_extent(min[0], max[0]),
            axis_half_extent(min[1], max[1]),
            axis_half_extent(min[2], max[2]),
        );
        let center_depth = self.forward.dot(center - self.eye);
        let depth_radius = self.forward.abs().dot(half_extent);
        let nearest_depth = (center_depth - depth_radius).max(near_distance);
        multiply_divide(node.geometric_error, pixel_scale, nearest_depth)
    }
}

#[derive(Clone, Copy, Debug)]
enum ScreenErrorProjection {
    Perspective {
        pixel_scale: f64,
        near_distance: f64,
    },
    Orthographic {
        viewport_height: f64,
        vertical_world_height: f64,
    },
}

#[derive(Clone, Copy, Debug)]
struct ViewGeometry {
    eye: DVec3,
    forward: DVec3,
    right: DVec3,
    up: DVec3,
}

impl ViewGeometry {
    fn new(camera: &Camera) -> Self {
        let basis = camera.world_basis();
        Self {
            eye: DVec3::from_array(camera.eye()),
            forward: DVec3::from_array(basis.forward()),
            right: DVec3::from_array(basis.right()),
            up: DVec3::from_array(basis.up()),
        }
    }
}

fn multiply_divide(left: f64, right: f64, divisor: f64) -> f64 {
    if left == 0.0 || right == 0.0 {
        return 0.0;
    }

    let product = left * right;
    if product.is_normal() {
        return product / divisor;
    }

    let (left_fraction, left_exponent) = libm::frexp(left);
    let (right_fraction, right_exponent) = libm::frexp(right);
    let (divisor_fraction, divisor_exponent) = libm::frexp(divisor);
    let fraction = left_fraction * right_fraction / divisor_fraction;
    let exponent = left_exponent + right_exponent - divisor_exponent;
    libm::scalbn(fraction, exponent)
}

fn axis_half_extent(min: f64, max: f64) -> f64 {
    (-min).midpoint(max)
}

#[derive(Clone, Copy, Debug)]
struct Plane {
    normal: DVec3,
    camera_origin: DVec3,
    offset: f64,
}

impl Plane {
    fn from_camera(normal: DVec3, camera_origin: DVec3, offset: f64) -> Self {
        let normal_length = normal.length();
        debug_assert!(normal_length.is_finite() && normal_length > 0.0);
        Self {
            normal: normal / normal_length,
            camera_origin,
            offset: offset / normal_length,
        }
    }

    fn excludes(&self, bounds: AxisAlignedBox) -> bool {
        let support = DVec3::new(
            if self.normal.x >= 0.0 {
                bounds.max[0]
            } else {
                bounds.min[0]
            },
            if self.normal.y >= 0.0 {
                bounds.max[1]
            } else {
                bounds.min[1]
            },
            if self.normal.z >= 0.0 {
                bounds.max[2]
            } else {
                bounds.min[2]
            },
        );
        let scaled_support = support * PLANE_EVALUATION_SCALE;
        let scaled_origin = self.camera_origin * PLANE_EVALUATION_SCALE;
        let relative_support = scaled_support - scaled_origin;
        let scaled_offset = self.offset * PLANE_EVALUATION_SCALE;
        let evaluation = self.normal.dot(relative_support) + scaled_offset;
        let subtraction_magnitude = scaled_support.abs() + scaled_origin.abs();
        let evaluation_magnitude =
            self.normal.abs().dot(subtraction_magnitude) + scaled_offset.abs();
        let roundoff_bound = evaluation_magnitude * PLANE_EVALUATION_ROUNDOFF_FACTOR;
        evaluation < -roundoff_bound
    }
}

// One eighth leaves headroom for opposite-sign subtraction and a three-axis unit-normal dot.
const PLANE_EVALUATION_SCALE: f64 = 0.125;
// Plane construction and evaluation use several rounded operations; expand the plane outward.
const PLANE_EVALUATION_ROUNDOFF_FACTOR: f64 = f64::EPSILON * 16.0;

#[derive(Clone, Copy, Debug)]
struct Frustum {
    planes: [Plane; 6],
}

impl Frustum {
    fn perspective(
        geometry: ViewGeometry,
        half_vertical_tangent: f64,
        aspect_ratio: f64,
        near_distance: f64,
        far_distance: f64,
    ) -> Self {
        let half_horizontal_tangent = half_vertical_tangent * aspect_ratio;
        Self {
            planes: [
                Plane::from_camera(geometry.forward, geometry.eye, -near_distance),
                Plane::from_camera(-geometry.forward, geometry.eye, far_distance),
                Plane::from_camera(
                    geometry.forward * half_horizontal_tangent + geometry.right,
                    geometry.eye,
                    0.0,
                ),
                Plane::from_camera(
                    geometry.forward * half_horizontal_tangent - geometry.right,
                    geometry.eye,
                    0.0,
                ),
                Plane::from_camera(
                    geometry.forward * half_vertical_tangent + geometry.up,
                    geometry.eye,
                    0.0,
                ),
                Plane::from_camera(
                    geometry.forward * half_vertical_tangent - geometry.up,
                    geometry.eye,
                    0.0,
                ),
            ],
        }
    }

    fn orthographic(
        geometry: ViewGeometry,
        half_vertical_world_height: f64,
        aspect_ratio: f64,
        near_distance: f64,
        far_distance: f64,
    ) -> Self {
        let half_horizontal_world_width = half_vertical_world_height * aspect_ratio;
        Self {
            planes: [
                Plane::from_camera(geometry.forward, geometry.eye, -near_distance),
                Plane::from_camera(-geometry.forward, geometry.eye, far_distance),
                Plane::from_camera(geometry.right, geometry.eye, half_horizontal_world_width),
                Plane::from_camera(-geometry.right, geometry.eye, half_horizontal_world_width),
                Plane::from_camera(geometry.up, geometry.eye, half_vertical_world_height),
                Plane::from_camera(-geometry.up, geometry.eye, half_vertical_world_height),
            ],
        }
    }

    fn intersects(&self, bounds: AxisAlignedBox) -> bool {
        self.planes.iter().all(|plane| !plane.excludes(bounds))
    }
}

#[derive(Clone, Copy, Debug)]
struct RefinementCandidate {
    index: usize,
    key: NodeKey,
    screen_error: f64,
    was_refined: bool,
}

impl PartialEq for RefinementCandidate {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
            && self.was_refined == other.was_refined
            && self.screen_error.total_cmp(&other.screen_error) == Ordering::Equal
    }
}

impl Eq for RefinementCandidate {}

impl PartialOrd for RefinementCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RefinementCandidate {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reconstruct the last accepted topology before spending budget on a
        // new refinement. Intermediate ancestors may already be retired and
        // must not become transient load targets while history is replayed.
        self.was_refined.cmp(&other.was_refined).then_with(|| {
            self.screen_error
                .total_cmp(&other.screen_error)
                .then_with(|| other.key.cmp(&self.key))
        })
    }
}

fn select_target_cut(
    hierarchy: &ProjectedHierarchy,
    previous_refinements: &BTreeSet<NodeKey>,
    config: crate::PlannerConfig,
    budget: PlanningBudget,
) -> Result<(BTreeSet<usize>, BTreeSet<NodeKey>), PlanError> {
    let mut target_cut = hierarchy
        .roots
        .iter()
        .copied()
        .filter(|index| hierarchy.node_projections[*index].visible)
        .collect::<BTreeSet<_>>();
    let mut candidates = BinaryHeap::new();
    for index in target_cut.iter().copied() {
        push_candidate(
            &mut candidates,
            hierarchy,
            previous_refinements,
            config,
            index,
        );
    }
    let mut missing_transition_targets = BTreeSet::new();

    let mut refinements = hierarchy
        .nodes
        .iter()
        .enumerate()
        .filter(|(index, node)| {
            !hierarchy.node_projections[*index].visible && previous_refinements.contains(&node.key)
        })
        .map(|(_, node)| node.key)
        .collect::<BTreeSet<_>>();
    while let Some(candidate) = candidates.pop() {
        if !target_cut.contains(&candidate.index) {
            continue;
        }
        let candidate_node = &hierarchy.nodes[candidate.index];
        if candidate_node.status == NodeStatus::Missing
            && !has_visible_coverage(hierarchy, candidate.index)
            && target_is_requestable_in_current_cut(
                hierarchy,
                &target_cut,
                budget,
                candidate.index,
            )?
        {
            continue;
        }
        let expansion = expand_candidate_and_replay_history(
            hierarchy,
            previous_refinements,
            config,
            candidate,
            &mut target_cut,
            &mut missing_transition_targets,
            &mut refinements,
        );

        if !transition_targets_fit_budget(
            hierarchy,
            &target_cut,
            &missing_transition_targets,
            budget,
        )? {
            expansion.rollback(
                hierarchy,
                &mut target_cut,
                &mut missing_transition_targets,
                &mut refinements,
            );
            continue;
        }

        for candidate in expansion.next_candidates {
            candidates.push(candidate);
        }
    }
    Ok((target_cut, refinements))
}

#[derive(Debug)]
struct RefinementExpansion {
    changes: Vec<RefinementChange>,
    next_candidates: Vec<RefinementCandidate>,
}

impl RefinementExpansion {
    fn rollback(
        self,
        hierarchy: &ProjectedHierarchy,
        target_cut: &mut BTreeSet<usize>,
        missing_transition_targets: &mut BTreeSet<usize>,
        refinements: &mut BTreeSet<NodeKey>,
    ) {
        for change in self.changes.into_iter().rev() {
            refinements.remove(&hierarchy.nodes[change.candidate_index].key);
            change.rollback(hierarchy, target_cut, missing_transition_targets);
        }
    }
}

#[derive(Debug)]
struct RefinementChange {
    candidate_index: usize,
    candidate_was_transition_target: bool,
}

impl RefinementChange {
    fn rollback(
        self,
        hierarchy: &ProjectedHierarchy,
        target_cut: &mut BTreeSet<usize>,
        missing_transition_targets: &mut BTreeSet<usize>,
    ) {
        for child in hierarchy.visible_children(self.candidate_index) {
            target_cut.remove(&child);
            missing_transition_targets.remove(&child);
        }
        target_cut.insert(self.candidate_index);
        if self.candidate_was_transition_target {
            missing_transition_targets.insert(self.candidate_index);
        }
    }
}

fn expand_candidate_and_replay_history(
    hierarchy: &ProjectedHierarchy,
    previous_refinements: &BTreeSet<NodeKey>,
    config: crate::PlannerConfig,
    candidate: RefinementCandidate,
    target_cut: &mut BTreeSet<usize>,
    missing_transition_targets: &mut BTreeSet<usize>,
    refinements: &mut BTreeSet<NodeKey>,
) -> RefinementExpansion {
    let mut replay_candidates = BinaryHeap::from([candidate]);
    let mut changes = Vec::new();
    let mut next_candidates = Vec::new();

    while let Some(candidate) = replay_candidates.pop() {
        if !target_cut.contains(&candidate.index) {
            continue;
        }
        let change = apply_refinement(
            hierarchy,
            candidate.index,
            target_cut,
            missing_transition_targets,
        );
        let inserted = refinements.insert(candidate.key);
        debug_assert!(inserted, "a target-cut candidate is refined at most once");
        for child in hierarchy.visible_children(candidate.index) {
            if let Some(child_candidate) =
                refinement_candidate(hierarchy, previous_refinements, config, child)
            {
                if child_candidate.was_refined {
                    replay_candidates.push(child_candidate);
                } else {
                    next_candidates.push(child_candidate);
                }
            }
        }
        changes.push(change);
    }

    RefinementExpansion {
        changes,
        next_candidates,
    }
}

fn apply_refinement(
    hierarchy: &ProjectedHierarchy,
    candidate_index: usize,
    target_cut: &mut BTreeSet<usize>,
    missing_transition_targets: &mut BTreeSet<usize>,
) -> RefinementChange {
    target_cut.remove(&candidate_index);
    let candidate_was_transition_target = missing_transition_targets.remove(&candidate_index);
    for child in hierarchy.visible_children(candidate_index) {
        let inserted = target_cut.insert(child);
        debug_assert!(inserted, "hierarchy children have one target-cut parent");
        if hierarchy.nodes[child].status == NodeStatus::Missing {
            let inserted = missing_transition_targets.insert(child);
            debug_assert!(inserted, "new target children are new transition targets");
        }
    }
    RefinementChange {
        candidate_index,
        candidate_was_transition_target,
    }
}

fn push_candidate(
    candidates: &mut BinaryHeap<RefinementCandidate>,
    hierarchy: &ProjectedHierarchy,
    previous_refinements: &BTreeSet<NodeKey>,
    config: crate::PlannerConfig,
    index: usize,
) {
    if let Some(candidate) = refinement_candidate(hierarchy, previous_refinements, config, index) {
        candidates.push(candidate);
    }
}

fn refinement_candidate(
    hierarchy: &ProjectedHierarchy,
    previous_refinements: &BTreeSet<NodeKey>,
    config: crate::PlannerConfig,
    index: usize,
) -> Option<RefinementCandidate> {
    let node = &hierarchy.nodes[index];
    let has_coverage = has_visible_coverage(hierarchy, index);
    let has_visible_children = hierarchy.visible_children(index).next().is_some();
    if (has_coverage || node.status == NodeStatus::Missing)
        && has_visible_children
        && exceeds_refinement_threshold(
            hierarchy.node_projections[index].screen_error,
            previous_refinements.contains(&node.key),
            config,
        )
    {
        Some(RefinementCandidate {
            index,
            key: node.key,
            screen_error: hierarchy.node_projections[index].screen_error,
            was_refined: previous_refinements.contains(&node.key),
        })
    } else {
        None
    }
}

fn has_visible_coverage(hierarchy: &ProjectedHierarchy, index: usize) -> bool {
    matches!(hierarchy.nodes[index].status, NodeStatus::Resident { .. })
        || has_visible_resident_descendant(hierarchy, index)
}

fn target_is_requestable_in_current_cut(
    hierarchy: &ProjectedHierarchy,
    target_cut: &BTreeSet<usize>,
    budget: PlanningBudget,
    target: usize,
) -> Result<bool, PlanError> {
    let retained_mask = required_residents(hierarchy, target_cut);
    Ok(
        select_request_indices(hierarchy, target_cut, &retained_mask, budget)?
            .is_some_and(|requests| requests.contains(&target)),
    )
}

fn has_visible_resident_descendant(hierarchy: &ProjectedHierarchy, index: usize) -> bool {
    let mut pending = hierarchy.children[index].clone();
    while let Some(descendant) = pending.pop() {
        if !hierarchy.node_projections[descendant].visible {
            continue;
        }
        if matches!(
            hierarchy.nodes[descendant].status,
            NodeStatus::Resident { .. }
        ) {
            return true;
        }
        pending.extend(hierarchy.children[descendant].iter().copied());
    }
    false
}

fn exceeds_refinement_threshold(
    screen_error: f64,
    was_refined: bool,
    config: crate::PlannerConfig,
) -> bool {
    if was_refined {
        screen_error >= config.max_error_pixels - config.hysteresis_pixels
    } else {
        screen_error > config.max_error_pixels + config.hysteresis_pixels
    }
}

fn transition_targets_fit_budget(
    hierarchy: &ProjectedHierarchy,
    target_cut: &BTreeSet<usize>,
    missing_transition_targets: &BTreeSet<usize>,
    budget: PlanningBudget,
) -> Result<bool, PlanError> {
    let retained_mask = required_residents(hierarchy, target_cut);
    let Some(request_indices) =
        select_request_indices(hierarchy, target_cut, &retained_mask, budget)?
    else {
        return Ok(false);
    };
    if missing_transition_targets.is_empty() {
        return Ok(true);
    }
    let request_indices = request_indices.into_iter().collect::<BTreeSet<_>>();
    Ok(missing_transition_targets
        .iter()
        .all(|index| request_indices.contains(index)))
}

fn required_residents(hierarchy: &ProjectedHierarchy, target_cut: &BTreeSet<usize>) -> Vec<bool> {
    let mut retained = vec![false; hierarchy.nodes.len()];
    let mut unavailable_targets = vec![false; hierarchy.nodes.len()];

    for index in target_cut.iter().copied() {
        if matches!(hierarchy.nodes[index].status, NodeStatus::Resident { .. }) {
            retained[index] = true;
        } else {
            unavailable_targets[index] = true;
            retain_resident_ancestors(hierarchy, index, &mut retained);
        }
    }
    for root in hierarchy.roots.iter().copied() {
        retain_resident_descendants(hierarchy, &unavailable_targets, root, false, &mut retained);
    }
    retained
}

fn retain_resident_ancestors(hierarchy: &ProjectedHierarchy, index: usize, retained: &mut [bool]) {
    let mut parent = hierarchy.parents[index];
    while let Some(parent_index) = parent {
        if matches!(
            hierarchy.nodes[parent_index].status,
            NodeStatus::Resident { .. }
        ) {
            retained[parent_index] = true;
            break;
        }
        parent = hierarchy.parents[parent_index];
    }
}

fn retain_resident_descendants(
    hierarchy: &ProjectedHierarchy,
    unavailable_targets: &[bool],
    index: usize,
    below_unavailable_target: bool,
    retained: &mut [bool],
) {
    let mut pending = vec![(index, below_unavailable_target)];
    while let Some((descendant, below_unavailable_target)) = pending.pop() {
        let below_unavailable_target = below_unavailable_target || unavailable_targets[descendant];
        if below_unavailable_target
            && hierarchy.node_projections[descendant].visible
            && matches!(
                hierarchy.nodes[descendant].status,
                NodeStatus::Resident { .. }
            )
        {
            retained[descendant] = true;
            continue;
        }
        pending.extend(
            hierarchy.children[descendant]
                .iter()
                .rev()
                .map(|child| (*child, below_unavailable_target)),
        );
    }
}

fn select_requests(
    hierarchy: &ProjectedHierarchy,
    target_cut: &BTreeSet<usize>,
    retained_mask: &[bool],
    budget: PlanningBudget,
    view_generation: render_protocol::ViewGenerationKey,
) -> Result<Vec<NodeRequest>, PlanError> {
    let Some(request_indices) =
        select_request_indices(hierarchy, target_cut, retained_mask, budget)?
    else {
        return Ok(Vec::new());
    };

    Ok(request_indices
        .into_iter()
        .map(|index| {
            let node = &hierarchy.nodes[index];
            NodeRequest {
                view_generation,
                node_key: node.key,
                batch_key: node.batch_key,
                point_count: node.point_count,
                estimated_bytes: node.estimated_bytes,
                screen_space_error_pixels: request_screen_error(hierarchy, index),
            }
        })
        .collect())
}

fn select_request_indices(
    hierarchy: &ProjectedHierarchy,
    target_cut: &BTreeSet<usize>,
    retained_mask: &[bool],
    budget: PlanningBudget,
) -> Result<Option<Vec<usize>>, PlanError> {
    let mut resource_mask = retained_mask.to_vec();
    for index in target_cut.iter().copied() {
        if hierarchy.nodes[index].status == NodeStatus::Requested {
            resource_mask[index] = true;
        }
    }
    let mut current_usage = accounted_resource_usage(hierarchy, &resource_mask)?;
    if !current_usage.fits_within(budget) {
        return Ok(None);
    }

    let mut request_indices = Vec::new();
    for index in ordered_nonresident_targets(hierarchy, target_cut) {
        if hierarchy.nodes[index].status != NodeStatus::Missing {
            continue;
        }
        let mut proposed_usage = current_usage;
        add_node_usage(&mut proposed_usage, &hierarchy.nodes[index])?;
        if !proposed_usage.fits_within(budget) {
            continue;
        }
        current_usage = proposed_usage;
        request_indices.push(index);
    }
    Ok(Some(request_indices))
}

fn demanded_nodes(hierarchy: &ProjectedHierarchy, target_cut: &BTreeSet<usize>) -> Vec<NodeKey> {
    ordered_nonresident_targets(hierarchy, target_cut)
        .into_iter()
        .map(|index| hierarchy.nodes[index].key)
        .collect()
}

fn ordered_nonresident_targets(
    hierarchy: &ProjectedHierarchy,
    target_cut: &BTreeSet<usize>,
) -> Vec<usize> {
    let mut targets = target_cut
        .iter()
        .copied()
        .filter(|index| !matches!(hierarchy.nodes[*index].status, NodeStatus::Resident { .. }))
        .collect::<Vec<_>>();
    targets.sort_by(|left, right| {
        request_screen_error(hierarchy, *right)
            .total_cmp(&request_screen_error(hierarchy, *left))
            .then_with(|| hierarchy.nodes[*left].key.cmp(&hierarchy.nodes[*right].key))
    });
    targets
}

fn request_screen_error(hierarchy: &ProjectedHierarchy, index: usize) -> f64 {
    let priority_source = hierarchy.parents[index].unwrap_or(index);
    hierarchy.node_projections[priority_source].screen_error
}

fn planned_resource_usage(
    hierarchy: &ProjectedHierarchy,
    retained_mask: &[bool],
    demanded_nodes: &[NodeKey],
    requests: &[NodeRequest],
) -> Result<ResourceUsage, PlanError> {
    let demanded_keys = demanded_nodes.iter().copied().collect::<BTreeSet<_>>();
    let request_keys = requests
        .iter()
        .map(|request| request.node_key)
        .collect::<BTreeSet<_>>();
    let mut resource_mask = retained_mask.to_vec();
    for (index, node) in hierarchy.nodes.iter().enumerate() {
        if request_keys.contains(&node.key)
            || (node.status == NodeStatus::Requested && demanded_keys.contains(&node.key))
        {
            resource_mask[index] = true;
        }
    }
    accounted_resource_usage(hierarchy, &resource_mask)
}

fn accounted_resource_usage(
    hierarchy: &ProjectedHierarchy,
    resource_mask: &[bool],
) -> Result<ResourceUsage, PlanError> {
    let mut usage = ResourceUsage::default();
    for (node, included) in hierarchy.nodes.iter().zip(resource_mask) {
        if !included {
            continue;
        }
        add_node_usage(&mut usage, node)?;
    }
    Ok(usage)
}

fn add_node_usage(usage: &mut ResourceUsage, node: &AvailableNode) -> Result<(), PlanError> {
    usage.point_count = usage
        .point_count
        .checked_add(node.point_count)
        .ok_or(PlanError::ResourceUsageOverflow)?;
    usage.estimated_bytes = usage
        .estimated_bytes
        .checked_add(node.estimated_bytes)
        .ok_or(PlanError::ResourceUsageOverflow)?;
    usage.batch_count = usage
        .batch_count
        .checked_add(1)
        .ok_or(PlanError::ResourceUsageOverflow)?;
    Ok(())
}

fn retained_nodes(
    hierarchy: &ProjectedHierarchy,
    retained_mask: &[bool],
    view_generation: render_protocol::ViewGenerationKey,
) -> Vec<RetainedNode> {
    hierarchy
        .nodes
        .iter()
        .zip(retained_mask)
        .filter_map(|(node, retained)| {
            if !retained {
                return None;
            }
            let NodeStatus::Resident { version } = node.status else {
                return None;
            };
            Some(RetainedNode {
                view_generation,
                node_key: node.key,
                batch_key: node.batch_key,
                version,
            })
        })
        .collect()
}

fn retirements(
    hierarchy: &ProjectedHierarchy,
    retained_mask: &[bool],
    view_generation: render_protocol::ViewGenerationKey,
) -> Vec<Retirement> {
    let mut retirements = hierarchy
        .nodes
        .iter()
        .zip(retained_mask)
        .filter_map(|(node, retained)| {
            let NodeStatus::Resident { version } = node.status else {
                return None;
            };
            (!retained).then_some(Retirement {
                view_generation,
                batch_key: node.batch_key,
                expected_version: version,
            })
        })
        .collect::<Vec<_>>();
    retirements.sort_by_key(|retirement| retirement.batch_key);
    retirements
}
