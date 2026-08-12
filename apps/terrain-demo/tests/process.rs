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
fn verify_round_trip_publishes_deterministic_pass_and_semantic_failure_evidence() {
    let fixture = ProcessFixture::new("qualification");
    assert_success(&fixture.run("start"));
    let run_before = snapshot_run(&fixture.run_root);
    let original = fs::read(fixture.run_root.join("terrain.xml")).unwrap();
    let returned = fixture.directory.path().join("returned.xml");
    fs::write(&returned, &original).unwrap();
    let passing = fixture.directory.path().join("passing-evidence.json");

    let passed = fixture.verify(&returned, &passing, ["format=LandXML;1.2", "units=meter"]);
    assert_success(&passed);
    let stdout = String::from_utf8_lossy(&passed.stdout);
    assert!(stdout.contains("Round-Trip Evidence passed"), "{stdout}");
    assert!(stdout.contains("evidence hash "), "{stdout}");
    let passing_bytes = fs::read(&passing).unwrap();
    let passing_json: serde_json::Value = serde_json::from_slice(&passing_bytes).unwrap();
    assert_eq!(
        passing_json["schema"],
        "punctra.terrain-demo.landxml-round-trip-evidence.v1"
    );
    assert_eq!(passing_json["result"], "passed");
    assert_eq!(
        passing_json["downstream_declaration"]["settings"][0]["key"],
        "format"
    );
    assert_eq!(
        passing_json["downstream_declaration"]["settings"][0]["value"],
        "LandXML;1.2"
    );
    assert_eq!(snapshot_run(&fixture.run_root), run_before);

    let reconciled = fixture.verify(&returned, &passing, ["units=meter", "format=LandXML;1.2"]);
    assert_success(&reconciled);
    assert!(
        String::from_utf8_lossy(&reconciled.stdout).contains("disposition reconciled_existing")
    );
    assert_eq!(fs::read(&passing).unwrap(), passing_bytes);

    let returned_text = String::from_utf8(original).unwrap();
    let point_start = returned_text.find("<P id=\"").unwrap();
    let coordinate_start = returned_text[point_start..].find('>').unwrap() + point_start + 1;
    let coordinate_end = returned_text[coordinate_start..].find('<').unwrap() + coordinate_start;
    let mut coordinates = returned_text[coordinate_start..coordinate_end]
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    coordinates[1] = format!("{:.17}", coordinates[1].parse::<f64>().unwrap() + 100.0);
    let returned_text = format!(
        "{}{}{}",
        &returned_text[..coordinate_start],
        coordinates.join(" "),
        &returned_text[coordinate_end..]
    );
    fs::write(&returned, returned_text).unwrap();
    let failed_target = fixture.directory.path().join("failed-evidence.json");
    let failed = fixture.verify(
        &returned,
        &failed_target,
        ["format=LandXML;1.2", "units=meter"],
    );
    assert!(!failed.status.success());
    let diagnostic = String::from_utf8_lossy(&failed.stderr);
    assert!(diagnostic.contains("PRT_SEMANTIC_MISMATCH"), "{diagnostic}");
    assert!(diagnostic.contains("hash "), "{diagnostic}");
    assert!(diagnostic.contains("bytes "), "{diagnostic}");
    let failed_json: serde_json::Value =
        serde_json::from_slice(&fs::read(&failed_target).unwrap()).unwrap();
    assert_eq!(failed_json["result"], "failed");
    assert_eq!(
        failed_json["checks"]["unique_mapping"]["reason"],
        "PRT_TOLERANCE_DRIFT"
    );
    assert_eq!(snapshot_run(&fixture.run_root), run_before);
    write_qualification_corpus_if_requested(&fixture, &returned, &passing, &failed_target);
}

#[test]
fn verify_round_trip_operational_failures_publish_no_evidence_and_preserve_inputs() {
    let fixture = ProcessFixture::new("qualification-fail-closed");
    assert_success(&fixture.run("start"));
    let run_before = snapshot_run(&fixture.run_root);
    let returned = fixture.directory.path().join("returned.xml");
    fs::write(&returned, b"<not-complete").unwrap();
    let evidence = fixture.directory.path().join("evidence.json");
    let malformed = fixture.verify(&returned, &evidence, ["units=meter"]);
    assert!(!malformed.status.success());
    assert!(!evidence.exists());
    assert_eq!(snapshot_run(&fixture.run_root), run_before);

    let journal = fixture.run_root.join("run.pwf");
    let mut torn = fs::read(&journal).unwrap();
    torn.pop();
    overwrite_and_sync(&journal, &torn).unwrap();
    fs::write(
        &returned,
        fs::read(fixture.run_root.join("terrain.xml")).unwrap(),
    )
    .unwrap();
    let torn_before = fs::read(&journal).unwrap();
    let rejected = fixture.verify(&returned, &evidence, ["units=meter"]);
    assert!(!rejected.status.success());
    assert_eq!(
        fs::read(&journal).unwrap(),
        torn_before,
        "qualification must not repair"
    );
    assert!(!evidence.exists());
}

#[test]
fn verify_round_trip_rejects_same_file_and_preserves_conflicting_evidence() {
    let fixture = ProcessFixture::new("qualification-targets");
    assert_success(&fixture.run("start"));
    let evidence = fixture.directory.path().join("evidence.json");
    let same = fixture.verify(
        &fixture.run_root.join("terrain.xml"),
        &evidence,
        ["units=meter"],
    );
    assert!(!same.status.success());
    assert!(!evidence.exists());
    assert!(
        String::from_utf8_lossy(&same.stderr).contains("must be a distinct file"),
        "{}",
        String::from_utf8_lossy(&same.stderr)
    );

    let returned = fixture.directory.path().join("returned.xml");
    fs::copy(fixture.run_root.join("terrain.xml"), &returned).unwrap();
    let caller = b"caller-owned evidence conflict\n";
    fs::write(&evidence, caller).unwrap();
    let conflict = fixture.verify(&returned, &evidence, ["units=meter"]);
    assert!(!conflict.status.success());
    assert_eq!(fs::read(&evidence).unwrap(), caller);
}

#[test]
fn verify_round_trip_publishes_unit_and_duplicate_face_failures() {
    let fixture = ProcessFixture::new("qualification-semantic-families");
    assert_success(&fixture.run("start"));
    let original = fs::read_to_string(fixture.run_root.join("terrain.xml")).unwrap();
    let units_start = original.find("  <Units>").unwrap();
    let units_end = original.find("  </Units>\n").unwrap() + "  </Units>\n".len();
    let units = &original[units_start..units_end];
    for (name, replacement) in [
        ("missing", String::new()),
        ("duplicate", format!("{units}{units}")),
        (
            "imperial",
            "  <Units><Imperial linearUnit=\"foot\"/></Units>\n".to_owned(),
        ),
        (
            "non-meter",
            "  <Units><Metric linearUnit=\"millimeter\"/></Units>\n".to_owned(),
        ),
    ] {
        let returned = fixture.directory.path().join(format!("{name}.xml"));
        fs::write(
            &returned,
            format!(
                "{}{}{}",
                &original[..units_start],
                replacement,
                &original[units_end..]
            ),
        )
        .unwrap();
        let evidence = fixture.directory.path().join(format!("{name}.json"));
        let output = fixture.verify(&returned, &evidence, ["units=declared"]);
        assert!(!output.status.success(), "{}", diagnostics(&output));
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(&evidence).unwrap()).unwrap();
        assert_eq!(value["result"], "failed");
        assert_eq!(value["checks"]["units"]["reason"], "PRT_UNIT_DRIFT");
        for dependent in ["unique_mapping", "tolerance", "topology"] {
            assert_eq!(value["checks"][dependent]["status"], "not_evaluated");
            assert_eq!(value["checks"][dependent]["reason"], "none");
        }
    }

    let first_face_start = original.find("          <F>").unwrap();
    let first_face_end =
        original[first_face_start..].find("</F>").unwrap() + first_face_start + "</F>".len();
    let first_face = &original[first_face_start..first_face_end];
    let duplicate = format!(
        "{}{}\n{}{}",
        &original[..first_face_start],
        first_face,
        first_face,
        &original[first_face_end..]
    );
    let returned = fixture.directory.path().join("duplicate-face.xml");
    fs::write(&returned, duplicate).unwrap();
    let evidence = fixture.directory.path().join("duplicate-face.json");
    let output = fixture.verify(&returned, &evidence, ["units=meter"]);
    assert!(!output.status.success(), "{}", diagnostics(&output));
    let value: serde_json::Value = serde_json::from_slice(&fs::read(&evidence).unwrap()).unwrap();
    assert_eq!(value["checks"]["topology"]["status"], "failed");
    assert_eq!(value["checks"]["topology"]["reason"], "PRT_TOPOLOGY_DRIFT");
    assert_eq!(value["comparison"]["added_face_count"], 1);
}

#[test]
fn verify_round_trip_rejects_relocated_run_root_without_mutation() {
    let fixture = ProcessFixture::new("qualification-relocated-run");
    assert_success(&fixture.run("start"));
    let relocated = fixture.directory.path().join("relocated-run");
    fs::rename(&fixture.run_root, &relocated).unwrap();
    let before = snapshot_run(&relocated);
    let returned = fixture.directory.path().join("returned.xml");
    fs::copy(relocated.join("terrain.xml"), &returned).unwrap();
    let evidence = fixture.directory.path().join("relocated-evidence.json");
    let output = ProcessFixture::verify_at(&relocated, &returned, &evidence, ["units=meter"]);
    assert!(!output.status.success(), "relocated Run must fail closed");
    assert!(!evidence.exists());
    assert_eq!(snapshot_run(&relocated), before);
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

    fn verify<const N: usize>(
        &self,
        returned: &std::path::Path,
        evidence: &std::path::Path,
        settings: [&str; N],
    ) -> Output {
        Self::verify_at(&self.run_root, returned, evidence, settings)
    }

    fn verify_at<const N: usize>(
        run_root: &std::path::Path,
        returned: &std::path::Path,
        evidence: &std::path::Path,
        settings: [&str; N],
    ) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_terrain-demo"));
        command
            .arg("verify-round-trip")
            .args(["--downstream-app", "generated-fixture"])
            .args(["--downstream-version", "test-only"]);
        for setting in settings {
            command.args(["--downstream-setting", setting]);
        }
        command
            .args(["--horizontal-tolerance-metres", "0.001"])
            .args(["--vertical-tolerance-metres", "0.001"])
            .arg(run_root)
            .arg(returned)
            .arg(evidence)
            .output()
            .expect("run qualification process")
    }
}

fn snapshot_run(root: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut entries = fs::read_dir(root)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().to_string_lossy().into_owned(),
                fs::read(entry.path()).unwrap(),
            )
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}

fn write_qualification_corpus_if_requested(
    fixture: &ProcessFixture,
    failed_returned: &std::path::Path,
    passing_evidence: &std::path::Path,
    failed_evidence: &std::path::Path,
) {
    if std::env::var_os("PUNCTRA_WRITE_QUALIFICATION_CORPUS").is_none() {
        return;
    }
    let destination =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/qualification-v1");
    fs::create_dir_all(&destination).unwrap();
    let complete = fs::read(fixture.run_root.join("run.pwf")).unwrap();
    let artifacts = [
        ("run-complete.pwf", complete.clone()),
        (
            "terrain.xml",
            fs::read(fixture.run_root.join("terrain.xml")).unwrap(),
        ),
        (
            "audit.json",
            fs::read(fixture.run_root.join("audit.json")).unwrap(),
        ),
        (
            "returned-pass.xml",
            fs::read(fixture.run_root.join("terrain.xml")).unwrap(),
        ),
        ("returned-fail.xml", fs::read(failed_returned).unwrap()),
        ("evidence-pass.json", fs::read(passing_evidence).unwrap()),
        ("evidence-fail.json", fs::read(failed_evidence).unwrap()),
    ];
    let mut entries = std::collections::BTreeMap::new();
    for (name, bytes) in artifacts {
        fs::write(destination.join(name), &bytes).unwrap();
        entries.insert(
            name.to_owned(),
            manifest_entry(&bytes, qualification_support_class(name)),
        );
    }
    let mut prefixes = Vec::new();
    for (index, end) in journal_frame_ends(&complete)
        .unwrap()
        .into_iter()
        .enumerate()
    {
        let name = format!("run-prefix-{:02}.pwf", index + 1);
        let bytes = &complete[..end];
        fs::write(destination.join(&name), bytes).unwrap();
        entries.insert(name.clone(), manifest_entry(bytes, "authoritative"));
        prefixes.push(name);
    }
    let audit: serde_json::Value =
        serde_json::from_slice(&fs::read(destination.join("audit.json")).unwrap()).unwrap();
    let evidence: serde_json::Value =
        serde_json::from_slice(&fs::read(destination.join("evidence-pass.json")).unwrap()).unwrap();
    let manifest = serde_json::json!({
        "schema": "punctra.terrain-demo.qualification-corpus.v1",
        "owner": "terrain-demo",
        "path_base": "manifest_directory",
        "run_versions": {"disk": 1, "semantic": 1, "frame": 1},
        "report_schema": "punctra.terrain-workflow.audit.v1",
        "evidence_schema": "punctra.terrain-demo.landxml-round-trip-evidence.v1",
        "matcher_version": 1,
        "declaration": {
            "application": "generated-fixture",
            "version": "test-only",
            "settings": [
                {"key": "format", "value": "LandXML;1.2"},
                {"key": "units", "value": "meter"}
            ]
        },
        "tolerances_metres": {"horizontal": 0.001, "vertical": 0.001},
        "entries": entries,
        "journal_checkpoint_prefixes": prefixes,
        "expected": {
            "run_identity": audit["identities"]["run"],
            "source_identity": audit["identities"]["source"],
            "workspace_identity": audit["identities"]["workspace"],
            "baseline_revision": audit["identities"]["baseline_revision"],
            "operation_identity": audit["identities"]["operation"],
            "request_hash": audit["request"]["request_hash"],
            "revision": audit["identities"]["changed_revision"],
            "complete_journal_hash": evidence["run"]["complete_journal_hash"],
            "terrain_xml_hash": evidence["run"]["terrain_xml_hash"],
            "terrain_xml_bytes": evidence["run"]["terrain_xml_bytes"],
            "audit_json_hash": evidence["run"]["audit_json_hash"],
            "audit_json_bytes": evidence["run"]["audit_json_bytes"],
            "passing_result": evidence["result"],
            "passing_point_count": evidence["returned_landxml"]["point_count"],
            "passing_face_count": evidence["returned_landxml"]["face_count"],
            "passing_mapped_point_count": evidence["comparison"]["mapped_point_count"],
            "passing_added_face_count": evidence["comparison"]["added_face_count"],
            "passing_removed_face_count": evidence["comparison"]["removed_face_count"],
            "failed_result": "failed",
            "failed_reason": "PRT_TOLERANCE_DRIFT",
            "failed_unmatched_point_count": 1
        }
    });
    let mut bytes = serde_json::to_vec_pretty(&manifest).unwrap();
    bytes.push(b'\n');
    fs::write(destination.join("manifest.json"), bytes).unwrap();
}

fn manifest_entry(bytes: &[u8], support_class: &str) -> serde_json::Value {
    serde_json::json!({
        "byte_length": bytes.len(),
        "blake3": blake3::hash(bytes).to_hex().to_string(),
        "support_class": support_class
    })
}

fn qualification_support_class(name: &str) -> &'static str {
    match name {
        "run-complete.pwf" => "authoritative",
        "terrain.xml" | "audit.json" | "evidence-pass.json" | "evidence-fail.json" => {
            "caller_owned_published"
        }
        "returned-pass.xml" | "returned-fail.xml" => "test_only_input",
        _ => unreachable!("fixed qualification corpus artifact"),
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
