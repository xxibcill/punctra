use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

use glam::DVec3;
use render_protocol::Camera;

use crate::{
    AvailableNode, AvailableNodes, AxisAlignedBox, NodeKey, NodeRequest, NodeStatus, PlanError,
    PlanningBudget, ResourceUsage, RetainedNode, Retirement, ViewPlan, ViewPlanner,
};

pub(super) fn plan(
    planner: &mut ViewPlanner,
    camera: &Camera,
    viewport: [u32; 2],
    available_nodes: AvailableNodes<'_>,
    budget: PlanningBudget,
) -> Result<ViewPlan, PlanError> {
    if viewport.contains(&0) {
        return Err(PlanError::InvalidViewport);
    }

    let hierarchy = Hierarchy::new(available_nodes.nodes)?;
    let projection = Projection::new(camera, viewport);
    let visibility = hierarchy
        .nodes
        .iter()
        .map(|node| projection.node_projection(node))
        .collect::<Vec<_>>();
    let previous_refinements = if planner.active_generation == Some(available_nodes.view_generation)
    {
        &planner.refined_nodes
    } else {
        &BTreeSet::new()
    };

    let (target_cut, next_refinements) = select_target_cut(
        &hierarchy,
        &visibility,
        previous_refinements,
        planner.config,
        budget,
    )?;
    let retained_mask = required_residents(&hierarchy, &visibility, &target_cut);
    let requests = select_requests(
        &hierarchy,
        &visibility,
        &target_cut,
        &retained_mask,
        budget,
        available_nodes.view_generation,
    )?;
    let resource_usage = actual_resource_usage(&hierarchy, &retained_mask, &requests)?;
    let retained = retained_nodes(&hierarchy, &retained_mask, available_nodes.view_generation);
    let retirements = retirements(&hierarchy, &retained_mask, available_nodes.view_generation);

    planner.active_generation = Some(available_nodes.view_generation);
    planner.refined_nodes = next_refinements;

    Ok(ViewPlan {
        view_generation: available_nodes.view_generation,
        requests,
        retained,
        retirements,
        resource_usage,
    })
}

#[derive(Debug)]
struct Hierarchy {
    nodes: Vec<AvailableNode>,
    parents: Vec<Option<usize>>,
    children: Vec<Vec<usize>>,
    roots: Vec<usize>,
}

impl Hierarchy {
    fn new(input_nodes: &[AvailableNode]) -> Result<Self, PlanError> {
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

        Ok(Self {
            nodes,
            parents,
            children,
            roots,
        })
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
            if let Some(cycle_start) = path.iter().position(|candidate| *candidate == index) {
                let key = path[cycle_start..]
                    .iter()
                    .map(|cycle_index| nodes[*cycle_index].key)
                    .min()
                    .expect("a repeated path index creates a non-empty cycle");
                return Err(PlanError::ParentCycle { key });
            }
            path.push(index);
            cursor = parents[index];
        }
        for index in path {
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
struct Projection {
    eye: DVec3,
    forward: DVec3,
    pixel_scale: f64,
    near_distance: f64,
    frustum: Frustum,
}

impl Projection {
    fn new(camera: &Camera, viewport: [u32; 2]) -> Self {
        let eye = DVec3::from_array(camera.eye());
        let world_basis = camera.world_basis();
        let forward = DVec3::from_array(world_basis.forward());
        let right = DVec3::from_array(world_basis.right());
        let up = DVec3::from_array(world_basis.up());
        let half_vertical_tangent =
            (f64::from(camera.vertical_field_of_view_radians()) * 0.5).tan();
        let aspect_ratio = f64::from(viewport[0]) / f64::from(viewport[1]);
        let near_distance = f64::from(camera.near_distance());
        let far_distance = f64::from(camera.far_distance());
        let pixel_scale = f64::from(viewport[1]) / (2.0 * half_vertical_tangent);
        let frustum = Frustum::new(
            eye,
            forward,
            right,
            up,
            half_vertical_tangent,
            aspect_ratio,
            near_distance,
            far_distance,
        );
        Self {
            eye,
            forward,
            pixel_scale,
            near_distance,
            frustum,
        }
    }

    fn node_projection(self, node: &AvailableNode) -> NodeProjection {
        NodeProjection {
            visible: self.frustum.intersects(node.bounds),
            screen_error: self.screen_error(node),
        }
    }

    fn screen_error(self, node: &AvailableNode) -> f64 {
        let min = DVec3::from_array(node.bounds.min);
        let max = DVec3::from_array(node.bounds.max);
        let center = (min + max) * 0.5;
        let half_extent = (max - min) * 0.5;
        let center_depth = self.forward.dot(center - self.eye);
        let depth_radius = self.forward.abs().dot(half_extent);
        let nearest_depth = (center_depth - depth_radius).max(self.near_distance);
        node.geometric_error * self.pixel_scale / nearest_depth
    }
}

#[derive(Clone, Copy, Debug)]
struct Plane {
    normal: DVec3,
    camera_origin: DVec3,
    offset: f64,
}

impl Plane {
    fn from_camera(normal: DVec3, camera_origin: DVec3, offset: f64) -> Self {
        Self {
            normal,
            camera_origin,
            offset,
        }
    }

    fn excludes(self, bounds: AxisAlignedBox) -> bool {
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
        self.normal.dot(support - self.camera_origin) + self.offset < 0.0
    }
}

#[derive(Clone, Copy, Debug)]
struct Frustum {
    planes: [Plane; 6],
}

impl Frustum {
    #[allow(clippy::too_many_arguments)]
    fn new(
        eye: DVec3,
        forward: DVec3,
        right: DVec3,
        up: DVec3,
        half_vertical_tangent: f64,
        aspect_ratio: f64,
        near_distance: f64,
        far_distance: f64,
    ) -> Self {
        let half_horizontal_tangent = half_vertical_tangent * aspect_ratio;
        Self {
            planes: [
                Plane {
                    normal: forward,
                    camera_origin: eye,
                    offset: -near_distance,
                },
                Plane {
                    normal: -forward,
                    camera_origin: eye,
                    offset: far_distance,
                },
                Plane::from_camera(forward * half_horizontal_tangent + right, eye, 0.0),
                Plane::from_camera(forward * half_horizontal_tangent - right, eye, 0.0),
                Plane::from_camera(forward * half_vertical_tangent + up, eye, 0.0),
                Plane::from_camera(forward * half_vertical_tangent - up, eye, 0.0),
            ],
        }
    }

    fn intersects(self, bounds: AxisAlignedBox) -> bool {
        self.planes.iter().all(|plane| !plane.excludes(bounds))
    }
}

#[derive(Clone, Copy, Debug)]
struct RefinementCandidate {
    index: usize,
    key: NodeKey,
    screen_error: f64,
}

impl PartialEq for RefinementCandidate {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
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
        self.screen_error
            .total_cmp(&other.screen_error)
            .then_with(|| other.key.cmp(&self.key))
    }
}

fn select_target_cut(
    hierarchy: &Hierarchy,
    visibility: &[NodeProjection],
    previous_refinements: &BTreeSet<NodeKey>,
    config: crate::PlannerConfig,
    budget: PlanningBudget,
) -> Result<(BTreeSet<usize>, BTreeSet<NodeKey>), PlanError> {
    let mut target_cut = hierarchy
        .roots
        .iter()
        .copied()
        .filter(|index| visibility[*index].visible)
        .collect::<BTreeSet<_>>();
    let mut candidates = BinaryHeap::new();
    for index in target_cut.iter().copied() {
        push_candidate(
            &mut candidates,
            hierarchy,
            visibility,
            previous_refinements,
            config,
            index,
        );
    }

    let mut refinements = hierarchy
        .nodes
        .iter()
        .enumerate()
        .filter(|(index, node)| {
            !visibility[*index].visible && previous_refinements.contains(&node.key)
        })
        .map(|(_, node)| node.key)
        .collect::<BTreeSet<_>>();
    while let Some(candidate) = candidates.pop() {
        if !target_cut.remove(&candidate.index) {
            continue;
        }
        let visible_children = hierarchy.children[candidate.index]
            .iter()
            .copied()
            .filter(|child| visibility[*child].visible)
            .collect::<Vec<_>>();
        target_cut.extend(visible_children.iter().copied());

        let usage = conservative_target_usage(hierarchy, visibility, &target_cut)?;
        if !usage.fits_within(budget) {
            for child in visible_children {
                target_cut.remove(&child);
            }
            target_cut.insert(candidate.index);
            continue;
        }

        refinements.insert(candidate.key);
        for child in visible_children {
            push_candidate(
                &mut candidates,
                hierarchy,
                visibility,
                previous_refinements,
                config,
                child,
            );
        }
    }
    Ok((target_cut, refinements))
}

fn push_candidate(
    candidates: &mut BinaryHeap<RefinementCandidate>,
    hierarchy: &Hierarchy,
    visibility: &[NodeProjection],
    previous_refinements: &BTreeSet<NodeKey>,
    config: crate::PlannerConfig,
    index: usize,
) {
    let node = &hierarchy.nodes[index];
    let has_coverage = matches!(node.status, NodeStatus::Resident { .. })
        || has_visible_resident_descendant(hierarchy, visibility, index);
    let has_visible_children = hierarchy.children[index]
        .iter()
        .any(|child| visibility[*child].visible);
    if has_coverage
        && has_visible_children
        && exceeds_refinement_threshold(
            visibility[index].screen_error,
            previous_refinements.contains(&node.key),
            config,
        )
    {
        candidates.push(RefinementCandidate {
            index,
            key: node.key,
            screen_error: visibility[index].screen_error,
        });
    }
}

fn has_visible_resident_descendant(
    hierarchy: &Hierarchy,
    visibility: &[NodeProjection],
    index: usize,
) -> bool {
    let mut pending = hierarchy.children[index].clone();
    while let Some(descendant) = pending.pop() {
        if !visibility[descendant].visible {
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

fn conservative_target_usage(
    hierarchy: &Hierarchy,
    visibility: &[NodeProjection],
    target_cut: &BTreeSet<usize>,
) -> Result<ResourceUsage, PlanError> {
    let mut resource_mask = required_residents(hierarchy, visibility, target_cut);
    for index in target_cut.iter().copied() {
        resource_mask[index] = true;
    }
    for (index, node) in hierarchy.nodes.iter().enumerate() {
        if node.status == NodeStatus::Requested {
            resource_mask[index] = true;
        }
    }
    usage_for_mask(hierarchy, &resource_mask)
}

fn required_residents(
    hierarchy: &Hierarchy,
    visibility: &[NodeProjection],
    target_cut: &BTreeSet<usize>,
) -> Vec<bool> {
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
        retain_resident_descendants(
            hierarchy,
            visibility,
            &unavailable_targets,
            root,
            false,
            &mut retained,
        );
    }
    retained
}

fn retain_resident_ancestors(hierarchy: &Hierarchy, index: usize, retained: &mut [bool]) {
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
    hierarchy: &Hierarchy,
    visibility: &[NodeProjection],
    unavailable_targets: &[bool],
    index: usize,
    below_unavailable_target: bool,
    retained: &mut [bool],
) {
    let mut pending = vec![(index, below_unavailable_target)];
    while let Some((descendant, below_unavailable_target)) = pending.pop() {
        let below_unavailable_target = below_unavailable_target || unavailable_targets[descendant];
        if below_unavailable_target
            && visibility[descendant].visible
            && matches!(
                hierarchy.nodes[descendant].status,
                NodeStatus::Resident { .. }
            )
        {
            retained[descendant] = true;
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
    hierarchy: &Hierarchy,
    visibility: &[NodeProjection],
    target_cut: &BTreeSet<usize>,
    retained_mask: &[bool],
    budget: PlanningBudget,
    view_generation: render_protocol::ViewGenerationKey,
) -> Result<Vec<NodeRequest>, PlanError> {
    let mut resource_mask = retained_mask.to_vec();
    for (index, node) in hierarchy.nodes.iter().enumerate() {
        if node.status == NodeStatus::Requested {
            resource_mask[index] = true;
        }
    }
    let current_usage = usage_for_mask(hierarchy, &resource_mask)?;
    if !current_usage.fits_within(budget) {
        return Ok(Vec::new());
    }

    let mut missing_targets = target_cut
        .iter()
        .copied()
        .filter(|index| hierarchy.nodes[*index].status == NodeStatus::Missing)
        .collect::<Vec<_>>();
    missing_targets.sort_by(|left, right| {
        visibility[*right]
            .screen_error
            .total_cmp(&visibility[*left].screen_error)
            .then_with(|| hierarchy.nodes[*left].key.cmp(&hierarchy.nodes[*right].key))
    });

    let mut requests = Vec::new();
    for index in missing_targets {
        resource_mask[index] = true;
        let proposed_usage = usage_for_mask(hierarchy, &resource_mask)?;
        if !proposed_usage.fits_within(budget) {
            resource_mask[index] = false;
            continue;
        }
        let node = &hierarchy.nodes[index];
        requests.push(NodeRequest {
            view_generation,
            node_key: node.key,
            batch_key: node.batch_key,
            point_count: node.point_count,
            estimated_bytes: node.estimated_bytes,
            screen_space_error_pixels: visibility[index].screen_error,
        });
    }
    Ok(requests)
}

fn actual_resource_usage(
    hierarchy: &Hierarchy,
    retained_mask: &[bool],
    requests: &[NodeRequest],
) -> Result<ResourceUsage, PlanError> {
    let request_keys = requests
        .iter()
        .map(|request| request.node_key)
        .collect::<BTreeSet<_>>();
    let resource_mask = hierarchy
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            retained_mask[index]
                || node.status == NodeStatus::Requested
                || request_keys.contains(&node.key)
        })
        .collect::<Vec<_>>();
    usage_for_mask(hierarchy, &resource_mask)
}

fn usage_for_mask(
    hierarchy: &Hierarchy,
    resource_mask: &[bool],
) -> Result<ResourceUsage, PlanError> {
    let mut usage = ResourceUsage::default();
    for (node, included) in hierarchy.nodes.iter().zip(resource_mask) {
        if !included {
            continue;
        }
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
    }
    Ok(usage)
}

fn retained_nodes(
    hierarchy: &Hierarchy,
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
    hierarchy: &Hierarchy,
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
