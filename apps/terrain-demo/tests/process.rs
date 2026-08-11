//! GPU-free process coverage for the complete LAS and LAZ terrain host.

use std::{
    fs, io,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

use las::{
    Builder, Point, Transform, Vector, Writer,
    point::{Classification, Format},
};
use roxmltree::Document;

const DOCUMENT_DATE: &str = "2026-01-02";
const DOCUMENT_TIME: &str = "03:04:05Z";
const LANDXML_NAMESPACE: &str = "http://www.landxml.org/schema/LandXML-1.2";

#[test]
fn generated_las_and_laz_run_the_complete_host_without_a_gpu() {
    let directory = TestDirectory::new().expect("create test directory");
    let mut previous_xml = None;

    for extension in ["las", "laz"] {
        let source = directory.path().join(format!("fixture.{extension}"));
        let index = directory.path().join(format!("fixture-{extension}.pidx"));
        let workspace = directory.path().join(format!("fixture-{extension}.pcw"));
        let target = directory.path().join(format!("fixture-{extension}.xml"));
        write_fixture(&source).expect("write generated Source fixture");
        let source_bytes = fs::read(&source).expect("read immutable generated Source bytes");

        let built = run_host(&source, &index, &workspace, &target, true, true);
        assert_success(&built);
        assert_bounded_summary(&built, "Built", "created");
        assert!(!recovery_record_path(&workspace).exists());
        assert_eq!(
            fs::read(&source).expect("reread generated Source after correction and Revert"),
            source_bytes,
            "{extension} Source bytes must remain immutable",
        );
        let first_xml = fs::read(&target).expect("read first LandXML output");
        assert_landxml(&first_xml);

        fs::remove_file(&target).expect("remove first output before failure check");
        let missing_assertion = run_host(&source, &index, &workspace, &target, false, false);
        assert!(!missing_assertion.status.success());
        assert!(
            String::from_utf8_lossy(&missing_assertion.stderr)
                .contains("Source coordinates require an explicit metric-metre assertion"),
            "{}",
            diagnostics(&missing_assertion),
        );
        assert!(
            !target.exists(),
            "failed publication must not leave a target"
        );

        let opened = run_host(&source, &index, &workspace, &target, true, true);
        assert_success(&opened);
        assert_bounded_summary(&opened, "Opened", "opened");
        assert!(!recovery_record_path(&workspace).exists());
        assert_eq!(
            fs::read(&source)
                .expect("reread generated Source after reopened correction and Revert"),
            source_bytes,
            "repeated {extension} correction and Revert must not rewrite Source bytes",
        );
        let second_xml = fs::read(&target).expect("read repeated LandXML output");
        assert_eq!(first_xml, second_xml, "repeat {extension} output");
        if let Some(previous_xml) = &previous_xml {
            assert_eq!(previous_xml, &second_xml, "LAS and LAZ output equivalence");
        }
        previous_xml = Some(second_xml);
    }
}

#[test]
fn failed_changed_derivation_still_reverts_the_workspace() {
    let directory = TestDirectory::new().expect("create test directory");
    let source = directory.path().join("minimal-ground.las");
    let index = directory.path().join("minimal-ground.pidx");
    let workspace = directory.path().join("minimal-ground.pcw");
    let target = directory.path().join("minimal-ground.xml");
    write_minimal_ground_fixture(&source).expect("write minimal Ground fixture");

    let correction = run_host(&source, &index, &workspace, &target, true, true);
    assert!(!correction.status.success());
    assert!(
        String::from_utf8_lossy(&correction.stderr)
            .contains("terrain requires at least three Ground Input Points; found 2"),
        "{}",
        diagnostics(&correction),
    );
    assert!(!recovery_record_path(&workspace).exists());
    assert!(!target.exists());

    let reopened = run_host(&source, &index, &workspace, &target, true, false);
    assert_success(&reopened);
    let stdout = String::from_utf8_lossy(&reopened.stdout);
    assert!(stdout.contains("Ground Input Points: 3"), "{stdout}");
    assert!(target.exists());
}

fn recovery_record_path(workspace: &Path) -> PathBuf {
    let mut path = workspace.as_os_str().to_os_string();
    path.push(".recovery");
    PathBuf::from(path)
}

fn run_host(
    source: &Path,
    index: &Path,
    workspace: &Path,
    target: &Path,
    assert_metric: bool,
    exercise_correction_revert: bool,
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_terrain-demo"));
    command
        .args([
            "--qa-sample",
            "--date",
            DOCUMENT_DATE,
            "--time",
            DOCUMENT_TIME,
        ])
        .arg(source)
        .arg(index)
        .arg(workspace)
        .arg(target);
    if exercise_correction_revert {
        command.args(["--exercise-correction-revert", "4"]);
    }
    if assert_metric {
        command.arg("--assert-crs-metric");
    }
    command.output().expect("run terrain-demo process")
}

fn assert_success(output: &Output) {
    assert!(output.status.success(), "{}", diagnostics(output));
}

fn assert_bounded_summary(output: &Output, index_disposition: &str, workspace_disposition: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "Verified Source",
        &format!("disposition: {index_disposition}"),
        &format!("Workspace {workspace_disposition}"),
        "Terrain derived",
        "Classification correction and Revert",
        "changed Ground Input Points: 4",
        "restored Ground Input Points: 5",
        "restored geometry/topology hashes: yes",
        "Detached QA sample",
        "covered: 1",
        "gaps: 1",
        "LandXML exported",
    ] {
        assert!(stdout.contains(expected), "missing {expected:?}\n{stdout}");
    }
    assert!(stdout.len() < 8 * 1024, "summary must remain bounded");
    assert!(stdout.lines().count() < 64, "summary must remain concise");
}

fn assert_landxml(bytes: &[u8]) {
    let text = std::str::from_utf8(bytes).expect("LandXML is UTF-8");
    let document = Document::parse(text).expect("independent XML parser accepts output");
    let root = document.root_element();
    assert_eq!(root.tag_name().name(), "LandXML");
    assert_eq!(root.tag_name().namespace(), Some(LANDXML_NAMESPACE));
    assert_eq!(root.attribute("version"), Some("1.2"));
    assert_eq!(root.attribute("date"), Some(DOCUMENT_DATE));
    assert_eq!(root.attribute("time"), Some(DOCUMENT_TIME));

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
        5,
    );
    assert!(
        document
            .descendants()
            .filter(|node| node.has_tag_name((LANDXML_NAMESPACE, "F")))
            .count()
            >= 4
    );
}

fn write_fixture(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = Builder::from((1, 4));
    builder.point_format = Format::new(0)?;
    builder.transforms = Vector {
        x: Transform {
            scale: 0.01,
            offset: 500_000.0,
        },
        y: Transform {
            scale: 0.01,
            offset: 4_600_000.0,
        },
        z: Transform {
            scale: 0.01,
            offset: 120.0,
        },
    };
    let mut writer = Writer::from_path(path, builder.into_header()?)?;
    for [x, y, z] in [
        [500_000.0, 4_600_000.0, 120.0],
        [500_010.0, 4_600_000.0, 121.0],
        [500_010.0, 4_600_010.0, 123.0],
        [500_000.0, 4_600_010.0, 122.0],
        [500_005.0, 4_600_005.0, 121.5],
    ] {
        writer.write_point(Point {
            x,
            y,
            z,
            return_number: 1,
            number_of_returns: 1,
            classification: Classification::Ground,
            ..Point::default()
        })?;
    }
    writer.close()?;
    Ok(())
}

fn write_minimal_ground_fixture(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = Builder::from((1, 4));
    builder.point_format = Format::new(0)?;
    builder.transforms = Vector {
        x: Transform {
            scale: 0.01,
            offset: 0.0,
        },
        y: Transform {
            scale: 0.01,
            offset: 0.0,
        },
        z: Transform {
            scale: 0.01,
            offset: 0.0,
        },
    };
    let mut writer = Writer::from_path(path, builder.into_header()?)?;
    for ([x, y, z], classification) in [
        ([-20.0, -20.0, 0.0], Classification::Unclassified),
        ([-10.0, -10.0, 0.0], Classification::Unclassified),
        ([0.0, 0.0, 0.0], Classification::Ground),
        ([10.0, 0.0, 1.0], Classification::Ground),
        ([0.0, 10.0, 2.0], Classification::Ground),
    ] {
        writer.write_point(Point {
            x,
            y,
            z,
            return_number: 1,
            number_of_returns: 1,
            classification,
            ..Point::default()
        })?;
    }
    writer.close()?;
    Ok(())
}

fn diagnostics(output: &Output) -> String {
    format!(
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> io::Result<Self> {
        static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
        loop {
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "punctra-terrain-demo-{}-{sequence}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
