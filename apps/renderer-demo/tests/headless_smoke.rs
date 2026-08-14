//! Process-level smoke coverage for the synthetic and verified LAS bridges.

use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use las::point::{Classification, Format};
use las::{Builder, Color, Point, Transform, Vector, Writer};

#[test]
fn synthetic_headless_smoke_needs_no_gpu() {
    let output = Command::new(env!("CARGO_BIN_EXE_renderer-demo"))
        .arg("--smoke")
        .output()
        .unwrap();

    assert!(output.status.success(), "{}", diagnostics(&output));
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("Headless bridge smoke accepted one atomic Upsert")
    );
}

#[test]
fn orthographic_headless_smoke_uses_the_same_cpu_model_path() {
    let output = Command::new(env!("CARGO_BIN_EXE_renderer-demo"))
        .args(["--smoke", "--projection", "orthographic"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{}", diagnostics(&output));
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("Headless bridge smoke accepted one atomic Upsert")
    );
}

#[test]
fn invalid_request_and_missing_source_have_stable_phase_diagnostics() {
    for arguments in [
        vec!["--unknown"],
        vec!["--display"],
        vec!["--display", "neutral", "--display", "elevation"],
        vec!["--projection"],
        vec![
            "--projection",
            "perspective",
            "--projection",
            "orthographic",
        ],
    ] {
        let invalid = Command::new(env!("CARGO_BIN_EXE_renderer-demo"))
            .args(arguments)
            .output()
            .unwrap();
        assert!(!invalid.status.success());
        assert!(
            String::from_utf8_lossy(&invalid.stderr)
                .contains("PVIEW_INVALID_REQUEST at request-validation"),
            "{}",
            diagnostics(&invalid)
        );
    }

    let missing = Command::new(env!("CARGO_BIN_EXE_renderer-demo"))
        .args(["--smoke", "definitely-missing-source.laz"])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(
        String::from_utf8_lossy(&missing.stderr).contains("PVIEW_SOURCE at source-verification"),
        "{}",
        diagnostics(&missing)
    );
}

#[test]
fn las_and_laz_headless_smoke_full_verify_build_open_and_upload_one_node() {
    let directory = TestDirectory::new().unwrap();
    for extension in ["las", "laz"] {
        let source = directory.path().join(format!("fixture.{extension}"));
        let index = directory.path().join(format!("fixture-{extension}.pidx"));
        write_las(&source).unwrap();

        let built = run_smoke(&source, &index);
        assert_smoke_output(&built, "Built");
        assert!(
            String::from_utf8_lossy(&built.stdout).contains("display: neutral application color")
        );
        assert!(index.is_file());

        let opened = run_elevation_smoke(&source, &index);
        assert_smoke_output(&opened, "Opened");
        assert!(String::from_utf8_lossy(&opened.stdout).contains(
            "display: elevation palette normalized by Source world Z bounds [120, 121.5]"
        ));

        let inspection_index = directory
            .path()
            .join(format!("fixture-{extension}.inspection.pidx"));
        for (position, mode) in ["rgb", "intensity", "classification"]
            .into_iter()
            .enumerate()
        {
            let output = run_display_smoke(&source, &inspection_index, mode);
            assert_smoke_output(&output, if position == 0 { "Built" } else { "Opened" });
            assert!(
                String::from_utf8_lossy(&output.stdout).contains(&format!("display: {mode}")),
                "{}",
                diagnostics(&output)
            );
        }
    }
}

#[test]
fn rgb_display_rejects_a_source_without_rgb_before_index_work() {
    let directory = TestDirectory::new().unwrap();
    let source = directory.path().join("without-rgb.las");
    let index = directory.path().join("without-rgb.inspection.pidx");
    write_las_without_rgb(&source).unwrap();

    let output = run_display_smoke(&source, &index, "rgb");

    assert!(!output.status.success(), "{}", diagnostics(&output));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("PVIEW_INVALID_REQUEST at request-validation"),
        "{}",
        diagnostics(&output)
    );
    assert!(
        stderr.contains("requires all three LAS RGB Attributes 16, 17, and 18 as U16"),
        "{}",
        diagnostics(&output)
    );
    assert!(
        !index.exists(),
        "invalid display input must not create an index"
    );
}

#[test]
fn elevation_display_rejects_the_synthetic_fixture() {
    let output = Command::new(env!("CARGO_BIN_EXE_renderer-demo"))
        .args(["--smoke", "--display", "elevation"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("--display elevation requires a LAS or LAZ SOURCE"),
        "{}",
        diagnostics(&output)
    );
}

#[test]
fn default_bare_index_targets_separate_position_and_inspection_recipes() {
    let directory = TestDirectory::new().unwrap();
    let source_name = "fixture.las";
    write_las(&directory.path().join(source_name)).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_renderer-demo"))
        .current_dir(directory.path())
        .args(["--smoke", source_name])
        .output()
        .unwrap();

    assert_smoke_output(&output, "Built");
    assert!(directory.path().join("fixture.las.pidx").is_file());

    let attributed = Command::new(env!("CARGO_BIN_EXE_renderer-demo"))
        .current_dir(directory.path())
        .args(["--smoke", "--display", "classification", source_name])
        .output()
        .unwrap();

    assert_smoke_output(&attributed, "Built");
    assert!(
        directory
            .path()
            .join("fixture.las.inspection-v2.pidx")
            .is_file()
    );
    assert!(directory.path().join("fixture.las.pidx").is_file());
}

#[test]
fn corpus_failure_publishes_a_private_report_and_returns_the_causal_diagnostic() {
    let directory = TestDirectory::new().unwrap();
    let manifest = directory.path().join("manifest.json");
    let report = directory.path().join("report.json");
    let source = directory.path().join("secret-source-name.las");
    let index = directory.path().join("secret-index-name.pidx");
    write_corpus_manifest(&manifest, &source, &index, 2, 1).unwrap();

    let output = run_corpus(&manifest, &report);

    assert!(!output.status.success(), "{}", diagnostics(&output));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr
            .matches("PVIEW_SOURCE at source-verification")
            .count(),
        1
    );
    assert!(!stderr.contains("report-publication"));
    let encoded = fs::read_to_string(&report).unwrap();
    assert!(!encoded.contains("secret-source-name"));
    assert!(!encoded.contains("secret-index-name"));
    assert!(!encoded.contains("private-project-name"));
    assert!(!encoded.contains("private-firm-name"));
    let report: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(report["summary"]["failed_count"], 1);
    assert_eq!(report["entries"][0]["disposition"], "failed");
    assert_eq!(report["entries"][0]["failure"]["code"], "PVIEW_SOURCE");
    assert_eq!(
        report["entries"][0]["failure"]["phase"],
        "source-verification"
    );
    assert_eq!(report["entries"][0]["declared_initial_frame_count"], 2);
    assert_eq!(report["entries"][0]["declared_trace"][0]["frame_count"], 1);
    assert!(report["entries"][0].get("source_file_bytes").is_none());
}

#[test]
fn corpus_success_binds_trace_inputs_and_separate_resource_measurements() {
    let directory = TestDirectory::new().unwrap();
    let source = directory.path().join("fixture.las");
    let index = directory.path().join("fixture.pidx");
    let manifest = directory.path().join("manifest.json");
    let report = directory.path().join("report.json");
    write_las(&source).unwrap();
    write_corpus_manifest(&manifest, &source, &index, 3, 2).unwrap();

    let output = run_corpus(&manifest, &report);
    if !output.status.success()
        && String::from_utf8_lossy(&output.stderr).contains("PVIEW_GPU")
        && std::env::var_os("PUNCTRA_REQUIRE_GPU").is_none()
    {
        return;
    }
    assert!(output.status.success(), "{}", diagnostics(&output));
    let encoded = fs::read_to_string(&report).unwrap();
    assert!(!encoded.contains(source.to_string_lossy().as_ref()));
    assert!(!encoded.contains(index.to_string_lossy().as_ref()));
    assert!(!encoded.contains("peak_total_temporary_disk_bytes"));
    let report: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    let entry = &report["entries"][0];
    assert_eq!(entry["disposition"], "passed");
    assert_eq!(entry["index_disposition"], "built");
    assert!(entry["index_prepare_nanoseconds"].is_number());
    assert!(entry["index_warm_open_nanoseconds"].is_number());
    assert!(entry.get("source_file_bytes").is_none());
    assert_eq!(
        entry["index_artifact_bytes"],
        fs::metadata(&index).unwrap().len()
    );
    assert!(entry["peak_index_temporary_disk_bytes"].as_u64().unwrap() > 0);
    assert!(entry["first_accepted_visible_batch_nanoseconds"].is_number());
    assert!(entry["peak_queued_host_bytes"].as_u64().unwrap() >= 1);
    assert!(entry["peak_resident_batches"].as_u64().unwrap() >= 1);
    assert!(entry["peak_resident_points"].as_u64().unwrap() >= 1);
    assert!(entry["peak_resident_bytes"].as_u64().unwrap() >= 1);
    assert_eq!(entry["declared_initial_frame_count"], 3);
    assert_eq!(entry["declared_trace"][0]["frame_count"], 2);
    assert_eq!(entry["trace"].as_array().unwrap().len(), 2);
    assert_eq!(entry["trace"][0]["requested_frame_count"], 3);
    assert_eq!(entry["trace"][0]["completed_frame_count"], 3);
    assert_eq!(entry["trace"][0]["input"]["frame_count"], 3);
    assert_eq!(entry["trace"][1]["requested_frame_count"], 2);
    assert_eq!(entry["trace"][1]["completed_frame_count"], 2);
    assert_eq!(entry["trace"][1]["input"]["orbit_horizontal_pixels"], 11.0);
    assert_eq!(entry["trace"][1]["input"]["pan_vertical_pixels"], -4.0);
    assert_eq!(entry["trace"][1]["input"]["zoom_lines"], 1.5);
}

fn run_smoke(source: &Path, index: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_renderer-demo"))
        .args(["--smoke"])
        .arg(source)
        .arg(index)
        .output()
        .unwrap()
}

fn run_elevation_smoke(source: &Path, index: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_renderer-demo"))
        .args(["--smoke", "--display", "elevation"])
        .arg(source)
        .arg(index)
        .output()
        .unwrap()
}

fn run_display_smoke(source: &Path, index: &Path, mode: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_renderer-demo"))
        .args(["--smoke", "--display", mode])
        .arg(source)
        .arg(index)
        .output()
        .unwrap()
}

fn run_corpus(manifest: &Path, report: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_renderer-demo"))
        .arg("corpus")
        .arg("--manifest")
        .arg(manifest)
        .arg("--report")
        .arg(report)
        .output()
        .unwrap()
}

fn write_corpus_manifest(
    path: &Path,
    source: &Path,
    index: &Path,
    initial_frame_count: u32,
    trace_frame_count: u32,
) -> io::Result<()> {
    let manifest = serde_json::json!({
        "schema": "punctra.renderer-demo.field-corpus.v1",
        "corpus_id": "local-process-fixture",
        "machine": {
            "label": "test-machine",
            "operating_system": std::env::consts::OS,
            "filesystem": "temporary-test-directory",
            "gpu_expectation": "local-adapter-or-explicit-skip"
        },
        "entries": [{
            "id": "opaque-entry",
            "project_id": "private-project-name",
            "firm_id": "private-firm-name",
            "source_path": source,
            "index_path": index,
            "inspect_permission": true,
            "measure_permission": true,
            "display": "neutral",
            "projection": "orthographic",
            "initial_frame_count": initial_frame_count,
            "trace": [{
                "orbit_horizontal_pixels": 11.0,
                "orbit_vertical_pixels": 7.0,
                "pan_horizontal_pixels": 3.0,
                "pan_vertical_pixels": -4.0,
                "zoom_lines": 1.5,
                "frame_count": trace_frame_count
            }]
        }]
    });
    let bytes = serde_json::to_vec(&manifest).map_err(io::Error::other)?;
    fs::write(path, bytes)
}

fn assert_smoke_output(output: &std::process::Output, disposition: &str) {
    assert!(output.status.success(), "{}", diagnostics(output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Verified Source (Full)"));
    assert!(stdout.contains(&format!("disposition: {disposition}")));
    assert!(stdout.contains("First accepted visible real-cloud batch"));
    assert!(stdout.contains("Headless bridge smoke accepted one atomic Upsert"));
}

fn write_las(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    write_las_with_format(path, Format::new(2)?, true)
}

fn write_las_without_rgb(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    write_las_with_format(path, Format::new(0)?, false)
}

fn write_las_with_format(
    path: &Path,
    point_format: Format,
    include_color: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = Builder::from((1, 4));
    builder.point_format = point_format;
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
    for offset in [0.0, 1.0, 2.0, 3.0] {
        writer.write_point(Point {
            x: 500_000.0 + offset,
            y: 4_600_000.0 + offset * 2.0,
            z: 120.0 + offset * 0.5,
            intensity: 32_768,
            return_number: 1,
            number_of_returns: 1,
            classification: Classification::Ground,
            color: include_color.then(|| Color::new(0, 32_768, u16::MAX)),
            ..Point::default()
        })?;
    }
    writer.close()?;
    Ok(())
}

fn diagnostics(output: &std::process::Output) -> String {
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
                "punctra-renderer-smoke-{}-{sequence}",
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
