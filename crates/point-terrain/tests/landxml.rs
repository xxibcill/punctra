//! Independent semantic acceptance for the private metric-metre encoder.

mod support;

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use blake3::Hasher;
use point_contracts::ContentHash;
use point_terrain::{
    LandXmlDisposition, LandXmlLimits, LandXmlOptions, TerrainError, TerrainSurface,
};
use roxmltree::{Document, Node};

use support::{TerrainFixture, derive_surface};

const LANDXML_NAMESPACE: &str = "http://www.landxml.org/schema/LandXML-1.2";
const XML_SCHEMA_NAMESPACE: &str = "http://www.w3.org/2001/XMLSchema-instance";
const GROUND: u8 = 2;

static NEXT_OUTPUT: AtomicU64 = AtomicU64::new(1);

fn planar_surface(label: &str) -> (TerrainFixture, TerrainSurface) {
    let fixture = TerrainFixture::new(
        label,
        vec![[0, 0, 0], [10, 0, 10], [10, 10, 30], [0, 10, 20]],
        vec![GROUND; 4],
    );
    let surface = derive_surface(fixture.snapshot(), GROUND);
    (fixture, surface)
}

fn asserted_options(name: &str) -> LandXmlOptions {
    LandXmlOptions::metric_metres(name, "2026-08-10", "12:34:56Z")
        .expect("fixture LandXML options are valid")
        .allow_unknown_coordinate_reference_as_metric_metres()
}

#[test]
fn options_require_bounded_xml_text_and_explicit_valid_root_date_time() {
    let options = LandXmlOptions::metric_metres("Existing Ground", "2024-02-29", "00:00:00Z")
        .expect("leap-day options are valid");
    assert_eq!(options.surface_name(), "Existing Ground");
    assert_eq!(options.document_date(), "2024-02-29");
    assert_eq!(options.document_time(), "00:00:00Z");
    assert!(!options.allows_unknown_coordinate_reference());
    assert!(
        options
            .allow_unknown_coordinate_reference_as_metric_metres()
            .allows_unknown_coordinate_reference()
    );

    for (name, date, time) in [
        (" ", "2026-08-10", "00:00:00Z"),
        ("Ground\0", "2026-08-10", "00:00:00Z"),
        ("Ground", "2023-02-29", "00:00:00Z"),
        ("Ground", "0000-01-01", "00:00:00Z"),
        ("Ground", "2026-8-10", "00:00:00Z"),
        ("Ground", "2026-08-10", "24:00:00Z"),
        ("Ground", "2026-08-10", "00:00:60Z"),
        ("Ground", "2026-08-10", "00:00:00+00:00"),
    ] {
        assert!(LandXmlOptions::metric_metres(name, date, time).is_err());
    }
    assert!(LandXmlOptions::metric_metres("x".repeat(1_025), "2026-08-10", "00:00:00Z").is_err());
}

#[test]
fn unknown_coordinate_reference_requires_the_explicit_metric_metre_assertion() {
    let (_fixture, surface) = planar_surface("landxml-reference");
    assert!(surface.descriptor().coordinate_reference().is_unknown());
    let output = TemporaryOutput::new("reference");
    let target = output.path("terrain.xml");
    let options = LandXmlOptions::metric_metres("Ground", "2026-08-10", "12:34:56Z").unwrap();

    let error = surface
        .export_landxml(&target, options, LandXmlLimits::default())
        .blocking_wait()
        .expect_err("unknown reference is not guessed");

    assert!(matches!(error, TerrainError::InvalidArgument { .. }));
    assert!(!target.exists());
    output.assert_no_stages();
}

#[test]
fn deterministic_bytes_round_trip_through_an_independent_semantic_parser() {
    let (_fixture, surface) = planar_surface("landxml-semantic");
    let output = TemporaryOutput::new("semantic");
    let first = output.path("first.xml");
    let second = output.path("second.xml");
    let expected_name = "Existing & <Ground> \"A\"\nB\tC\rD";
    let options = asserted_options(expected_name);

    let first_receipt = surface
        .export_landxml(&first, options.clone(), LandXmlLimits::default())
        .blocking_wait()
        .expect("first LandXML export succeeds");
    let second_receipt = surface
        .export_landxml(&second, options, LandXmlLimits::default())
        .blocking_wait()
        .expect("repeated LandXML export succeeds at another create-new target");
    let first_bytes = fs::read(&first).expect("read first published XML");
    let second_bytes = fs::read(&second).expect("read second published XML");

    assert_eq!(first_bytes, second_bytes);
    assert_eq!(first_receipt, second_receipt);
    assert_eq!(first_receipt.disposition(), LandXmlDisposition::Created);
    assert_eq!(
        first_receipt.content_hash(),
        ContentHash::new(*blake3::hash(&first_bytes).as_bytes())
    );
    assert_eq!(first_receipt.byte_length(), first_bytes.len() as u64);
    assert_eq!(
        first_receipt.vertex_count(),
        surface.vertices().len() as u64
    );
    assert_eq!(first_receipt.face_count(), surface.faces().len() as u64);
    assert_eq!(
        first_receipt.surface_artifact_hash(),
        surface.descriptor().artifact_hash()
    );
    assert_eq!(
        first_receipt.geometry_hash(),
        surface.descriptor().geometry_hash()
    );
    assert_eq!(
        first_receipt.topology_hash(),
        surface.descriptor().topology_hash()
    );
    let xml = std::str::from_utf8(&first_bytes).expect("encoder emits UTF-8");
    let parsed = parse_landxml(xml);
    assert_eq!(parsed.surface_name, expected_name);
    assert_eq!(parsed.date, "2026-08-10");
    assert_eq!(parsed.time, "12:34:56Z");
    assert_eq!(parsed.linear_unit, "meter");
    assert_eq!(parsed.points.len(), surface.vertices().len());
    assert_eq!(parsed.faces.len(), surface.faces().len());
    assert_eq!(
        semantic_digest(&parsed.points, &parsed.faces),
        surface_digest(&surface)
    );
    output.assert_no_stages();
}

#[test]
fn create_new_never_replaces_an_existing_target() {
    let (_fixture, surface) = planar_surface("landxml-create-new");
    let output = TemporaryOutput::new("create-new");
    let target = output.path("terrain.xml");
    fs::write(&target, b"caller-owned sentinel").expect("create caller-owned target");

    let error = surface
        .export_landxml(
            &target,
            asserted_options("Ground"),
            LandXmlLimits::default(),
        )
        .blocking_wait()
        .expect_err("create-new export rejects an existing target");

    assert!(matches!(error, TerrainError::TargetExists { .. }));
    assert_eq!(
        fs::read(&target).expect("existing target remains readable"),
        b"caller-owned sentinel"
    );
    output.assert_no_stages();
}

#[test]
fn ensure_creates_then_reconciles_the_exact_existing_target() {
    let (_fixture, surface) = planar_surface("landxml-ensure");
    let output = TemporaryOutput::new("ensure");
    let target = output.path("terrain.xml");
    let options = asserted_options("Recovery Ground");

    let created = surface
        .ensure_landxml(&target, options.clone(), LandXmlLimits::default())
        .blocking_wait()
        .expect("ensure creates a missing target");
    let original = fs::read(&target).expect("created target is readable");
    let reconciled = surface
        .ensure_landxml(&target, options, LandXmlLimits::default())
        .blocking_wait()
        .expect("ensure reconciles exact existing bytes");

    assert_eq!(created.disposition(), LandXmlDisposition::Created);
    assert_eq!(
        reconciled.disposition(),
        LandXmlDisposition::ReconciledExisting
    );
    assert_eq!(created.content_hash(), reconciled.content_hash());
    assert_eq!(created.byte_length(), reconciled.byte_length());
    assert_eq!(
        fs::read(&target).expect("reconciled target remains readable"),
        original
    );
    output.assert_no_stages();
}

#[test]
fn ensure_reports_conflict_without_modifying_different_existing_bytes() {
    let (_fixture, surface) = planar_surface("landxml-conflict");
    let output = TemporaryOutput::new("conflict");
    let target = output.path("terrain.xml");
    let sentinel = b"caller-owned conflicting LandXML";
    fs::write(&target, sentinel).expect("create conflicting target");

    let error = surface
        .ensure_landxml(
            &target,
            asserted_options("Expected Ground"),
            LandXmlLimits::default(),
        )
        .blocking_wait()
        .expect_err("different existing bytes conflict");

    let TerrainError::ExportConflict {
        expected_hash,
        actual_hash,
        ..
    } = error
    else {
        panic!("different existing bytes must return a structured conflict");
    };
    assert_ne!(expected_hash, actual_hash);
    assert_eq!(
        actual_hash,
        ContentHash::new(*blake3::hash(sentinel).as_bytes())
    );
    assert_eq!(
        fs::read(&target).expect("target remains readable"),
        sentinel
    );
    output.assert_no_stages();
}

#[test]
fn ensure_rejects_non_regular_targets_without_modification() {
    let (_fixture, surface) = planar_surface("landxml-non-regular");
    let output = TemporaryOutput::new("non-regular");
    let target = output.path("terrain.xml");
    fs::create_dir(&target).expect("create conflicting directory target");

    let error = surface
        .ensure_landxml(
            &target,
            asserted_options("Expected Ground"),
            LandXmlLimits::default(),
        )
        .blocking_wait()
        .expect_err("directory target fails closed");

    assert!(matches!(error, TerrainError::InvalidArgument { .. }));
    assert!(target.is_dir());
    output.assert_no_stages();
}

#[cfg(unix)]
#[test]
fn ensure_rejects_a_symlink_without_reading_or_modifying_its_destination() {
    use std::os::unix::fs::symlink;

    let (_fixture, surface) = planar_surface("landxml-symlink");
    let output = TemporaryOutput::new("symlink");
    let destination = output.path("destination.xml");
    let target = output.path("terrain.xml");
    let sentinel = b"caller-owned symlink destination";
    fs::write(&destination, sentinel).expect("create symlink destination");
    symlink(&destination, &target).expect("create conflicting symlink target");

    let error = surface
        .ensure_landxml(
            &target,
            asserted_options("Expected Ground"),
            LandXmlLimits::default(),
        )
        .blocking_wait()
        .expect_err("symlink target fails closed");

    assert!(matches!(error, TerrainError::InvalidArgument { .. }));
    assert!(
        fs::symlink_metadata(&target)
            .expect("symlink remains")
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(&destination).unwrap(), sentinel);
    output.assert_no_stages();
}

#[test]
fn ensure_applies_resource_limits_before_reconciling_an_existing_target() {
    let (_fixture, surface) = planar_surface("landxml-ensure-limits");
    let output = TemporaryOutput::new("ensure-limits");
    let target = output.path("terrain.xml");
    let options = asserted_options("Bounded Recovery Ground");
    surface
        .ensure_landxml(&target, options.clone(), LandXmlLimits::default())
        .blocking_wait()
        .expect("seed ensure succeeds");
    let exact = fs::read(&target).expect("seed target is readable");

    let error = surface
        .ensure_landxml(
            &target,
            options,
            replace_byte_limits(LandXmlLimits::default(), Some(1), None, None, None, None),
        )
        .blocking_wait()
        .expect_err("output limit is not bypassed by an existing target");

    assert!(matches!(
        error,
        TerrainError::ResourceLimit {
            limit: "LandXML output bytes",
            ..
        }
    ));
    assert_eq!(fs::read(&target).unwrap(), exact);
    output.assert_no_stages();
}

#[test]
fn every_landxml_resource_family_fails_without_a_target_or_stage() {
    let (_fixture, surface) = planar_surface("landxml-limits");
    let output = TemporaryOutput::new("limits");
    let defaults = LandXmlLimits::default();
    let cases = [
        (
            "vertices.xml",
            LandXmlLimits::new(
                3,
                defaults.max_faces(),
                defaults.max_output_bytes(),
                defaults.max_staging_bytes(),
                defaults.max_write_buffer_bytes(),
                defaults.max_xml_token_bytes(),
                defaults.max_working_bytes(),
            ),
            "LandXML vertices",
        ),
        (
            "faces.xml",
            LandXmlLimits::new(
                defaults.max_vertices(),
                1,
                defaults.max_output_bytes(),
                defaults.max_staging_bytes(),
                defaults.max_write_buffer_bytes(),
                defaults.max_xml_token_bytes(),
                defaults.max_working_bytes(),
            ),
            "LandXML faces",
        ),
        (
            "output.xml",
            replace_byte_limits(defaults, Some(1), None, None, None, None),
            "LandXML output bytes",
        ),
        (
            "staging.xml",
            replace_byte_limits(defaults, None, Some(1), None, None, None),
            "LandXML staging bytes",
        ),
        (
            "buffer.xml",
            replace_byte_limits(defaults, None, None, Some(0), None, None),
            "LandXML write buffer bytes",
        ),
        (
            "token.xml",
            replace_byte_limits(defaults, None, None, None, Some(1), None),
            "LandXML XML token bytes",
        ),
        (
            "working.xml",
            replace_byte_limits(defaults, None, None, None, None, Some(0)),
            "LandXML working bytes",
        ),
    ];

    for (name, limits, expected_limit) in cases {
        let target = output.path(name);
        let error = surface
            .export_landxml(&target, asserted_options("S"), limits)
            .blocking_wait()
            .expect_err("resource ceiling rejects a complete publication");
        assert!(matches!(
            error,
            TerrainError::ResourceLimit { limit, .. } if limit == expected_limit
        ));
        assert!(!target.exists());
        output.assert_no_stages();
    }
}

#[test]
fn cancellation_before_publication_leaves_no_target_or_stage() {
    let side = 72_i64;
    let ticks = (0..side)
        .flat_map(|y| (0..side).map(move |x| [x, y, x + 2 * y]))
        .collect::<Vec<_>>();
    let fixture = TerrainFixture::new("landxml-cancel", ticks.clone(), vec![GROUND; ticks.len()]);
    let surface = derive_surface(fixture.snapshot(), GROUND);
    let output = TemporaryOutput::new("cancel");
    let target = output.path("cancelled.xml");
    let job = surface.export_landxml(
        &target,
        asserted_options("Cancellation Fixture"),
        LandXmlLimits::default(),
    );
    job.handle().cancel();

    assert!(matches!(job.blocking_wait(), Err(TerrainError::Cancelled)));
    assert!(!target.exists());
    output.assert_no_stages();
}

#[test]
fn ensure_cancellation_before_publication_leaves_no_target_or_stage() {
    let side = 72_i64;
    let ticks = (0..side)
        .flat_map(|y| (0..side).map(move |x| [x, y, x + 2 * y]))
        .collect::<Vec<_>>();
    let fixture = TerrainFixture::new(
        "landxml-ensure-cancel",
        ticks.clone(),
        vec![GROUND; ticks.len()],
    );
    let surface = derive_surface(fixture.snapshot(), GROUND);
    let output = TemporaryOutput::new("ensure-cancel");
    let target = output.path("cancelled.xml");
    let job = surface.ensure_landxml(
        &target,
        asserted_options("Ensure Cancellation Fixture"),
        LandXmlLimits::default(),
    );
    job.handle().cancel();

    assert!(matches!(job.blocking_wait(), Err(TerrainError::Cancelled)));
    assert!(!target.exists());
    output.assert_no_stages();
}

fn replace_byte_limits(
    defaults: LandXmlLimits,
    output: Option<u64>,
    staging: Option<u64>,
    write_buffer: Option<u64>,
    token: Option<u64>,
    working: Option<u64>,
) -> LandXmlLimits {
    LandXmlLimits::new(
        defaults.max_vertices(),
        defaults.max_faces(),
        output.unwrap_or(defaults.max_output_bytes()),
        staging.unwrap_or(defaults.max_staging_bytes()),
        write_buffer.unwrap_or(defaults.max_write_buffer_bytes()),
        token.unwrap_or(defaults.max_xml_token_bytes()),
        working.unwrap_or(defaults.max_working_bytes()),
    )
}

struct ParsedSurface {
    surface_name: String,
    date: String,
    time: String,
    linear_unit: String,
    points: Vec<(u32, [f64; 3])>,
    faces: Vec<[u32; 3]>,
}

fn parse_landxml(xml: &str) -> ParsedSurface {
    let document = Document::parse(xml).expect("independent XML parse succeeds");
    let root = document.root_element();
    assert_eq!(root.tag_name().name(), "LandXML");
    assert_eq!(root.tag_name().namespace(), Some(LANDXML_NAMESPACE));
    assert_eq!(root.attribute("version"), Some("1.2"));
    let date = required_attribute(root, "date").to_owned();
    let time = required_attribute(root, "time").to_owned();
    assert_eq!(
        root.attribute((XML_SCHEMA_NAMESPACE, "schemaLocation")),
        Some(
            "http://www.landxml.org/schema/LandXML-1.2 \
             http://www.landxml.org/schema/LandXML-1.2/LandXML-1.2.xsd"
        )
    );
    assert_eq!(element_names(root), ["Units", "Surfaces"]);

    let units = only_child(root, "Units");
    assert_eq!(element_names(units), ["Metric"]);
    let metric = only_child(units, "Metric");
    assert_eq!(required_attribute(metric, "linearUnit"), "meter");
    assert_eq!(required_attribute(metric, "areaUnit"), "squareMeter");
    assert_eq!(required_attribute(metric, "volumeUnit"), "cubicMeter");

    let surfaces = only_child(root, "Surfaces");
    assert_eq!(element_names(surfaces), ["Surface"]);
    let surface = only_child(surfaces, "Surface");
    let surface_name = required_attribute(surface, "name").to_owned();
    assert_eq!(element_names(surface), ["Definition"]);
    let definition = only_child(surface, "Definition");
    assert_eq!(required_attribute(definition, "surfType"), "TIN");
    assert_eq!(element_names(definition), ["Pnts", "Faces"]);

    let point_nodes = element_children(only_child(definition, "Pnts"));
    assert!(
        point_nodes
            .iter()
            .all(|node| node.has_tag_name((LANDXML_NAMESPACE, "P")))
    );
    let points = point_nodes
        .into_iter()
        .enumerate()
        .map(|(index, node)| parse_point(index, node))
        .collect::<Vec<_>>();
    let face_nodes = element_children(only_child(definition, "Faces"));
    assert!(
        face_nodes
            .iter()
            .all(|node| node.has_tag_name((LANDXML_NAMESPACE, "F")))
    );
    let faces = face_nodes.into_iter().map(parse_face).collect::<Vec<_>>();
    for face in &faces {
        assert!(
            face.iter()
                .all(|id| { usize::try_from(*id).is_ok_and(|id| id >= 1 && id <= points.len()) })
        );
    }

    ParsedSurface {
        surface_name,
        date,
        time,
        linear_unit: required_attribute(metric, "linearUnit").to_owned(),
        points,
        faces,
    }
}

fn parse_point(index: usize, node: Node<'_, '_>) -> (u32, [f64; 3]) {
    let id = required_attribute(node, "id")
        .parse::<u32>()
        .expect("point identity is an unsigned integer");
    assert_eq!(
        usize::try_from(id).expect("point identity fits usize"),
        index + 1
    );
    let coordinates = node
        .text()
        .expect("point has coordinate text")
        .split_whitespace()
        .map(|value| value.parse::<f64>().expect("coordinate is numeric"))
        .collect::<Vec<_>>();
    let [northing, easting, elevation] = coordinates.as_slice() else {
        panic!("point must contain exactly Y X Z");
    };
    assert!(
        [northing, easting, elevation]
            .iter()
            .all(|coordinate| coordinate.is_finite())
    );
    (id, [*easting, *northing, *elevation])
}

fn parse_face(node: Node<'_, '_>) -> [u32; 3] {
    let identities = node
        .text()
        .expect("face has identity text")
        .split_whitespace()
        .map(|value| value.parse::<u32>().expect("face identity is numeric"))
        .collect::<Vec<_>>();
    let [a, b, c] = identities.as_slice() else {
        panic!("face must contain exactly three vertex identities");
    };
    [*a, *b, *c]
}

fn surface_digest(surface: &TerrainSurface) -> blake3::Hash {
    let transform = surface.descriptor().position_transform();
    let points = surface
        .vertices()
        .iter()
        .map(|vertex| {
            let mut world = transform.world_f64(vertex.ticks());
            for value in &mut world {
                *value = canonical_zero(*value);
            }
            (vertex.id().get(), world)
        })
        .collect::<Vec<_>>();
    let faces = surface
        .faces()
        .iter()
        .map(|face| face.vertices().map(point_terrain::SurfaceVertexId::get))
        .collect::<Vec<_>>();
    semantic_digest(&points, &faces)
}

fn semantic_digest(points: &[(u32, [f64; 3])], faces: &[[u32; 3]]) -> blake3::Hash {
    let mut hasher = Hasher::new();
    hasher.update(b"punctra-landxml-independent-semantics-v1");
    hasher.update(&(points.len() as u64).to_le_bytes());
    for (id, position) in points {
        hasher.update(&id.to_le_bytes());
        for coordinate in position {
            hasher.update(&canonical_zero(*coordinate).to_bits().to_le_bytes());
        }
    }
    hasher.update(&(faces.len() as u64).to_le_bytes());
    for face in faces {
        for id in face {
            hasher.update(&id.to_le_bytes());
        }
    }
    hasher.finalize()
}

fn element_children<'a, 'input>(node: Node<'a, 'input>) -> Vec<Node<'a, 'input>> {
    node.children().filter(Node::is_element).collect()
}

fn element_names<'input>(node: Node<'_, 'input>) -> Vec<&'input str> {
    element_children(node)
        .into_iter()
        .map(|child| child.tag_name().name())
        .collect()
}

fn only_child<'a, 'input>(parent: Node<'a, 'input>, name: &str) -> Node<'a, 'input> {
    let mut matches = parent
        .children()
        .filter(Node::is_element)
        .filter(|node| node.has_tag_name((LANDXML_NAMESPACE, name)));
    let child = matches.next().expect("required LandXML child exists");
    assert!(matches.next().is_none(), "LandXML child must be unique");
    child
}

fn required_attribute<'a>(node: Node<'a, '_>, name: &str) -> &'a str {
    node.attribute(name)
        .expect("required LandXML attribute exists")
}

fn canonical_zero(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

struct TemporaryOutput {
    directory: PathBuf,
}

impl TemporaryOutput {
    fn new(label: &str) -> Self {
        let sequence = NEXT_OUTPUT.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "punctra-landxml-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&directory).expect("create isolated LandXML output directory");
        Self { directory }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.directory.join(name)
    }

    fn assert_no_stages(&self) {
        let stages = fs::read_dir(&self.directory)
            .expect("read output directory")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .filter(|name| name.to_string_lossy().starts_with(".punctra-landxml-"))
            .collect::<Vec<_>>();
        assert!(stages.is_empty(), "staging files remain: {stages:?}");
    }
}

impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
