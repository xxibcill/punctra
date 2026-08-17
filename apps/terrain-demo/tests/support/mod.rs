//! Shared generated fixtures for the recoverable workflow evidence.

#![allow(dead_code)]

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write as _},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use las::{
    Builder, Point, Transform, Vector, Vlr, Writer,
    point::{Classification, Format},
};
use serde_json::{Value, json};

const JOURNAL_HEADER_BYTES: usize = 80;
const FRAME_HEADER_BYTES: usize = 56;
const FRAME_HASH_BYTES: usize = 32;
const FRAME_PAYLOAD_BYTES_OFFSET: usize = 16;

pub struct TestDirectory(PathBuf);

/// Test-owned filesystem obstruction used to retain a retryable Workspace intent.
pub struct RevisionDirectoryBlocker {
    workspace: PathBuf,
    revisions: PathBuf,
    backup: PathBuf,
    restored: bool,
}

impl TestDirectory {
    pub fn new(label: &str) -> io::Result<Self> {
        static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
        loop {
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "punctra-terrain-workflow-{label}-{}-{sequence}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl RevisionDirectoryBlocker {
    pub fn install(workspace: &Path) -> io::Result<Self> {
        let revisions = workspace.join("revisions");
        let backup = workspace.join(".punctra-evidence-revisions-backup");
        fs::rename(&revisions, &backup)?;
        if let Err(error) = overwrite_and_sync(&revisions, b"block Revision publication") {
            let _ = fs::rename(&backup, &revisions);
            return Err(error);
        }
        Ok(Self {
            workspace: workspace.to_path_buf(),
            revisions,
            backup,
            restored: false,
        })
    }

    pub fn restore(mut self) -> io::Result<()> {
        self.restore_inner()
    }

    fn restore_inner(&mut self) -> io::Result<()> {
        fs::remove_file(&self.revisions)?;
        fs::rename(&self.backup, &self.revisions)?;
        File::open(&self.workspace)?.sync_all()?;
        self.restored = true;
        Ok(())
    }
}

impl Drop for RevisionDirectoryBlocker {
    fn drop(&mut self) {
        if !self.restored {
            let _ = self.restore_inner();
        }
    }
}

/// Writes equivalent generated LAS or LAZ meaning according to `path`'s extension.
pub fn write_las_family_fixture(path: &Path, point_count: usize) -> io::Result<()> {
    write_las_family_fixture_with_vlrs(path, point_count, Vec::new())
}

/// Writes a generated LAS or LAZ fixture with one complete metric projected profile.
pub fn write_las_family_fixture_with_profile(path: &Path, point_count: usize) -> io::Result<()> {
    let entries = [
        (1_024, 0, 1, 1),
        (3_072, 0, 1, 32_647),
        (3_076, 0, 1, 9_001),
        (4_096, 0, 1, 5_703),
        (4_099, 0, 1, 9_001),
    ];
    let count = u16::try_from(entries.len()).expect("generated GeoKey count fits u16");
    let data = [1, 1, 0, count]
        .into_iter()
        .chain(
            entries
                .into_iter()
                .flat_map(|(key, location, value_count, value)| {
                    [key, location, value_count, value]
                }),
        )
        .flat_map(u16::to_le_bytes)
        .collect();
    write_las_family_fixture_with_vlrs(
        path,
        point_count,
        vec![Vlr {
            user_id: "LASF_Projection".to_owned(),
            record_id: 34_735,
            description: "complete metric GeoKey directory".to_owned(),
            data,
        }],
    )
}

fn write_las_family_fixture_with_vlrs(
    path: &Path,
    point_count: usize,
    vlrs: Vec<Vlr>,
) -> io::Result<()> {
    let mut builder = Builder::from((1, 4));
    builder.point_format = Format::new(0).map_err(io::Error::other)?;
    builder.vlrs = vlrs;
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
    let header = builder.into_header().map_err(io::Error::other)?;
    let mut writer = Writer::from_path(path, header).map_err(io::Error::other)?;
    let width = integer_square_width(point_count);
    for ordinal in 0..point_count {
        let x = ordinal % width;
        let y = ordinal / width;
        let z_ticks = (x.saturating_mul(3) + y.saturating_mul(5)) % 17;
        writer
            .write_point(Point {
                x: 500_000.0 + exact_f64(x),
                y: 4_600_000.0 + exact_f64(y),
                z: 120.0 + exact_f64(z_ticks) / 10.0,
                return_number: 1,
                number_of_returns: 1,
                classification: Classification::Ground,
                ..Point::default()
            })
            .map_err(io::Error::other)?;
    }
    writer.close().map_err(io::Error::other)
}

/// Returns the byte end of every complete frame in a sealed v0.7 journal.
pub fn journal_frame_ends(bytes: &[u8]) -> io::Result<Vec<usize>> {
    if bytes.len() < JOURNAL_HEADER_BYTES || &bytes[..8] != b"PTWFJ001" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "workflow journal header is absent or invalid",
        ));
    }
    let mut cursor = JOURNAL_HEADER_BYTES;
    let mut ends = Vec::new();
    while cursor < bytes.len() {
        let minimum_end = cursor
            .checked_add(FRAME_HEADER_BYTES + FRAME_HASH_BYTES)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "frame offset overflow"))?;
        if minimum_end > bytes.len() || &bytes[cursor..cursor + 4] != b"PWF1" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "workflow journal has a truncated or invalid frame",
            ));
        }
        let length_offset = cursor + FRAME_PAYLOAD_BYTES_OFFSET;
        let payload_bytes = u32::from_le_bytes(
            bytes[length_offset..length_offset + 4]
                .try_into()
                .expect("validated fixed-width payload length"),
        );
        let end = minimum_end
            .checked_add(usize::try_from(payload_bytes).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "payload is not addressable")
            })?)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "frame size overflow"))?;
        if end > bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "workflow journal frame payload is truncated",
            ));
        }
        ends.push(end);
        cursor = end;
    }
    Ok(ends)
}

pub fn restore_journal_prefix(path: &Path, complete: &[u8], end: usize) -> io::Result<()> {
    let prefix = complete
        .get(..end)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid journal prefix"))?;
    let mut file = OpenOptions::new().write(true).truncate(true).open(path)?;
    file.write_all(prefix)?;
    file.sync_all()?;
    drop(file);
    sync_parent(path)
}

pub fn overwrite_and_sync(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    sync_parent(path)
}

/// Removes storage identities while retaining the coordinate-domain report meaning.
pub fn semantic_report_projection(bytes: &[u8]) -> serde_json::Result<Value> {
    let report: Value = serde_json::from_slice(bytes)?;
    Ok(json!({
        "schema": report["schema"],
        "edit": {
            "classification_after": report["edit"]["classification_after"],
            "ordinals": report["edit"]["ordinals"],
            "changed_point_count": report["edit"]["changed_point_count"],
            "footprint": report["edit"]["footprint"],
            "transitions": report["edit"]["transitions"],
        },
        "terrain": {
            "baseline": surface_projection(&report["terrain"]["baseline"]),
            "changed": surface_projection(&report["terrain"]["changed"]),
        },
        "surface_change_envelope": {
            "meaning": report["surface_change_envelope"]["meaning"],
            "added_face_count": report["surface_change_envelope"]["added_face_count"],
            "removed_face_count": report["surface_change_envelope"]["removed_face_count"],
            "bounds": report["surface_change_envelope"]["bounds"],
        },
        "qa": {
            "outcomes": report["qa"]["outcomes"],
            "statistics": report["qa"]["statistics"],
            "face_tests": report["qa"]["face_tests"],
            "accounted_peak_working_bytes": report["qa"]["accounted_peak_working_bytes"],
        },
        "landxml": {
            "outcome": report["landxml"]["outcome"],
            "content_hash": report["landxml"]["content_hash"],
            "byte_length": report["landxml"]["byte_length"],
            "vertex_count": report["landxml"]["vertex_count"],
            "face_count": report["landxml"]["face_count"],
        },
        "limits": report["limits"],
        "external_evidence": report["external_evidence"],
    }))
}

fn surface_projection(surface: &Value) -> Value {
    json!({
        "input_point_count": surface["input_point_count"],
        "vertex_count": surface["vertex_count"],
        "face_count": surface["face_count"],
        "hull_vertex_count": surface["hull_vertex_count"],
        "bounds": surface["bounds"],
        "accounted_peak_working_bytes": surface["accounted_peak_working_bytes"],
        "retained_surface_bytes": surface["retained_surface_bytes"],
        "topology_steps": surface["topology_steps"],
    })
}

fn sync_parent(path: &Path) -> io::Result<()> {
    File::open(path.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()
}

fn integer_square_width(point_count: usize) -> usize {
    let mut width = 1_usize;
    while width.saturating_mul(width) < point_count {
        width = width.saturating_add(1);
    }
    width
}

fn exact_f64(value: usize) -> f64 {
    let value = u32::try_from(value).expect("generated fixture coordinate fits u32");
    f64::from(value)
}
