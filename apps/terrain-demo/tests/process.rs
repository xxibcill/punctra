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

    let inspected = Command::new(env!("CARGO_BIN_EXE_terrain-demo"))
        .arg("inspect")
        .arg(&fixture.run_root)
        .output()
        .expect("inspect workflow process");
    assert_success(&inspected);
    let inspection = String::from_utf8_lossy(&inspected.stdout);
    assert!(inspection.contains("frames 8"), "{inspection}");
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
}

struct ProcessFixture {
    _directory: TestDirectory,
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
            _directory: directory,
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
        "frames 8",
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
