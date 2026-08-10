//! Independent public-surface topology and Delaunay evidence.

mod support;

use std::collections::{BTreeMap, BTreeSet};

use point_contracts::PointId;
use point_terrain::{SurfaceVertexId, TerrainSurface};
use robust::{Coord, incircle, orient2d};

use support::{TerrainFixture, derive_surface};

#[derive(Clone, Copy)]
struct EdgeUse {
    opposite: SurfaceVertexId,
}

#[test]
fn cocircular_square_uses_the_canonical_diagonal_and_faces() {
    let fixture = TerrainFixture::new(
        "cocircular-square",
        vec![[0, 0, 0], [10, 0, 1], [10, 10, 2], [0, 10, 3]],
        vec![2; 4],
    );
    let surface = derive_surface(fixture.snapshot(), 2);
    let faces = surface
        .faces()
        .iter()
        .map(|face| face.vertices().map(SurfaceVertexId::get))
        .collect::<Vec<_>>();

    assert_eq!(faces, vec![[1, 3, 4], [1, 4, 2]]);
    assert_eq!(surface.descriptor().hull_vertex_count(), 4);
    assert_topology_oracle(&surface, &fixture);
}

#[test]
fn every_ground_vertex_is_used_by_a_ccw_manifold_delaunay_disk() {
    let fixture = TerrainFixture::new(
        "topology-oracle",
        vec![
            [0, 0, 0],
            [12, 0, 1],
            [15, 7, 2],
            [11, 14, 3],
            [2, 15, 4],
            [-2, 8, 5],
            [3, 4, 6],
            [8, 3, 7],
            [10, 9, 8],
            [5, 11, 9],
            [6, 7, 10],
        ],
        vec![6; 11],
    );
    let surface = derive_surface(fixture.snapshot(), 6);

    assert_topology_oracle(&surface, &fixture);
}

fn assert_topology_oracle(surface: &TerrainSurface, fixture: &TerrainFixture) {
    assert_canonical_vertices(surface, fixture);
    let edges = assert_faces_and_collect_edges(surface);
    assert_manifold_disk(surface, &edges);
    assert_local_delaunay(surface, &edges);
}

fn assert_canonical_vertices(surface: &TerrainSurface, fixture: &TerrainFixture) {
    let mut expected = fixture
        .ticks()
        .iter()
        .copied()
        .enumerate()
        .map(|(ordinal, ticks)| {
            (
                ticks,
                fixture.point(u64::try_from(ordinal).expect("fixture ordinal fits u64")),
            )
        })
        .collect::<Vec<_>>();
    expected.sort();
    let actual = surface
        .vertices()
        .iter()
        .enumerate()
        .map(|(index, vertex)| {
            assert_eq!(
                vertex.id().get(),
                u32::try_from(index).unwrap().saturating_add(1)
            );
            (vertex.ticks(), vertex.point())
        })
        .collect::<Vec<([i64; 3], PointId)>>();
    assert_eq!(actual, expected);
}

fn assert_faces_and_collect_edges(surface: &TerrainSurface) -> BTreeMap<(u32, u32), Vec<EdgeUse>> {
    let mut edges = BTreeMap::<(u32, u32), Vec<EdgeUse>>::new();
    let mut referenced = BTreeSet::new();
    for (index, face) in surface.faces().iter().enumerate() {
        assert_eq!(
            face.id().get(),
            u32::try_from(index).unwrap().saturating_add(1)
        );
        let [first, second, third] = face.vertices();
        assert_eq!(first, first.min(second).min(third));
        assert!(exact_orientation(surface, first, second, third) > 0);
        for vertex in [first, second, third] {
            referenced.insert(vertex);
        }
        insert_edge(&mut edges, first, second, third);
        insert_edge(&mut edges, second, third, first);
        insert_edge(&mut edges, third, first, second);
    }
    assert_eq!(referenced.len(), surface.vertices().len());
    assert!(
        (1..=u32::try_from(surface.vertices().len()).unwrap())
            .all(|identity| referenced.iter().any(|vertex| vertex.get() == identity))
    );
    edges
}

fn insert_edge(
    edges: &mut BTreeMap<(u32, u32), Vec<EdgeUse>>,
    first: SurfaceVertexId,
    second: SurfaceVertexId,
    opposite: SurfaceVertexId,
) {
    let key = canonical_edge(first.get(), second.get());
    edges.entry(key).or_default().push(EdgeUse { opposite });
}

fn assert_manifold_disk(surface: &TerrainSurface, edges: &BTreeMap<(u32, u32), Vec<EdgeUse>>) {
    assert!(edges.values().all(|uses| matches!(uses.len(), 1 | 2)));
    let boundary = edges
        .iter()
        .filter_map(|(&edge, uses)| (uses.len() == 1).then_some(edge))
        .collect::<Vec<_>>();
    assert_eq!(
        u64::try_from(boundary.len()).unwrap(),
        surface.descriptor().hull_vertex_count()
    );
    let vertices = i64::try_from(surface.vertices().len()).unwrap();
    let edge_count = i64::try_from(edges.len()).unwrap();
    let faces = i64::try_from(surface.faces().len()).unwrap();
    assert_eq!(vertices - edge_count + faces, 1);
    assert_eq!(
        faces,
        2 * vertices - 2 - i64::try_from(boundary.len()).unwrap()
    );

    let mut adjacency = BTreeMap::<u32, Vec<u32>>::new();
    for &(first, second) in &boundary {
        adjacency.entry(first).or_default().push(second);
        adjacency.entry(second).or_default().push(first);
    }
    assert!(adjacency.values().all(|neighbors| neighbors.len() == 2));
    let start = *adjacency.keys().next().expect("a terrain has a hull");
    let mut visited = BTreeSet::new();
    let mut previous = None;
    let mut current = start;
    loop {
        assert!(visited.insert(current), "boundary cycle repeated early");
        let neighbors = &adjacency[&current];
        let next = if Some(neighbors[0]) == previous {
            neighbors[1]
        } else {
            neighbors[0]
        };
        previous = Some(current);
        current = next;
        if current == start {
            break;
        }
    }
    assert_eq!(visited.len(), adjacency.len());
}

fn assert_local_delaunay(surface: &TerrainSurface, edges: &BTreeMap<(u32, u32), Vec<EdgeUse>>) {
    for (&(first, second), uses) in edges {
        if uses.len() != 2 {
            continue;
        }
        let mut a = coordinate(surface, first);
        let mut b = coordinate(surface, second);
        let c = coordinate(surface, uses[0].opposite.get());
        let d = coordinate(surface, uses[1].opposite.get());
        if orient2d(a, b, c) < 0.0 {
            std::mem::swap(&mut a, &mut b);
        }
        let circle = incircle(a, b, c, d);
        assert!(
            circle <= 0.0,
            "interior edge ({first}, {second}) violates local Delaunay"
        );
        if circle == 0.0 {
            assert!(
                canonical_edge(first, second)
                    < canonical_edge(uses[0].opposite.get(), uses[1].opposite.get()),
                "cocircular quadrilateral did not retain its canonical diagonal"
            );
        }
    }
}

fn exact_orientation(
    surface: &TerrainSurface,
    first: SurfaceVertexId,
    second: SurfaceVertexId,
    third: SurfaceVertexId,
) -> i128 {
    let [ax, ay, _] = ticks(surface, first);
    let [bx, by, _] = ticks(surface, second);
    let [cx, cy, _] = ticks(surface, third);
    (i128::from(bx) - i128::from(ax)) * (i128::from(cy) - i128::from(ay))
        - (i128::from(by) - i128::from(ay)) * (i128::from(cx) - i128::from(ax))
}

fn coordinate(surface: &TerrainSurface, identity: u32) -> Coord<f64> {
    let [x, y, _] = ticks(
        surface,
        surface.vertices()[usize::try_from(identity - 1).unwrap()].id(),
    );
    #[allow(clippy::cast_precision_loss)]
    Coord {
        x: x as f64,
        y: y as f64,
    }
}

fn ticks(surface: &TerrainSurface, identity: SurfaceVertexId) -> [i64; 3] {
    surface.vertices()[usize::try_from(identity.get() - 1).unwrap()].ticks()
}

fn canonical_edge(first: u32, second: u32) -> (u32, u32) {
    (first.min(second), first.max(second))
}
