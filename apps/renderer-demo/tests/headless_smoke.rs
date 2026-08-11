//! Process-level smoke coverage for the synthetic and verified LAS bridges.

use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use las::point::Format;
use las::{Builder, Point, Transform, Vector, Writer};

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
fn las_and_laz_headless_smoke_full_verify_build_open_and_upload_one_node() {
    let directory = TestDirectory::new().unwrap();
    for extension in ["las", "laz"] {
        let source = directory.path().join(format!("fixture.{extension}"));
        let index = directory.path().join(format!("fixture-{extension}.pidx"));
        write_las(&source).unwrap();

        let built = run_smoke(&source, &index);
        assert_smoke_output(&built, "Built");
        assert!(index.is_file());

        let opened = run_smoke(&source, &index);
        assert_smoke_output(&opened, "Opened");
    }
}

#[test]
fn default_bare_index_target_builds_in_the_working_directory() {
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
}

fn run_smoke(source: &Path, index: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_renderer-demo"))
        .args(["--smoke"])
        .arg(source)
        .arg(index)
        .output()
        .unwrap()
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
    for offset in [0.0, 1.0, 2.0, 3.0] {
        writer.write_point(Point {
            x: 500_000.0 + offset,
            y: 4_600_000.0 + offset * 2.0,
            z: 120.0 + offset * 0.5,
            return_number: 1,
            number_of_returns: 1,
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
