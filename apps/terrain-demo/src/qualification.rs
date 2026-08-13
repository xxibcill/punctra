//! Strictly read-only qualification facts for a durably Complete Workflow Run.

// This intentionally private seam binds canonical evidence to one exact,
// durably Complete Run without turning qualification into a public API.
use std::{
    fs::{self, File},
    io::{self, Read as _},
    path::{Path, PathBuf},
};

use blake3::Hasher;
use thiserror::Error;

use crate::{
    journal::{self, CompleteJournalSnapshot, Digest, JournalError, JournalLimits, WorkflowRunId},
    publication::same_file_identity,
    report::REPORT_HASH_DOMAIN,
    workflow::{PATH_BINDING_BYTES, ReadOnlyRunGuard},
};

const ARTIFACT_HASH_BUFFER_BYTES: usize = 8 * 1024;
const MAX_CAPTURED_AUDIT_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QualificationBoundary {
    Journal,
    LandXml,
    Audit,
    Reference,
}

trait QualificationHook {
    fn reach(&self, boundary: QualificationBoundary) -> io::Result<()>;
}

struct ProductionQualificationHook;

impl QualificationHook for ProductionQualificationHook {
    fn reach(&self, _boundary: QualificationBoundary) -> io::Result<()> {
        Ok(())
    }
}

/// Detached immutable facts qualified from one exact Complete journal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CompleteRunQualificationSnapshot {
    pub(crate) run: WorkflowRunId,
    pub(crate) request_hash: Digest,
    pub(crate) terminal_journal_hash: Digest,
    pub(crate) journal_bytes: u64,
    pub(crate) source: Digest,
    pub(crate) workspace: [u8; 16],
    pub(crate) baseline_revision: Digest,
    pub(crate) operation: [u8; 16],
    pub(crate) revision: Digest,
    pub(crate) audit_hash: Digest,
    pub(crate) surface_hash: Digest,
    pub(crate) qa_hash: Digest,
    pub(crate) landxml_hash: Digest,
    pub(crate) landxml_bytes: u64,
    pub(crate) report_hash: Digest,
    pub(crate) report_bytes: u64,
    pub(crate) options_hash: Digest,
    pub(crate) path_bindings: [Digest; 4],
}

/// A Complete-Run snapshot whose existing shared Run lock remains held.
pub(crate) struct CompleteRunQualification {
    requested_run_root: PathBuf,
    canonical_run_root: PathBuf,
    guard: ReadOnlyRunGuard,
    journal: CompleteJournalSnapshot,
    snapshot: CompleteRunQualificationSnapshot,
}

/// Exact immutable Run artifacts checked against the Complete journal.
pub(crate) struct QualifiedRunArtifacts {
    pub(crate) audit_json: Vec<u8>,
    landxml: StableArtifactWitness,
    audit: StableArtifactWitness,
}

impl QualifiedRunArtifacts {
    /// Revalidates both exact artifact path bindings while their files remain open.
    pub(crate) fn verify(&self) -> Result<(), QualificationError> {
        self.landxml.verify()?;
        self.audit.verify()
    }
}

/// Failure to obtain a stable, read-only Complete-Run qualification snapshot.
#[derive(Debug, Error)]
pub(crate) enum QualificationError {
    #[error("failed to {operation} {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    Journal(#[from] JournalError),
    #[error("Complete Run qualification conflict: {0}")]
    Conflict(&'static str),
    #[error("Complete Run artifact {artifact} differs from its journal fact")]
    ArtifactConflict { artifact: &'static str },
    #[error("Complete Run artifact {artifact} exceeds the qualification capture limit")]
    ArtifactResource { artifact: &'static str },
}

impl QualificationError {
    fn io(operation: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.to_path_buf(),
            source,
        }
    }
}

impl CompleteRunQualification {
    pub(crate) fn acquire(
        run_root: &Path,
        limits: JournalLimits,
    ) -> Result<Self, QualificationError> {
        Self::acquire_with_hook(run_root, limits, &ProductionQualificationHook)
    }

    fn acquire_with_hook(
        run_root: &Path,
        limits: JournalLimits,
        hook: &impl QualificationHook,
    ) -> Result<Self, QualificationError> {
        let guard = ReadOnlyRunGuard::acquire(run_root).map_err(|source| {
            QualificationError::io("lock existing Run read-only", run_root, source)
        })?;
        guard.verify().map_err(|source| {
            QualificationError::io("verify read-only Run binding", run_root, source)
        })?;
        let canonical_run_root = guard.canonical_root().to_path_buf();
        hook.reach(QualificationBoundary::Journal)
            .map_err(|source| {
                QualificationError::io("enter journal qualification boundary", run_root, source)
            })?;
        guard.verify().map_err(|source| {
            QualificationError::io(
                "revalidate Run before journal qualification",
                run_root,
                source,
            )
        })?;
        let journal = CompleteJournalSnapshot::open(&canonical_run_root.join("run.pwf"), limits)?;
        guard.verify().map_err(|source| {
            QualificationError::io(
                "revalidate Run after journal qualification",
                run_root,
                source,
            )
        })?;
        let run_root_binding = journal::bind_path(run_root, PATH_BINDING_BYTES)?;
        if journal.intent().path_bindings[3] != run_root_binding {
            return Err(QualificationError::Conflict(
                "journal Run-root binding differs from the qualified path",
            ));
        }
        let snapshot = qualification_snapshot(&journal);
        let qualification = Self {
            requested_run_root: run_root.to_path_buf(),
            canonical_run_root,
            guard,
            journal,
            snapshot,
        };
        qualification.verify()?;
        Ok(qualification)
    }

    pub(crate) fn snapshot(&self) -> &CompleteRunQualificationSnapshot {
        &self.snapshot
    }

    pub(crate) fn verify(&self) -> Result<(), QualificationError> {
        self.guard.verify().map_err(|source| {
            QualificationError::io(
                "revalidate read-only Run binding",
                &self.requested_run_root,
                source,
            )
        })?;
        self.journal.verify()?;
        Ok(())
    }

    pub(crate) fn verify_artifacts(&self) -> Result<QualifiedRunArtifacts, QualificationError> {
        self.verify_artifacts_with_hook(&ProductionQualificationHook)
    }

    fn verify_artifacts_with_hook(
        &self,
        hook: &impl QualificationHook,
    ) -> Result<QualifiedRunArtifacts, QualificationError> {
        self.verify()?;
        hook.reach(QualificationBoundary::LandXml)
            .map_err(|source| {
                QualificationError::io(
                    "enter LandXML qualification boundary",
                    &self.requested_run_root,
                    source,
                )
            })?;
        self.verify()?;
        let landxml = read_stable_artifact(
            &self.canonical_run_root.join("terrain.xml"),
            self.snapshot.landxml_bytes,
            None,
            None,
        )?;
        if landxml.hash != self.snapshot.landxml_hash {
            return Err(QualificationError::ArtifactConflict {
                artifact: "terrain.xml",
            });
        }
        self.verify()?;
        hook.reach(QualificationBoundary::Audit).map_err(|source| {
            QualificationError::io(
                "enter audit qualification boundary",
                &self.requested_run_root,
                source,
            )
        })?;
        self.verify()?;
        let audit = read_stable_artifact(
            &self.canonical_run_root.join("audit.json"),
            self.snapshot.report_bytes,
            Some(REPORT_HASH_DOMAIN),
            Some(MAX_CAPTURED_AUDIT_BYTES),
        )?;
        if audit.hash != self.snapshot.report_hash {
            return Err(QualificationError::ArtifactConflict {
                artifact: "audit.json",
            });
        }
        let artifacts = QualifiedRunArtifacts {
            audit_json: audit.bytes,
            landxml: landxml.witness,
            audit: audit.witness,
        };
        artifacts.verify()?;
        self.verify()?;
        Ok(artifacts)
    }

    pub(crate) fn landxml_path(&self) -> PathBuf {
        self.canonical_run_root.join("terrain.xml")
    }

    pub(crate) fn verify_reference(&self) -> Result<PathBuf, QualificationError> {
        self.verify_reference_with_hook(&ProductionQualificationHook)
    }

    fn verify_reference_with_hook(
        &self,
        hook: &impl QualificationHook,
    ) -> Result<PathBuf, QualificationError> {
        hook.reach(QualificationBoundary::Reference)
            .map_err(|source| {
                QualificationError::io(
                    "enter LandXML reference boundary",
                    &self.requested_run_root,
                    source,
                )
            })?;
        self.verify()?;
        Ok(self.landxml_path())
    }

    pub(crate) fn run_root(&self) -> &Path {
        &self.canonical_run_root
    }
}

/// Qualifies one existing Complete Run without creating, repairing, or writing files.
#[cfg(test)]
pub(crate) fn snapshot_complete_run(
    run_root: &Path,
    limits: JournalLimits,
) -> Result<CompleteRunQualificationSnapshot, QualificationError> {
    let qualification = CompleteRunQualification::acquire(run_root, limits)?;
    Ok(qualification.snapshot)
}

fn qualification_snapshot(journal: &CompleteJournalSnapshot) -> CompleteRunQualificationSnapshot {
    let intent = journal.intent();
    let revision = journal.revision();
    let export = journal.export();
    let report = journal.report();
    let complete = journal.complete();
    CompleteRunQualificationSnapshot {
        run: intent.run,
        request_hash: intent.request_hash,
        terminal_journal_hash: journal.journal_hash(),
        journal_bytes: journal.byte_length(),
        source: intent.source,
        workspace: intent.workspace,
        baseline_revision: intent.baseline_revision,
        operation: intent.operation,
        revision: revision.revision,
        audit_hash: complete.audit_hash,
        surface_hash: complete.surface_hash,
        qa_hash: complete.qa_hash,
        landxml_hash: export.content_hash,
        landxml_bytes: export.byte_length,
        report_hash: report.report_hash,
        report_bytes: report.byte_length,
        options_hash: intent.options_hash,
        path_bindings: intent.path_bindings,
    }
}

struct StableArtifact {
    hash: Digest,
    bytes: Vec<u8>,
    witness: StableArtifactWitness,
}

struct StableArtifactWitness {
    artifact: &'static str,
    path: PathBuf,
    file: File,
    identity: fs::Metadata,
    expected_bytes: u64,
}

impl StableArtifactWitness {
    fn verify(&self) -> Result<(), QualificationError> {
        let opened = self.file.metadata().map_err(|source| {
            QualificationError::io("reinspect open Run artifact", &self.path, source)
        })?;
        let target = fs::symlink_metadata(&self.path).map_err(|source| {
            QualificationError::io("reinspect Run artifact path", &self.path, source)
        })?;
        if opened.file_type().is_file()
            && target.file_type().is_file()
            && opened.len() == self.expected_bytes
            && target.len() == self.expected_bytes
            && same_file_state(&self.identity, &opened)
            && same_file_state(&opened, &target)
        {
            Ok(())
        } else {
            Err(QualificationError::ArtifactConflict {
                artifact: self.artifact,
            })
        }
    }
}

fn read_stable_artifact(
    path: &Path,
    expected_bytes: u64,
    hash_domain: Option<&[u8]>,
    capture_limit: Option<u64>,
) -> Result<StableArtifact, QualificationError> {
    let initial = fs::symlink_metadata(path)
        .map_err(|source| QualificationError::io("inspect Run artifact", path, source))?;
    if !initial.file_type().is_file() || initial.len() != expected_bytes {
        return Err(QualificationError::ArtifactConflict {
            artifact: artifact_name(path),
        });
    }
    let mut file = File::open(path)
        .map_err(|source| QualificationError::io("open Run artifact read-only", path, source))?;
    let opened = file
        .metadata()
        .map_err(|source| QualificationError::io("inspect open Run artifact", path, source))?;
    if !same_file_state(&initial, &opened) {
        return Err(QualificationError::ArtifactConflict {
            artifact: artifact_name(path),
        });
    }
    if capture_limit.is_some_and(|limit| expected_bytes > limit) {
        return Err(QualificationError::ArtifactResource {
            artifact: artifact_name(path),
        });
    }
    let capacity = if capture_limit.is_some() {
        usize::try_from(expected_bytes).map_err(|_| QualificationError::ArtifactResource {
            artifact: artifact_name(path),
        })?
    } else {
        0
    };
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(capacity)
        .map_err(|_| QualificationError::ArtifactResource {
            artifact: artifact_name(path),
        })?;
    let mut hasher = Hasher::new();
    if let Some(domain) = hash_domain {
        hasher.update(domain);
    }
    let mut buffer = [0_u8; ARTIFACT_HASH_BUFFER_BYTES];
    let mut bytes_read = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| QualificationError::io("read Run artifact", path, source))?;
        if read == 0 {
            break;
        }
        bytes_read = bytes_read.saturating_add(read as u64);
        if bytes_read > expected_bytes {
            return Err(QualificationError::ArtifactConflict {
                artifact: artifact_name(path),
            });
        }
        if capture_limit.is_some() && bytes.len().saturating_add(read) > capacity {
            return Err(QualificationError::ArtifactConflict {
                artifact: artifact_name(path),
            });
        }
        hasher.update(&buffer[..read]);
        if capture_limit.is_some() {
            bytes.extend_from_slice(&buffer[..read]);
        }
    }
    let final_opened = file
        .metadata()
        .map_err(|source| QualificationError::io("reinspect Run artifact", path, source))?;
    let final_path = fs::symlink_metadata(path)
        .map_err(|source| QualificationError::io("reinspect Run artifact path", path, source))?;
    if bytes_read != expected_bytes
        || (capture_limit.is_some() && bytes.len() as u64 != expected_bytes)
        || !same_file_state(&opened, &final_opened)
        || !same_file_state(&final_opened, &final_path)
        || final_path.len() != expected_bytes
    {
        return Err(QualificationError::ArtifactConflict {
            artifact: artifact_name(path),
        });
    }
    Ok(StableArtifact {
        hash: *hasher.finalize().as_bytes(),
        bytes,
        witness: StableArtifactWitness {
            artifact: artifact_name(path),
            path: path.to_path_buf(),
            file,
            identity: final_opened,
            expected_bytes,
        },
    })
}

#[cfg(unix)]
fn same_file_state(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;

    same_file_identity(left, right)
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}

#[cfg(windows)]
fn same_file_state(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;

    same_file_identity(left, right)
        && left.len() == right.len()
        && left.creation_time() == right.creation_time()
        && left.last_write_time() == right.last_write_time()
}

#[cfg(not(any(unix, windows)))]
fn same_file_state(_left: &fs::Metadata, _right: &fs::Metadata) -> bool {
    false
}

fn artifact_name(path: &Path) -> &'static str {
    match path.file_name().and_then(|value| value.to_str()) {
        Some("terrain.xml") => "terrain.xml",
        Some("audit.json") => "audit.json",
        _ => "Run artifact",
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::*;
    use crate::journal::{
        AuditObserved, Checkpoint, Complete, ExportEnsured, IntentCheckPoint, Journal, QaObserved,
        ReportEnsured, RevisionResolved, SurfaceObserved, WorkflowIntent,
    };

    #[test]
    fn snapshot_qualifies_complete_facts_without_changing_run_files() {
        let directory = Directory::new("complete");
        let run_root = directory.path.join("run");
        fs::create_dir(&run_root).unwrap();
        fs::File::create(run_root.join("run.lock")).unwrap();
        let expected = write_complete_journal(&run_root);
        #[cfg(unix)]
        make_run_files_read_only(&run_root);
        let before_entries = entries(&run_root);
        let before_journal = fs::read(run_root.join("run.pwf")).unwrap();

        let snapshot = snapshot_complete_run(&run_root, JournalLimits::default()).unwrap();

        assert_eq!(snapshot, expected);
        assert_eq!(entries(&run_root), before_entries);
        assert_eq!(fs::read(run_root.join("run.pwf")).unwrap(), before_journal);
        assert_eq!(fs::metadata(run_root.join("run.lock")).unwrap().len(), 0);
    }

    #[test]
    fn snapshot_does_not_create_a_missing_run_lock() {
        let directory = Directory::new("missing-lock");
        let run_root = directory.path.join("run");
        fs::create_dir(&run_root).unwrap();
        let _ = write_complete_journal(&run_root);
        let before_entries = entries(&run_root);

        let error = snapshot_complete_run(&run_root, JournalLimits::default())
            .expect_err("a qualification snapshot requires an existing Run lock");

        assert!(matches!(
            error,
            QualificationError::Io { source, .. }
                if source.kind() == io::ErrorKind::NotFound
        ));
        assert_eq!(entries(&run_root), before_entries);
        assert!(!run_root.join("run.lock").exists());
    }

    #[test]
    fn artifact_witness_rejects_same_bytes_at_a_replaced_path() {
        let directory = Directory::new("replaced-artifact");
        let path = directory.path.join("terrain.xml");
        fs::write(&path, b"abc").unwrap();
        let artifact = read_stable_artifact(&path, 3, None, None).unwrap();
        fs::rename(&path, directory.path.join("original.xml")).unwrap();
        fs::write(&path, b"abc").unwrap();

        assert!(matches!(
            artifact.witness.verify(),
            Err(QualificationError::ArtifactConflict {
                artifact: "terrain.xml"
            })
        ));
    }

    #[test]
    fn artifact_witness_rejects_same_length_in_place_changes() {
        let directory = Directory::new("changed-artifact");
        let path = directory.path.join("terrain.xml");
        fs::write(&path, b"abc").unwrap();
        let artifact = read_stable_artifact(&path, 3, None, None).unwrap();
        fs::write(&path, b"xyz").unwrap();

        assert!(matches!(
            artifact.witness.verify(),
            Err(QualificationError::ArtifactConflict {
                artifact: "terrain.xml"
            })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn qualification_revalidates_the_locked_complete_journal() {
        use std::io::Write as _;

        let directory = Directory::new("changed-journal");
        let run_root = directory.path.join("run");
        fs::create_dir(&run_root).unwrap();
        fs::File::create(run_root.join("run.lock")).unwrap();
        let _ = write_complete_journal(&run_root);
        let qualification =
            CompleteRunQualification::acquire(&run_root, JournalLimits::default()).unwrap();

        fs::OpenOptions::new()
            .append(true)
            .open(run_root.join("run.pwf"))
            .unwrap()
            .write_all(b"x")
            .unwrap();

        assert!(matches!(
            qualification.verify(),
            Err(QualificationError::Journal(JournalError::Io { .. }))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn qualification_rejects_ancestor_retarget_at_every_run_read_boundary() {
        use std::os::unix::fs::symlink;

        for (index, boundary) in [
            QualificationBoundary::Journal,
            QualificationBoundary::LandXml,
            QualificationBoundary::Audit,
            QualificationBoundary::Reference,
        ]
        .into_iter()
        .enumerate()
        {
            let directory = Directory::new(match index {
                0 => "retarget-journal",
                1 => "retarget-landxml",
                2 => "retarget-audit",
                _ => "retarget-reference",
            });
            let run_a = directory.path.join("run-a");
            let run_b = directory.path.join("run-b");
            let alias = directory.path.join("run-alias");
            fs::create_dir(&run_a).unwrap();
            fs::File::create(run_a.join("run.lock")).unwrap();
            symlink(&run_a, &alias).unwrap();
            let terrain_bytes = vec![0; 100];
            let audit_bytes = vec![0; 200];
            let landxml_hash = *blake3::hash(&terrain_bytes).as_bytes();
            let mut report_hasher = Hasher::new();
            report_hasher.update(REPORT_HASH_DOMAIN);
            report_hasher.update(&audit_bytes);
            let report_hash = *report_hasher.finalize().as_bytes();
            let _ = write_complete_journal_bound(&run_a, &alias, landxml_hash, report_hash);
            fs::write(run_a.join("terrain.xml"), terrain_bytes).unwrap();
            fs::write(run_a.join("audit.json"), audit_bytes).unwrap();
            fs::create_dir(&run_b).unwrap();
            let hook = RetargetRunHook {
                boundary,
                alias: &alias,
                replacement: &run_b,
            };

            let error = if boundary == QualificationBoundary::Journal {
                match CompleteRunQualification::acquire_with_hook(
                    &alias,
                    JournalLimits::default(),
                    &hook,
                ) {
                    Ok(_) => panic!("journal-boundary retarget must fail"),
                    Err(error) => error,
                }
            } else {
                let qualification =
                    CompleteRunQualification::acquire(&alias, JournalLimits::default()).unwrap();
                match boundary {
                    QualificationBoundary::LandXml | QualificationBoundary::Audit => {
                        match qualification.verify_artifacts_with_hook(&hook) {
                            Ok(_) => panic!("artifact-boundary retarget must fail"),
                            Err(error) => error,
                        }
                    }
                    QualificationBoundary::Reference => qualification
                        .verify_reference_with_hook(&hook)
                        .expect_err("reference-boundary retarget must fail"),
                    QualificationBoundary::Journal => unreachable!(),
                }
            };

            assert!(
                matches!(error, QualificationError::Io { .. }),
                "boundary {boundary:?} returned {error:?}"
            );
        }
    }

    #[cfg(unix)]
    struct RetargetRunHook<'a> {
        boundary: QualificationBoundary,
        alias: &'a Path,
        replacement: &'a Path,
    }

    #[cfg(unix)]
    impl QualificationHook for RetargetRunHook<'_> {
        fn reach(&self, boundary: QualificationBoundary) -> io::Result<()> {
            if self.boundary == boundary {
                fs::remove_file(self.alias)?;
                std::os::unix::fs::symlink(self.replacement, self.alias)?;
            }
            Ok(())
        }
    }

    #[test]
    fn relocated_v1_golden_run_binding_fails_closed_without_mutation() {
        let directory = Directory::new("relocated-golden");
        let run_root = directory.path.join("run");
        fs::create_dir(&run_root).unwrap();
        for (name, bytes) in [
            (
                "run.pwf",
                &include_bytes!("../tests/fixtures/run-v1/complete/run.pwf")[..],
            ),
            (
                "terrain.xml",
                &include_bytes!("../tests/fixtures/run-v1/complete/terrain.xml")[..],
            ),
            (
                "audit.json",
                &include_bytes!("../tests/fixtures/run-v1/complete/audit.json")[..],
            ),
            (
                "run.lock",
                &include_bytes!("../tests/fixtures/run-v1/complete/run.lock")[..],
            ),
        ] {
            fs::write(run_root.join(name), bytes).unwrap();
        }
        let before = entries_with_bytes(&run_root);

        let Err(error) = CompleteRunQualification::acquire(&run_root, JournalLimits::default())
        else {
            panic!("v1 raw-path binding must reject a relocated checked fixture");
        };

        assert!(matches!(
            error,
            QualificationError::Conflict(
                "journal Run-root binding differs from the qualified path"
            )
        ));
        assert_eq!(entries_with_bytes(&run_root), before);
    }

    #[allow(clippy::too_many_lines)]
    fn write_complete_journal(run_root: &Path) -> CompleteRunQualificationSnapshot {
        write_complete_journal_bound(run_root, run_root, [14; 32], [15; 32])
    }

    #[allow(clippy::too_many_lines)]
    fn write_complete_journal_bound(
        run_root: &Path,
        bound_run_root: &Path,
        landxml: [u8; 32],
        report: [u8; 32],
    ) -> CompleteRunQualificationSnapshot {
        let run = WorkflowRunId::new([1; 16]).unwrap();
        let path_bindings = [
            [6; 32],
            [7; 32],
            [8; 32],
            journal::bind_path(bound_run_root, PATH_BINDING_BYTES).unwrap(),
        ];
        let intent = WorkflowIntent::new(
            run,
            [2; 32],
            [3; 16],
            [4; 32],
            [5; 16],
            vec![2, 7].into_boxed_slice(),
            2,
            1,
            None,
            vec![IntentCheckPoint {
                id: 1,
                position_bits: [1.0_f64.to_bits(), 2.0_f64.to_bits(), 3.0_f64.to_bits()],
            }]
            .into_boxed_slice(),
            "Ground".into(),
            "2026-08-10".into(),
            "00:00:00Z".into(),
            true,
            path_bindings,
            JournalLimits::default(),
        )
        .unwrap();
        let revision = [10; 32];
        let audit = [11; 32];
        let surface = [12; 32];
        let qa = [13; 32];
        let checkpoints = [
            Checkpoint::RevisionResolved(RevisionResolved {
                operation: intent.operation,
                revision,
                parent: intent.baseline_revision,
                sequence: 1,
                kind: 1,
            }),
            Checkpoint::AuditObserved(AuditObserved {
                revision,
                content_hash: audit,
                point_id_hash: [16; 32],
                changed_points: 2,
                transition_count: 1,
                footprint_bits: Some([[1, 2], [3, 4], [5, 6]]),
            }),
            Checkpoint::SurfaceObserved(SurfaceObserved {
                revision,
                recipe_hash: intent.recipe_hash,
                baseline_artifact_hash: [17; 32],
                changed_artifact_hash: surface,
                baseline_geometry_hash: [18; 32],
                changed_geometry_hash: [19; 32],
                baseline_topology_hash: [20; 32],
                changed_topology_hash: [21; 32],
                baseline_vertex_count: 5,
                baseline_face_count: 4,
                changed_vertex_count: 3,
                changed_face_count: 1,
                added_face_count: 1,
                removed_face_count: 4,
                added_face_hash: [22; 32],
                removed_face_hash: [23; 32],
                envelope_bits: Some([[1, 2], [3, 4], [5, 6]]),
            }),
            Checkpoint::QaObserved(QaObserved {
                surface_artifact_hash: surface,
                result_hash: qa,
                covered_count: 1,
                gap_count: 0,
                face_tests: 1,
                accounted_peak_working_bytes: 64,
                statistic_bits: [0; 4],
                statistic_mask: 15,
            }),
            Checkpoint::ExportEnsured(ExportEnsured {
                revision,
                surface_artifact_hash: surface,
                options_hash: intent.options_hash,
                target_binding: path_bindings[3],
                content_hash: landxml,
                byte_length: 100,
                outcome: 1,
            }),
            Checkpoint::ReportEnsured(ReportEnsured {
                report_hash: report,
                byte_length: 200,
                revision,
                audit_hash: audit,
                surface_hash: surface,
                qa_hash: qa,
                landxml_hash: landxml,
            }),
            Checkpoint::Complete(Complete {
                request_hash: intent.request_hash,
                revision,
                audit_hash: audit,
                surface_hash: surface,
                qa_hash: qa,
                landxml_hash: landxml,
                report_hash: report,
            }),
        ];
        let journal_path = run_root.join("run.pwf");
        let mut journal =
            Journal::create(&journal_path, intent.clone(), JournalLimits::default()).unwrap();
        for checkpoint in checkpoints {
            journal.record(checkpoint).unwrap();
        }
        let terminal_journal_hash = journal.terminal_hash();
        drop(journal);
        CompleteRunQualificationSnapshot {
            run,
            request_hash: intent.request_hash,
            terminal_journal_hash,
            journal_bytes: fs::metadata(journal_path).unwrap().len(),
            source: intent.source,
            workspace: intent.workspace,
            baseline_revision: intent.baseline_revision,
            operation: intent.operation,
            revision,
            audit_hash: audit,
            surface_hash: surface,
            qa_hash: qa,
            landxml_hash: landxml,
            landxml_bytes: 100,
            report_hash: report,
            report_bytes: 200,
            options_hash: intent.options_hash,
            path_bindings,
        }
    }

    fn entries(path: &Path) -> Vec<PathBuf> {
        let mut entries = fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        entries
    }

    fn entries_with_bytes(path: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        entries(path)
            .into_iter()
            .map(|entry| {
                let bytes = fs::read(&entry).unwrap();
                (entry, bytes)
            })
            .collect()
    }

    #[cfg(unix)]
    fn make_run_files_read_only(run_root: &Path) {
        use std::os::unix::fs::PermissionsExt as _;

        for path in [run_root.join("run.lock"), run_root.join("run.pwf")] {
            let mut permissions = fs::metadata(&path).unwrap().permissions();
            permissions.set_mode(0o444);
            fs::set_permissions(path, permissions).unwrap();
        }
    }

    struct Directory {
        path: PathBuf,
    }

    impl Directory {
        fn new(label: &str) -> Self {
            let mut random = [0; 8];
            getrandom::fill(&mut random).unwrap();
            let path = std::env::temp_dir().join(format!(
                "punctra-terrain-qualification-{label}-{}-{}",
                std::process::id(),
                u64::from_le_bytes(random)
            ));
            fs::create_dir(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.path).unwrap();
        }
    }
}
