//! Classify an exact LAS/LAZ Point Set, append a Revert, and reopen the result.
//!
//! Before each commit, the example syncs the caller-owned Workspace and
//! Operation identities to a `WORKSPACE.pcw.recovery` sidecar. Rerunning the
//! same command reconciles that record before attempting any new work.

use std::{
    env,
    error::Error,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use point_contracts::AttributeId;
use point_index::{PrepareLimits, prepare};
use point_workspace::{
    CommitLimits, CommitOutcome, CommitRejection, CommitRequest, OpenLimits, OperationId,
    OperationResolution, PointQuery, PointSetLimits, RevisionId, Workspace, WorkspaceId,
    WorkspaceSchema, create, open,
};

const RECOVERY_MAGIC: &[u8; 8] = b"PCWOP001";
const RECOVERY_PAYLOAD_BYTES: usize = 8 + 16 + 16;
const RECOVERY_RECORD_BYTES: usize = RECOVERY_PAYLOAD_BYTES + 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RecoveryRecord {
    workspace: WorkspaceId,
    operation: OperationId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecoveredOperation {
    Committed(RevisionId),
    Rejected(CommitRejection),
    NotRecorded,
}

#[allow(
    clippy::too_many_lines,
    reason = "the example keeps the complete caller recovery flow linear and visible"
)]
fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args_os();
    let program = arguments.next().unwrap_or_default();
    let source_path = required_argument(&mut arguments, &program, "SOURCE.las|laz")?;
    let index_path = required_argument(&mut arguments, &program, "INDEX.pidx")?;
    let workspace_path = required_argument(&mut arguments, &program, "WORKSPACE.pcw")?;
    let classification_text = required_argument(&mut arguments, &program, "CLASSIFICATION_ID")?;
    if arguments.next().is_some() {
        return Err(usage(&program).into());
    }
    let recovery_path = recovery_record_path(&workspace_path);
    if workspace_path.exists() {
        let (operation, revision) =
            reconcile_recovery_record(&source_path, &index_path, &workspace_path, &recovery_path)?;
        println!("reconciled retained Operation {operation} as committed Revision {revision}");
        return Ok(());
    }
    if recovery_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "recovery record {} exists without its Workspace",
                recovery_path.display()
            ),
        )
        .into());
    }
    let classification = classification_text.to_string_lossy().parse::<u32>()?;
    let classification = AttributeId::new(classification)?;

    let source = source_las::open(&source_path).blocking_wait()?;
    let index = prepare(source, &index_path, PrepareLimits::default()).blocking_wait()?;
    let workspace = create(
        &workspace_path,
        index,
        WorkspaceSchema::new(classification),
        OpenLimits::default(),
    )
    .blocking_wait()?;
    let root = workspace.head();
    let ground = root
        .select(
            PointQuery::all().classification_is(2),
            PointSetLimits::default(),
        )
        .blocking_wait()?;
    if ground.metadata().exact_count() == 0 {
        return Err(io::Error::other("Source contains no class-2 Points to demonstrate").into());
    }
    println!(
        "selected {} exact class-2 Points at Revision {}",
        ground.metadata().exact_count(),
        root.provenance().revision()
    );

    let set_operation = OperationId::generate()?;
    save_recovery_record(
        &recovery_path,
        RecoveryRecord {
            workspace: workspace.identity(),
            operation: set_operation,
        },
    )?;
    let set_outcome = workspace
        .commit(
            CommitRequest::set_classification(set_operation, ground, 1),
            CommitLimits::default(),
        )
        .blocking_wait()?;
    let set_revision = match set_outcome {
        CommitOutcome::Committed(receipt) => {
            clear_recovery_record(&recovery_path)?;
            receipt.revision()
        }
        CommitOutcome::Rejected(reason) => {
            clear_recovery_record(&recovery_path)?;
            return Err(io::Error::other(format!(
                "classification Operation {set_operation} was rejected: {reason:?}"
            ))
            .into());
        }
        CommitOutcome::Indeterminate(uncertainty) => {
            drop(root);
            drop(workspace);
            let (operation, recovered) = reconcile_recovery_record(
                &source_path,
                &index_path,
                &workspace_path,
                &recovery_path,
            )?;
            debug_assert_eq!(operation, set_operation);
            println!(
                "classification acknowledgement was uncertain at {:?}; recovery resolved Revision {recovered}",
                uncertainty.phase()
            );
            return Ok(());
        }
    };

    let revert_operation = OperationId::generate()?;
    save_recovery_record(
        &recovery_path,
        RecoveryRecord {
            workspace: workspace.identity(),
            operation: revert_operation,
        },
    )?;
    let revert_outcome = workspace
        .commit(
            CommitRequest::revert_head(revert_operation, set_revision),
            CommitLimits::default(),
        )
        .blocking_wait()?;
    let revert_revision = match revert_outcome {
        CommitOutcome::Committed(receipt) => {
            clear_recovery_record(&recovery_path)?;
            receipt.revision()
        }
        CommitOutcome::Rejected(reason) => {
            clear_recovery_record(&recovery_path)?;
            return Err(io::Error::other(format!(
                "Revert Operation {revert_operation} was rejected: {reason:?}"
            ))
            .into());
        }
        CommitOutcome::Indeterminate(uncertainty) => {
            drop(root);
            drop(workspace);
            let (operation, recovered) = reconcile_recovery_record(
                &source_path,
                &index_path,
                &workspace_path,
                &recovery_path,
            )?;
            debug_assert_eq!(operation, revert_operation);
            println!(
                "Revert acknowledgement was uncertain at {:?}; recovery resolved Revision {recovered}",
                uncertainty.phase()
            );
            return Ok(());
        }
    };

    drop(root);
    drop(workspace);
    let reopened = reopen_workspace(&source_path, &index_path, &workspace_path)?;
    if reopened.head().provenance().revision() != revert_revision {
        return Err(io::Error::other("reopened head differs from the committed Revert").into());
    }
    println!(
        "committed classification Revision {set_revision}, Revert Revision {revert_revision}, and reopened the complete Workspace"
    );
    Ok(())
}

fn reconcile_recovery_record(
    source_path: &Path,
    index_path: &Path,
    workspace_path: &Path,
    recovery_path: &Path,
) -> Result<(OperationId, RevisionId), Box<dyn Error>> {
    let record = load_recovery_record(recovery_path)?;
    let workspace = reopen_workspace(source_path, index_path, workspace_path)?;
    if workspace.identity() != record.workspace {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery record belongs to another Workspace",
        )
        .into());
    }
    let recovered = recover_operation(&workspace, record.operation)?;
    clear_recovery_record(recovery_path)?;
    let revision = match recovered {
        RecoveredOperation::Committed(revision) => revision,
        RecoveredOperation::Rejected(reason) => {
            return Err(io::Error::other(format!(
                "recorded Operation {} was rejected: {reason:?}",
                record.operation
            ))
            .into());
        }
        RecoveredOperation::NotRecorded => {
            return Err(io::Error::other(format!(
                "recorded Operation {} definitely published no durable state",
                record.operation
            ))
            .into());
        }
    };
    Ok((record.operation, revision))
}

fn recover_operation(
    workspace: &Workspace,
    operation: OperationId,
) -> Result<RecoveredOperation, Box<dyn Error>> {
    match workspace.resolve_operation(operation)? {
        OperationResolution::Committed(receipt) => {
            Ok(RecoveredOperation::Committed(receipt.revision()))
        }
        OperationResolution::Rejected(rejection) => {
            Ok(RecoveredOperation::Rejected(rejection.reason()))
        }
        OperationResolution::Retryable(intent) => {
            let expected = intent.revision();
            match workspace
                .retry_operation(operation, CommitLimits::default())
                .blocking_wait()?
            {
                CommitOutcome::Committed(receipt) if receipt.revision() == expected => {
                    Ok(RecoveredOperation::Committed(receipt.revision()))
                }
                CommitOutcome::Rejected(reason) => Ok(RecoveredOperation::Rejected(reason)),
                outcome => Err(io::Error::other(format!(
                    "retry of Operation {operation} did not resolve its recorded intent: {outcome:?}"
                ))
                .into()),
            }
        }
        OperationResolution::NotRecorded => Ok(RecoveredOperation::NotRecorded),
        OperationResolution::Indeterminate(uncertainty) => Err(io::Error::other(format!(
            "Operation {operation} remains indeterminate at {:?}: {}",
            uncertainty.phase(),
            uncertainty.reason()
        ))
        .into()),
    }
}

fn recovery_record_path(workspace_path: &Path) -> PathBuf {
    let mut path = workspace_path.as_os_str().to_os_string();
    path.push(".recovery");
    PathBuf::from(path)
}

fn save_recovery_record(path: &Path, record: RecoveryRecord) -> Result<(), io::Error> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&encode_recovery_record(record))?;
    file.sync_all()?;
    drop(file);
    sync_parent(path)
}

fn load_recovery_record(path: &Path) -> Result<RecoveryRecord, io::Error> {
    let mut file = File::open(path)?;
    let actual_bytes = file.metadata()?.len();
    if actual_bytes != RECOVERY_RECORD_BYTES as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("recovery record has {actual_bytes} bytes, expected {RECOVERY_RECORD_BYTES}"),
        ));
    }
    let mut bytes = [0_u8; RECOVERY_RECORD_BYTES];
    file.read_exact(&mut bytes)?;
    decode_recovery_record(&bytes)
}

fn clear_recovery_record(path: &Path) -> Result<(), io::Error> {
    fs::remove_file(path)?;
    sync_parent(path)
}

fn sync_parent(path: &Path) -> Result<(), io::Error> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    File::open(parent)?.sync_all()
}

fn encode_recovery_record(record: RecoveryRecord) -> [u8; RECOVERY_RECORD_BYTES] {
    let mut bytes = [0_u8; RECOVERY_RECORD_BYTES];
    bytes[..8].copy_from_slice(RECOVERY_MAGIC);
    bytes[8..24].copy_from_slice(record.workspace.as_bytes());
    bytes[24..RECOVERY_PAYLOAD_BYTES].copy_from_slice(record.operation.as_bytes());
    let checksum = blake3::hash(&bytes[..RECOVERY_PAYLOAD_BYTES]);
    bytes[RECOVERY_PAYLOAD_BYTES..].copy_from_slice(checksum.as_bytes());
    bytes
}

fn decode_recovery_record(
    bytes: &[u8; RECOVERY_RECORD_BYTES],
) -> Result<RecoveryRecord, io::Error> {
    let expected_checksum = blake3::hash(&bytes[..RECOVERY_PAYLOAD_BYTES]);
    if &bytes[..8] != RECOVERY_MAGIC
        || bytes[RECOVERY_PAYLOAD_BYTES..] != *expected_checksum.as_bytes()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery record is incompatible or corrupt",
        ));
    }
    let workspace = WorkspaceId::try_from_slice(&bytes[8..24])
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let operation = OperationId::try_from_slice(&bytes[24..RECOVERY_PAYLOAD_BYTES])
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(RecoveryRecord {
        workspace,
        operation,
    })
}

fn reopen_workspace(
    source_path: &Path,
    index_path: &Path,
    workspace_path: &Path,
) -> Result<Workspace, Box<dyn Error>> {
    let source = source_las::open(source_path).blocking_wait()?;
    let index = prepare(source, index_path, PrepareLimits::default()).blocking_wait()?;
    Ok(open(workspace_path, index, OpenLimits::default()).blocking_wait()?)
}

fn required_argument(
    arguments: &mut impl Iterator<Item = std::ffi::OsString>,
    program: &std::ffi::OsStr,
    _name: &str,
) -> Result<std::path::PathBuf, io::Error> {
    arguments
        .next()
        .map(std::path::PathBuf::from)
        .ok_or_else(|| usage(program))
}

fn usage(program: &std::ffi::OsStr) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "usage: {} SOURCE.las|laz INDEX.pidx WORKSPACE.pcw CLASSIFICATION_ID",
            Path::new(program).display()
        ),
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use point_workspace::{OperationId, WorkspaceId};

    use super::{
        RecoveryRecord, decode_recovery_record, encode_recovery_record, recovery_record_path,
    };

    #[test]
    fn recovery_record_round_trips_and_detects_corruption() {
        let record = RecoveryRecord {
            workspace: WorkspaceId::from_bytes([1; 16]).unwrap(),
            operation: OperationId::from_bytes([2; 16]).unwrap(),
        };
        let bytes = encode_recovery_record(record);
        assert_eq!(decode_recovery_record(&bytes).unwrap(), record);

        let mut changed = bytes;
        changed[12] ^= 0x80;
        assert!(decode_recovery_record(&changed).is_err());
    }

    #[test]
    fn recovery_record_sits_next_to_the_workspace() {
        assert_eq!(
            recovery_record_path(Path::new("survey.pcw")),
            Path::new("survey.pcw.recovery")
        );
    }
}
