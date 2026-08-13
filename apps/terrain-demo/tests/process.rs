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
fn verify_round_trip_binds_complete_run_and_reconciles_canonical_evidence() {
    let fixture = ProcessFixture::new("round-trip-evidence");
    assert_success(&fixture.run("start"));
    let journal_before = fs::read(fixture.run_root.join("run.pwf")).unwrap();
    let landxml_before = fs::read(fixture.run_root.join("terrain.xml")).unwrap();
    let audit_before = fs::read(fixture.run_root.join("audit.json")).unwrap();
    let returned = fixture.directory.path().join("returned.xml");
    let evidence = fixture.directory.path().join("round-trip.json");
    fs::write(&returned, &landxml_before).expect("write returned LandXML fixture");

    let first = fixture.verify_round_trip(&returned, &evidence);
    assert_success(&first);
    let first_summary = String::from_utf8_lossy(&first.stdout);
    for expected in [
        "LandXML round-trip passed",
        &format!("Run {}", hex(&RUN_ID)),
        "canonical evidence published true",
        "evidence hash ",
        "evidence bytes ",
        "external application execution verified false",
    ] {
        assert!(
            first_summary.contains(expected),
            "missing {expected:?}\n{first_summary}"
        );
    }
    let evidence_bytes = fs::read(&evidence).expect("read canonical evidence");
    let document: serde_json::Value =
        serde_json::from_slice(&evidence_bytes).expect("parse canonical evidence");
    assert_eq!(
        document["schema"],
        "punctra.terrain-demo.landxml-round-trip-evidence.v1"
    );
    assert_eq!(document["result"], "passed");
    assert_eq!(document["run"]["run_id"], hex(&RUN_ID));
    assert_eq!(document["checks"]["provenance"]["status"], "passed");
    assert_eq!(document["checks"]["topology"]["status"], "passed");
    assert_eq!(
        document["nonclaims"]["punctra_observed_downstream_execution"],
        false
    );

    let second = fixture.verify_round_trip(&returned, &evidence);
    assert_success(&second);
    assert_eq!(first.stdout, second.stdout);
    assert_eq!(fs::read(&evidence).unwrap(), evidence_bytes);
    assert_eq!(
        fs::read(fixture.run_root.join("run.pwf")).unwrap(),
        journal_before
    );
    assert_eq!(
        fs::read(fixture.run_root.join("terrain.xml")).unwrap(),
        landxml_before
    );
    assert_eq!(
        fs::read(fixture.run_root.join("audit.json")).unwrap(),
        audit_before
    );
}

#[test]
fn verify_round_trip_preserves_small_and_subnormal_tolerances() {
    let fixture = ProcessFixture::new("round-trip-small-tolerances");
    assert_success(&fixture.run("start"));
    let returned = fixture.directory.path().join("returned.xml");
    fs::copy(fixture.run_root.join("terrain.xml"), &returned).unwrap();

    for (name, horizontal, vertical) in [("small", "1e-18", "0"), ("subnormal", "5e-324", "5e-324")]
    {
        let evidence = fixture.directory.path().join(format!("{name}.json"));
        let verified =
            fixture.verify_round_trip_with_tolerances(&returned, &evidence, horizontal, vertical);
        assert_success(&verified);

        let document: serde_json::Value =
            serde_json::from_slice(&fs::read(evidence).unwrap()).unwrap();
        let policy = &document["comparison_policy"];
        assert_eq!(
            policy["horizontal_tolerance_metres"]
                .as_f64()
                .unwrap()
                .to_bits(),
            horizontal.parse::<f64>().unwrap().to_bits()
        );
        assert_eq!(
            policy["vertical_tolerance_metres"]
                .as_f64()
                .unwrap()
                .to_bits(),
            vertical.parse::<f64>().unwrap().to_bits()
        );
    }
}

#[test]
fn verify_round_trip_publishes_failed_evidence_but_not_operational_failures() {
    let fixture = ProcessFixture::new("round-trip-failed-evidence");
    assert_success(&fixture.run("start"));
    let original = fs::read_to_string(fixture.run_root.join("terrain.xml")).unwrap();
    let returned = fixture.directory.path().join("returned.xml");
    let evidence = fixture.directory.path().join("round-trip.json");
    let changed = original.replacen(">4600000 500000 120<", ">4600000.5 500000 120<", 1);
    assert_ne!(
        changed, original,
        "generated fixture coordinate must be changed"
    );
    fs::write(&returned, changed).unwrap();

    let failed = fixture.verify_round_trip(&returned, &evidence);
    assert!(!failed.status.success(), "semantic drift must fail");
    let diagnostic = String::from_utf8_lossy(&failed.stderr);
    assert!(diagnostic.contains("PRT_TOLERANCE_DRIFT"), "{diagnostic}");
    let document: serde_json::Value =
        serde_json::from_slice(&fs::read(&evidence).unwrap()).unwrap();
    assert_eq!(document["result"], "failed");
    assert_eq!(document["checks"]["tolerance"]["status"], "failed");
    assert_eq!(document["returned_landxml"]["namespace"], LANDXML_NAMESPACE);
    assert!(document["returned_landxml"]["point_count"].is_number());
    assert_eq!(document["comparison"]["mapped_point_count"], 61);
    assert_eq!(document["comparison"]["unmatched_point_count"], 1);
    assert_eq!(document["comparison"]["ambiguous_point_count"], 0);

    assert_ambiguous_mapping_evidence(&fixture, &original);

    let malformed_returned = fixture.directory.path().join("malformed-returned.xml");
    let malformed_evidence = fixture.directory.path().join("malformed-round-trip.json");
    fs::write(&malformed_returned, "<LandXML").unwrap();
    let malformed = fixture.verify_round_trip(&malformed_returned, &malformed_evidence);
    assert!(!malformed.status.success());
    let diagnostic = String::from_utf8_lossy(&malformed.stderr);
    assert!(diagnostic.contains("PRT_XML_INVALID"), "{diagnostic}");
    let document: serde_json::Value =
        serde_json::from_slice(&fs::read(&malformed_evidence).unwrap()).unwrap();
    assert_eq!(document["checks"]["parse"]["status"], "failed");
    assert!(document["returned_landxml"]["namespace"].is_null());

    let topology_returned = fixture.directory.path().join("topology-returned.xml");
    let topology_evidence = fixture.directory.path().join("topology-round-trip.json");
    let topology_changed = original.replacen("<F>1 9 2</F>", "<F>1 9 10</F>", 1);
    assert_ne!(topology_changed, original);
    fs::write(&topology_returned, topology_changed).unwrap();
    let topology_failed = fixture.verify_round_trip(&topology_returned, &topology_evidence);
    assert!(!topology_failed.status.success());
    let diagnostic = String::from_utf8_lossy(&topology_failed.stderr);
    assert!(diagnostic.contains("PRT_TOPOLOGY_DRIFT"), "{diagnostic}");
    let document: serde_json::Value =
        serde_json::from_slice(&fs::read(&topology_evidence).unwrap()).unwrap();
    assert_eq!(document["checks"]["topology"]["status"], "failed");
    assert_eq!(document["comparison"]["added_face_count"], 1);
    assert_eq!(document["comparison"]["removed_face_count"], 1);
    assert_eq!(document["comparison"]["mapped_point_count"], 62);
    assert_eq!(document["comparison"]["unmatched_point_count"], 0);
    assert_eq!(document["comparison"]["ambiguous_point_count"], 0);
    assert!(document["comparison"]["maximum_horizontal_delta_metres"].is_number());
    assert!(document["comparison"]["maximum_vertical_delta_metres"].is_number());
    assert!(document["comparison"]["added_face_hash"].is_string());
    assert!(document["comparison"]["removed_face_hash"].is_string());

    let incomplete = ProcessFixture::new("round-trip-incomplete-run");
    assert_success(&incomplete.run("start"));
    let journal_path = incomplete.run_root.join("run.pwf");
    let complete = fs::read(&journal_path).unwrap();
    let ends = journal_frame_ends(&complete).unwrap();
    restore_journal_prefix(&journal_path, &complete, ends[6]).unwrap();
    let returned = incomplete.directory.path().join("returned.xml");
    fs::copy(incomplete.run_root.join("terrain.xml"), &returned).unwrap();
    let evidence = incomplete.directory.path().join("round-trip.json");
    let operational = incomplete.verify_round_trip(&returned, &evidence);
    assert!(!operational.status.success());
    assert!(
        !evidence.exists(),
        "non-Complete Run must publish no evidence"
    );
    assert_eq!(fs::read(&journal_path).unwrap(), &complete[..ends[6]]);
}

fn assert_ambiguous_mapping_evidence(fixture: &ProcessFixture, original: &str) {
    let parsed_original = Document::parse(original).unwrap();
    let mut point_text = parsed_original
        .descendants()
        .filter(|node| node.has_tag_name((LANDXML_NAMESPACE, "P")))
        .filter_map(|node| node.text());
    let first_point = point_text.next().unwrap();
    let second_point = point_text.next().unwrap();
    let mut first_coordinates = first_point.split_whitespace();
    let northing = first_coordinates.next().unwrap().parse::<f64>().unwrap();
    let easting = first_coordinates.next().unwrap().parse::<f64>().unwrap();
    let elevation = first_coordinates.next().unwrap();
    let near_first_point = format!("{} {} {elevation}", northing + 0.04, easting + 0.04);
    let returned = fixture.directory.path().join("ambiguous-returned.xml");
    let evidence = fixture.directory.path().join("ambiguous-round-trip.json");
    let changed = original.replacen(
        &format!(">{second_point}<"),
        &format!(">{near_first_point}<"),
        1,
    );
    fs::write(&returned, changed).unwrap();

    let output = fixture.verify_round_trip_with_tolerances(&returned, &evidence, "0.1", "0");
    assert!(!output.status.success());
    let document: serde_json::Value =
        serde_json::from_slice(&fs::read(&evidence).unwrap()).unwrap();
    assert_eq!(document["comparison"]["mapped_point_count"], 60);
    assert_eq!(document["comparison"]["unmatched_point_count"], 1);
    assert_eq!(document["comparison"]["ambiguous_point_count"], 1);
}

#[test]
fn verify_round_trip_never_overwrites_conflicting_or_run_root_targets() {
    let fixture = ProcessFixture::new("round-trip-target-conflict");
    assert_success(&fixture.run("start"));
    let returned = fixture.directory.path().join("returned.xml");
    fs::copy(fixture.run_root.join("terrain.xml"), &returned).unwrap();
    let evidence = fixture.directory.path().join("round-trip.json");
    let conflict = b"caller-owned evidence conflict";
    fs::write(&evidence, conflict).unwrap();

    let failed = fixture.verify_round_trip(&returned, &evidence);
    assert!(!failed.status.success());
    assert_eq!(fs::read(&evidence).unwrap(), conflict);

    let inside_run = fixture.run_root.join("round-trip.json");
    let failed = fixture.verify_round_trip(&returned, &inside_run);
    assert!(!failed.status.success());
    assert!(!inside_run.exists());
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
        "use inputs within the named round-trip limits",
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
            .args(["--downstream-setting", "metric-tin-v1"])
            .args(["--horizontal-tolerance-metres", horizontal_tolerance])
            .args(["--vertical-tolerance-metres", vertical_tolerance])
            .arg(&self.run_root)
            .arg(returned)
            .arg(evidence)
            .output()
            .expect("verify process LandXML round trip")
    }
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
