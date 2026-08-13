//! Process-level evidence for the thin start/resume/inspect host.

mod support;

use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};

use point_contracts::AttributeId;
use point_index::PrepareLimits;
use point_workspace::{OpenLimits, WorkspaceSchema, create};
use roxmltree::Document;
use support::{
    TestDirectory, journal_frame_ends, overwrite_and_sync, restore_journal_prefix,
    write_las_family_fixture,
};

const RUN_ID: [u8; 16] = [0x31; 16];
const OPERATION_ID: [u8; 16] = [0x32; 16];
const CLASSIFICATION_ATTRIBUTE: u32 = 6;
const LANDXML_NAMESPACE: &str = "http://www.landxml.org/schema/LandXML-1.2";

#[test]
fn thin_process_starts_resumes_and_inspects_one_durable_run() {
    let fixture = ProcessFixture::new("commands");
    let source_bytes = fs::read(&fixture.source).expect("read immutable process Source");

    let started = fixture.run("start");
    assert_success(&started);
    assert_complete_summary(&started);
    let report = fs::read(fixture.run_root.join("audit.json")).expect("read process audit report");
    let landxml = fs::read(fixture.run_root.join("terrain.xml")).expect("read process LandXML");
    assert_landxml(&landxml);
    let returned = fixture.directory.path().join("returned.xml");
    fs::write(&returned, &landxml).expect("write distinct returned LandXML fixture");
    let compared = Command::new(env!("CARGO_BIN_EXE_terrain-demo"))
        .arg("compare-landxml")
        .args(["--application", "generated-fixture"])
        .args(["--application-version", "test-only"])
        .args(["--settings-profile", "metric-tin"])
        .args(["--horizontal-tolerance-metres", "1e-18"])
        .args(["--vertical-tolerance-metres", "0"])
        .arg(fixture.run_root.join("terrain.xml"))
        .arg(&returned)
        .output()
        .expect("compare process LandXML");
    assert_success(&compared);
    let comparison = String::from_utf8_lossy(&compared.stdout);
    for expected in [
        "LandXML semantic comparison passed",
        "caller-declared application generated-fixture",
        "horizontal tolerance metres 1e-18",
        "exact bytes true",
        "topology matches true",
        "run bound false",
        "canonical evidence published false",
        "external application execution verified false",
    ] {
        assert!(
            comparison.contains(expected),
            "missing {expected:?}\n{comparison}"
        );
    }

    let inspected = Command::new(env!("CARGO_BIN_EXE_terrain-demo"))
        .arg("inspect")
        .arg(&fixture.run_root)
        .output()
        .expect("inspect workflow process");
    assert_success(&inspected);
    let inspection = String::from_utf8_lossy(&inspected.stdout);
    assert!(inspection.contains("phase complete"), "{inspection}");
    assert!(inspection.contains("complete true"), "{inspection}");

    let resumed = fixture.run("resume");
    assert_success(&resumed);
    assert_complete_summary(&resumed);
    assert_eq!(
        fs::read(fixture.run_root.join("audit.json")).unwrap(),
        report
    );
    assert_eq!(
        fs::read(fixture.run_root.join("terrain.xml")).unwrap(),
        landxml
    );
    assert_eq!(fs::read(&fixture.source).unwrap(), source_bytes);
}

#[test]
fn verify_round_trip_publishes_canonical_run_bound_evidence_and_reconciles_exact() {
    let fixture = ProcessFixture::new("run-bound-evidence");
    assert_success(&fixture.run("start"));
    let returned = fixture.directory.path().join("returned.xml");
    fs::copy(fixture.run_root.join("terrain.xml"), &returned)
        .expect("copy generated returned LandXML");
    let evidence = fixture.directory.path().join("round-trip-evidence.json");
    let before_run = run_root_snapshot(&fixture.run_root);

    let first = fixture.verify_round_trip(&returned, &evidence);
    assert_success(&first);
    let summary = String::from_utf8_lossy(&first.stdout);
    for expected in [
        "LandXML round-trip evidence published",
        "result passed",
        "run bound true",
        "canonical evidence published true",
        "external application execution verified false",
    ] {
        assert!(
            summary.contains(expected),
            "missing {expected:?}\n{summary}"
        );
    }
    let evidence_bytes = fs::read(&evidence).expect("read canonical evidence");
    let value: serde_json::Value =
        serde_json::from_slice(&evidence_bytes).expect("evidence is canonical JSON");
    assert_eq!(
        value["schema"],
        "punctra.terrain-demo.landxml-round-trip-evidence.v1"
    );
    assert_eq!(value["result"], "passed");
    assert_eq!(value["run"]["run_identity"], hex(&RUN_ID));
    assert_eq!(value["checks"]["topology"]["status"], "passed");
    assert_eq!(value["limits"]["file_bytes_per_input"], 4_294_967_296_u64);
    assert_eq!(value["limits"]["points_per_surface"], 10_000_000_u64);
    assert_eq!(value["limits"]["faces_per_surface"], 20_000_000_u64);
    assert_eq!(value["limits"]["xml_token_bytes_per_input"], 4_096_u64);
    assert_eq!(
        value["limits"]["parser_working_bytes_per_input"],
        8_388_608_u64
    );
    assert_eq!(
        value["limits"]["retained_working_bytes_total"],
        4_294_967_296_u64
    );
    for (peak, ceiling) in [
        (
            "accounted_reference_parser_peak_bytes",
            "parser_working_bytes_per_input",
        ),
        (
            "accounted_returned_parser_peak_bytes",
            "parser_working_bytes_per_input",
        ),
        (
            "accounted_retained_peak_bytes",
            "retained_working_bytes_total",
        ),
    ] {
        assert!(
            matches!(
                (value["limits"][peak].as_u64(), value["limits"][ceiling].as_u64()),
                (Some(bytes), Some(allowed)) if bytes > 0 && bytes <= allowed
            ),
            "accounted peak {peak} is absent, zero, or over {ceiling}"
        );
    }
    assert_eq!(
        value["nonclaims"]["punctra_observed_downstream_execution"],
        false
    );
    assert_eq!(run_root_snapshot(&fixture.run_root), before_run);

    let second = fixture.verify_round_trip(&returned, &evidence);
    assert_success(&second);
    assert_eq!(fs::read(&evidence).unwrap(), evidence_bytes);
    assert_eq!(run_root_snapshot(&fixture.run_root), before_run);

    overwrite_and_sync(&evidence, b"caller-owned evidence conflict\n")
        .expect("install caller-owned evidence conflict");
    let conflict = fixture.verify_round_trip(&returned, &evidence);
    assert!(
        !conflict.status.success(),
        "different evidence target must fail"
    );
    let diagnostic = String::from_utf8_lossy(&conflict.stderr);
    assert!(diagnostic.contains("PWF_OUTPUT_CONFLICT"), "{diagnostic}");
    assert_eq!(
        fs::read(&evidence).unwrap(),
        b"caller-owned evidence conflict\n"
    );
    assert_eq!(run_root_snapshot(&fixture.run_root), before_run);
}

#[test]
fn verify_round_trip_publishes_canonical_failed_evidence_only_for_semantic_mismatch() {
    let fixture = ProcessFixture::new("run-bound-failed-evidence");
    assert_success(&fixture.run("start"));
    let returned = fixture.directory.path().join("returned.xml");
    let original = fs::read_to_string(fixture.run_root.join("terrain.xml")).unwrap();
    let returned_xml = original.replacen("linearUnit=\"meter\"", "linearUnit=\"foot\"", 1);
    overwrite_and_sync(&returned, returned_xml.as_bytes()).unwrap();
    let evidence = fixture.directory.path().join("failed-evidence.json");
    let before_run = run_root_snapshot(&fixture.run_root);

    let output = fixture.verify_round_trip(&returned, &evidence);

    assert!(
        !output.status.success(),
        "semantic mismatch must exit nonzero"
    );
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(diagnostic.contains("PRT_SEMANTIC_MISMATCH"), "{diagnostic}");
    assert!(diagnostic.contains("PRT_UNIT_DRIFT"), "{diagnostic}");
    assert!(
        diagnostic.contains("canonical failed evidence published"),
        "{diagnostic}"
    );
    let value: serde_json::Value = serde_json::from_slice(&fs::read(&evidence).unwrap()).unwrap();
    assert_eq!(value["result"], "failed");
    assert_eq!(value["checks"]["units"]["reason"], "PRT_UNIT_DRIFT");
    assert_eq!(value["checks"]["topology"]["status"], "not_evaluated");
    assert_eq!(run_root_snapshot(&fixture.run_root), before_run);

    let topology_returned = fixture.directory.path().join("topology-returned.xml");
    let first_face = original.find("<F>").expect("generated terrain has a face");
    let face_end = original[first_face..]
        .find("</F>")
        .map(|offset| first_face + offset + "</F>".len())
        .expect("generated terrain face is complete");
    let topology_xml = format!("{}{}", &original[..first_face], &original[face_end..]);
    overwrite_and_sync(&topology_returned, topology_xml.as_bytes()).unwrap();
    let topology_evidence = fixture
        .directory
        .path()
        .join("topology-failed-evidence.json");

    let topology_output = fixture.verify_round_trip(&topology_returned, &topology_evidence);

    assert!(!topology_output.status.success());
    let topology_value: serde_json::Value =
        serde_json::from_slice(&fs::read(&topology_evidence).unwrap()).unwrap();
    assert_eq!(topology_value["checks"]["topology"]["status"], "failed");
    assert_eq!(
        topology_value["checks"]["topology"]["reason"],
        "PRT_TOPOLOGY_DRIFT"
    );
    assert_eq!(topology_value["comparison"]["added_face_count"], 0);
    assert_eq!(topology_value["comparison"]["removed_face_count"], 1);
    assert_eq!(
        topology_value["comparison"]["removed_face_sample"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(run_root_snapshot(&fixture.run_root), before_run);

    let malformed = fixture.directory.path().join("malformed.xml");
    overwrite_and_sync(&malformed, b"<LandXML").unwrap();
    let no_evidence = fixture.directory.path().join("operational-failure.json");
    let malformed_output = fixture.verify_round_trip(&malformed, &no_evidence);
    assert!(!malformed_output.status.success());
    assert!(!no_evidence.exists());
    assert_eq!(run_root_snapshot(&fixture.run_root), before_run);
}

#[test]
fn verify_round_trip_rejects_non_complete_or_changed_run_artifacts_without_evidence() {
    let fixture = ProcessFixture::new("run-bound-rejection");
    assert_success(&fixture.run("start"));
    let returned = fixture.directory.path().join("returned.xml");
    fs::copy(fixture.run_root.join("terrain.xml"), &returned).unwrap();
    let evidence = fixture.directory.path().join("must-not-exist.json");
    let journal_path = fixture.run_root.join("run.pwf");
    let complete = fs::read(&journal_path).unwrap();
    let frame_ends = journal_frame_ends(&complete).unwrap();
    restore_journal_prefix(&journal_path, &complete, frame_ends[6]).unwrap();

    let incomplete = fixture.verify_round_trip(&returned, &evidence);

    assert!(!incomplete.status.success());
    assert!(!evidence.exists());
    assert_eq!(fs::read(&journal_path).unwrap().len(), frame_ends[6]);

    overwrite_and_sync(&journal_path, &complete).unwrap();
    let landxml_path = fixture.run_root.join("terrain.xml");
    let expected_landxml = fs::read(&landxml_path).unwrap();
    overwrite_and_sync(&landxml_path, b"caller-owned changed LandXML").unwrap();

    let changed = fixture.verify_round_trip(&returned, &evidence);

    assert!(!changed.status.success());
    assert!(!evidence.exists());
    assert_eq!(
        fs::read(&landxml_path).unwrap(),
        b"caller-owned changed LandXML"
    );
    overwrite_and_sync(&landxml_path, &expected_landxml).unwrap();

    let inside_run = fixture.run_root.join("evidence.json");
    let invalid_target = fixture.verify_round_trip(&returned, &inside_run);
    assert!(!invalid_target.status.success());
    assert!(!inside_run.exists());
}

#[test]
#[allow(clippy::too_many_lines)]
fn verify_round_trip_process_covers_the_remaining_generated_semantic_matrix() {
    let fixture = ProcessFixture::new("run-bound-generated-matrix");
    assert_success(&fixture.run("start"));
    let original = fs::read_to_string(fixture.run_root.join("terrain.xml")).unwrap();
    let before_run = run_root_snapshot(&fixture.run_root);

    let presentation = fixture.directory.path().join("presentation-only.xml");
    let presentation_xml = original.replacen(
        "name=\"Punctra Ground Surface\"",
        "name=\"Downstream Presentation Name\"",
        1,
    );
    assert_ne!(presentation_xml, original);
    overwrite_and_sync(&presentation, presentation_xml.as_bytes()).unwrap();
    let presentation_evidence = fixture.directory.path().join("presentation-evidence.json");
    assert_success(&fixture.verify_round_trip(&presentation, &presentation_evidence));
    let presentation_value: serde_json::Value =
        serde_json::from_slice(&fs::read(&presentation_evidence).unwrap()).unwrap();
    assert_eq!(presentation_value["result"], "passed");
    assert_eq!(
        presentation_value["returned_landxml"]["surface_name"],
        "Downstream Presentation Name"
    );

    let tolerance = fixture.directory.path().join("tolerance-boundary.xml");
    let tolerance_xml = shift_first_horizontal_coordinate(&original, 0.125);
    overwrite_and_sync(&tolerance, tolerance_xml.as_bytes()).unwrap();
    let tolerance_evidence = fixture.directory.path().join("tolerance-evidence.json");
    assert_success(&fixture.verify_round_trip_with_tolerances(
        &tolerance,
        &tolerance_evidence,
        "0.125",
        "0",
    ));
    let tolerance_value: serde_json::Value =
        serde_json::from_slice(&fs::read(&tolerance_evidence).unwrap()).unwrap();
    assert_eq!(tolerance_value["result"], "passed");
    assert_eq!(
        tolerance_value["comparison"]["maximum_horizontal_delta_metres"],
        "0.12500000000000000"
    );

    let ambiguous = fixture.directory.path().join("ambiguous.xml");
    overwrite_and_sync(&ambiguous, original.as_bytes()).unwrap();
    let ambiguous_evidence = fixture.directory.path().join("ambiguous-evidence.json");
    let ambiguous_output = fixture.verify_round_trip_with_tolerances(
        &ambiguous,
        &ambiguous_evidence,
        "1000000000",
        "1000000000",
    );
    assert!(!ambiguous_output.status.success());
    let ambiguous_value: serde_json::Value =
        serde_json::from_slice(&fs::read(&ambiguous_evidence).unwrap()).unwrap();
    assert_eq!(ambiguous_value["result"], "failed");
    assert_eq!(
        ambiguous_value["checks"]["unique_mapping"]["reason"],
        "PRT_VERTEX_AMBIGUOUS"
    );

    let unsupported = fixture.directory.path().join("unsupported-subset.xml");
    let unsupported_xml = original.replacen("<Units>", "<Unsupported/><Units>", 1);
    assert_ne!(unsupported_xml, original);
    overwrite_and_sync(&unsupported, unsupported_xml.as_bytes()).unwrap();
    let unsupported_evidence = fixture.directory.path().join("unsupported-evidence.json");
    let unsupported_output = fixture.verify_round_trip(&unsupported, &unsupported_evidence);
    assert!(!unsupported_output.status.success());
    assert!(
        String::from_utf8_lossy(&unsupported_output.stderr).contains("PRT_INVALID_INPUT"),
        "{}",
        diagnostics(&unsupported_output)
    );
    assert!(!unsupported_evidence.exists());

    let over_token = fixture.directory.path().join("over-token-limit.xml");
    let over_token_xml = original.replacen(
        "name=\"Punctra Ground Surface\"",
        &format!("name=\"{}\"", "x".repeat(16 * 1024)),
        1,
    );
    assert_ne!(over_token_xml, original);
    overwrite_and_sync(&over_token, over_token_xml.as_bytes()).unwrap();
    let resource_evidence = fixture.directory.path().join("resource-evidence.json");
    let resource_output = fixture.verify_round_trip(&over_token, &resource_evidence);
    assert!(!resource_output.status.success());
    assert!(
        String::from_utf8_lossy(&resource_output.stderr).contains("PRT_RESOURCE_LIMIT"),
        "{}",
        diagnostics(&resource_output)
    );
    assert!(!resource_evidence.exists());

    let over_file = fixture.directory.path().join("over-file-limit.xml");
    let over_file_handle = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&over_file)
        .unwrap();
    over_file_handle.set_len(4_294_967_297).unwrap();
    over_file_handle.sync_all().unwrap();
    drop(over_file_handle);
    let file_resource_evidence = fixture.directory.path().join("file-resource-evidence.json");
    let file_resource_output = fixture.verify_round_trip(&over_file, &file_resource_evidence);
    assert!(!file_resource_output.status.success());
    assert!(
        String::from_utf8_lossy(&file_resource_output.stderr).contains("PRT_RESOURCE_LIMIT"),
        "{}",
        diagnostics(&file_resource_output)
    );
    assert!(!file_resource_evidence.exists());
    assert_eq!(run_root_snapshot(&fixture.run_root), before_run);
}

#[test]
fn process_report_conflict_is_bounded_structured_and_non_destructive() {
    let fixture = ProcessFixture::new("diagnostic");
    assert_success(&fixture.run("start"));
    let journal_path = fixture.run_root.join("run.pwf");
    let complete = fs::read(&journal_path).expect("read complete process journal");
    let ends = journal_frame_ends(&complete).expect("parse process journal frames");
    restore_journal_prefix(&journal_path, &complete, ends[5])
        .expect("restore process ExportEnsured prefix");
    let conflict = b"caller-owned report conflict";
    let report_path = fixture.run_root.join("audit.json");
    overwrite_and_sync(&report_path, conflict).expect("install process report conflict");

    let failed = fixture.run("resume");
    assert!(!failed.status.success(), "conflicting report must fail");
    let diagnostic = String::from_utf8_lossy(&failed.stderr);
    for expected in [
        "PWF_OUTPUT_CONFLICT at report-ensure",
        "certainty=durable-fact",
        &format!("run={}", hex(&RUN_ID)),
        &format!("operation={}", hex(&OPERATION_ID)),
        "revision=",
        "recovery: remove or rename the conflicting caller-owned target, then resume",
    ] {
        assert!(
            diagnostic.contains(expected),
            "missing {expected:?}\n{diagnostic}",
        );
    }
    assert!(diagnostic.len() < 2 * 1024, "diagnostic must be bounded");
    assert!(diagnostic.lines().count() < 8, "diagnostic must be concise");
    assert_eq!(fs::read(&report_path).unwrap(), conflict);
    assert_eq!(
        journal_frame_ends(&fs::read(&journal_path).unwrap())
            .expect("journal remains a valid prefix")
            .len(),
        6,
    );
}

#[test]
fn process_help_and_invalid_input_are_bounded_and_do_not_create_a_run() {
    let help = Command::new(env!("CARGO_BIN_EXE_terrain-demo"))
        .arg("--help")
        .output()
        .expect("run process help");
    assert_success(&help);
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(help.contains("terrain-demo start|resume"));
    assert!(help.contains("terrain-demo inspect"));
    assert!(help.contains("terrain-demo compare-landxml"));
    assert!(help.contains("terrain-demo verify-round-trip"));
    assert!(help.len() < 4 * 1024);

    let fixture = ProcessFixture::new("invalid");
    let invalid = Command::new(env!("CARGO_BIN_EXE_terrain-demo"))
        .args(["start", "--run-id", "not-hex"])
        .output()
        .expect("run invalid process request");
    assert!(!invalid.status.success());
    let diagnostic = String::from_utf8_lossy(&invalid.stderr);
    assert!(diagnostic.contains("PWF_INVALID_REQUEST at validate"));
    assert!(diagnostic.contains("Run ID must contain exactly 32 hexadecimal characters"));
    assert!(diagnostic.len() < 2 * 1024);
    assert!(!fixture.run_root.join("run.pwf").exists());

    let invalid_comparison = Command::new(env!("CARGO_BIN_EXE_terrain-demo"))
        .arg("compare-landxml")
        .output()
        .expect("run invalid comparison request");
    assert!(!invalid_comparison.status.success());
    let diagnostic = String::from_utf8_lossy(&invalid_comparison.stderr);
    assert!(
        diagnostic.contains("PRT_INVALID_INPUT at landxml-round-trip-comparison"),
        "{diagnostic}"
    );
    assert!(
        diagnostic.contains(
            "recovery: correct the declaration or LandXML input, then retry the comparison"
        ),
        "{diagnostic}"
    );
    assert!(!diagnostic.contains("start a new Run"), "{diagnostic}");
}

#[test]
fn round_trip_process_failures_preserve_public_mappings() {
    let directory = TestDirectory::new("round-trip-failures").expect("create comparison fixture");
    let reference = directory.path().join("reference.xml");
    let returned = directory.path().join("returned.xml");
    fs::write(&reference, comparison_landxml("0 0 0")).expect("write reference LandXML");
    fs::write(&returned, comparison_landxml("0 0 1")).expect("write returned LandXML");

    let resource_failure = comparison_process(&reference, &returned, &"x".repeat(129));
    assert_round_trip_failure(
        &resource_failure,
        "PRT_RESOURCE_LIMIT",
        "use inputs within the named comparison limit or preserve them for a later slice",
    );

    let semantic_failure = comparison_process(&reference, &returned, "generated-fixture");
    assert_round_trip_failure(
        &semantic_failure,
        "PRT_SEMANTIC_MISMATCH",
        "review the downstream export settings or reject the returned deliverable",
    );
}

fn comparison_process(reference: &PathBuf, returned: &PathBuf, application: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_terrain-demo"))
        .arg("compare-landxml")
        .args(["--application", application])
        .args(["--application-version", "test-only"])
        .args(["--settings-profile", "metric-tin"])
        .args(["--horizontal-tolerance-metres", "0"])
        .args(["--vertical-tolerance-metres", "0"])
        .arg(reference)
        .arg(returned)
        .output()
        .expect("run LandXML comparison")
}

fn assert_round_trip_failure(output: &Output, code: &str, recovery: &str) {
    assert!(!output.status.success(), "comparison must fail");
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    for expected in [
        &format!("{code} at landxml-round-trip-comparison"),
        "certainty=pre-publication",
        &format!("recovery: {recovery}"),
    ] {
        assert!(
            diagnostic.contains(expected),
            "missing {expected:?}\n{diagnostic}"
        );
    }
}

fn comparison_landxml(first_point: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<LandXML xmlns=\"{LANDXML_NAMESPACE}\" version=\"1.2\">\
<Units><Metric linearUnit=\"meter\"/></Units>\
<Surfaces><Surface name=\"Generated\"><Definition surfType=\"TIN\">\
<Pnts><P id=\"1\">{first_point}</P><P id=\"2\">0 1 0</P><P id=\"3\">1 0 0</P></Pnts>\
<Faces><F>1 2 3</F></Faces>\
</Definition></Surface></Surfaces></LandXML>"
    )
}

struct ProcessFixture {
    directory: TestDirectory,
    source: PathBuf,
    index: PathBuf,
    workspace: PathBuf,
    run_root: PathBuf,
    baseline: [u8; 32],
}

impl ProcessFixture {
    fn new(label: &str) -> Self {
        let directory = TestDirectory::new(label).expect("create process fixture directory");
        let source = directory.path().join("fixture.las");
        let index = directory.path().join("fixture.pidx");
        let workspace = directory.path().join("fixture.pcw");
        let run_root = directory.path().join("run");
        fs::create_dir(&run_root).expect("create process Run root");
        write_las_family_fixture(&source, 64).expect("write generated process Source");
        let source_handle = source_las::open(&source)
            .blocking_wait()
            .expect("open process Source");
        let prepared = point_index::prepare(source_handle, &index, PrepareLimits::default())
            .blocking_wait()
            .expect("prepare process index");
        let workspace_handle = create(
            &workspace,
            prepared,
            WorkspaceSchema::new(AttributeId::new(CLASSIFICATION_ATTRIBUTE).unwrap()),
            OpenLimits::default(),
        )
        .blocking_wait()
        .expect("create process Workspace");
        let baseline = workspace_handle.head().provenance().revision().into_bytes();
        drop(workspace_handle);
        Self {
            directory,
            source,
            index,
            workspace,
            run_root,
            baseline,
        }
    }

    fn run(&self, command: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_terrain-demo"))
            .arg(command)
            .args(["--run-id", &hex(&RUN_ID)])
            .args(["--operation-id", &hex(&OPERATION_ID)])
            .args(["--baseline", &hex(&self.baseline)])
            .args(["--exclude-ground-ordinal", "9"])
            .args(["--exclude-ground-ordinal", "10"])
            .args(["--check-point", "1,500002,4600002,121.6"])
            .args(["--check-point", "2,600000,4600000,120"])
            .args(["--date", "2026-08-10"])
            .args(["--time", "00:00:00Z"])
            .arg("--assert-unknown-crs-metric")
            .arg(&self.source)
            .arg(&self.index)
            .arg(&self.workspace)
            .arg(&self.run_root)
            .output()
            .expect("run terrain-demo workflow process")
    }

    fn verify_round_trip(&self, returned: &PathBuf, evidence: &PathBuf) -> Output {
        self.verify_round_trip_with_tolerances(returned, evidence, "0", "0")
    }

    fn verify_round_trip_with_tolerances(
        &self,
        returned: &PathBuf,
        evidence: &PathBuf,
        horizontal_tolerance: &str,
        vertical_tolerance: &str,
    ) -> Output {
        Command::new(env!("CARGO_BIN_EXE_terrain-demo"))
            .arg("verify-round-trip")
            .args(["--downstream-app", "generated-fixture"])
            .args(["--downstream-version", "test-only"])
            .args(["--downstream-setting", "profile=metric-tin"])
            .args(["--horizontal-tolerance-metres", horizontal_tolerance])
            .args(["--vertical-tolerance-metres", vertical_tolerance])
            .arg(&self.run_root)
            .arg(returned)
            .arg(evidence)
            .output()
            .expect("run Run-bound round-trip verifier")
    }
}

fn shift_first_horizontal_coordinate(landxml: &str, delta: f64) -> String {
    let point_start = landxml
        .find("<P id=\"1\">")
        .expect("generated LandXML contains Point 1");
    let value_start = landxml[point_start..]
        .find('>')
        .map(|offset| point_start + offset + 1)
        .unwrap();
    let value_end = landxml[value_start..]
        .find("</P>")
        .map(|offset| value_start + offset)
        .unwrap();
    let mut coordinates = landxml[value_start..value_end]
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(coordinates.len(), 3);
    coordinates[0] = (coordinates[0].parse::<f64>().unwrap() + delta).to_string();
    format!(
        "{}{}{}",
        &landxml[..value_start],
        coordinates.join(" "),
        &landxml[value_end..]
    )
}

fn run_root_snapshot(run_root: &std::path::Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut snapshot = fs::read_dir(run_root)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = fs::read(&path).unwrap_or_default();
            (path, bytes)
        })
        .collect::<Vec<_>>();
    snapshot.sort_by(|left, right| left.0.cmp(&right.0));
    snapshot
}

fn assert_success(output: &Output) {
    assert!(output.status.success(), "{}", diagnostics(output));
    assert!(output.stderr.is_empty(), "{}", diagnostics(output));
}

fn assert_complete_summary(output: &Output) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "Run complete",
        &format!("Run {}", hex(&RUN_ID)),
        &format!("Operation {}", hex(&OPERATION_ID)),
        "Revision ",
        "report hash ",
        "report bytes ",
    ] {
        assert!(stdout.contains(expected), "missing {expected:?}\n{stdout}");
    }
    assert!(stdout.len() < 2 * 1024, "process summary must be bounded");
    assert!(
        stdout.lines().count() < 16,
        "process summary must be concise"
    );
}

fn assert_landxml(bytes: &[u8]) {
    let text = std::str::from_utf8(bytes).expect("LandXML is UTF-8");
    let document = Document::parse(text).expect("independent parser accepts LandXML");
    let root = document.root_element();
    assert_eq!(root.tag_name().name(), "LandXML");
    assert_eq!(root.tag_name().namespace(), Some(LANDXML_NAMESPACE));
    assert_eq!(root.attribute("version"), Some("1.2"));
    assert_eq!(root.attribute("date"), Some("2026-08-10"));
    assert_eq!(root.attribute("time"), Some("00:00:00Z"));

    let metric = document
        .descendants()
        .find(|node| node.has_tag_name((LANDXML_NAMESPACE, "Metric")))
        .expect("metric Units declaration exists");
    assert_eq!(metric.attribute("linearUnit"), Some("meter"));
    let surface = document
        .descendants()
        .find(|node| node.has_tag_name((LANDXML_NAMESPACE, "Surface")))
        .expect("Surface exists");
    assert_eq!(surface.attribute("name"), Some("Punctra Ground Surface"));
    assert_eq!(
        document
            .descendants()
            .filter(|node| node.has_tag_name((LANDXML_NAMESPACE, "P")))
            .count(),
        62,
    );
    assert!(
        document
            .descendants()
            .any(|node| node.has_tag_name((LANDXML_NAMESPACE, "F"))),
        "LandXML must contain at least one face",
    );
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut value, "{byte:02x}").expect("String writes cannot fail");
    }
    value
}

fn diagnostics(output: &Output) -> String {
    format!(
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
}
