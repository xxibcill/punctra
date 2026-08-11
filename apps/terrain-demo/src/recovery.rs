use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use point_index::PrepareLimits;
use point_workspace::{
    CommitLimits, CommitOutcome, CommitRejection, CommitRequest, OpenLimits, OperationId,
    OperationResolution, RevisionId, Workspace, WorkspaceId,
};

use crate::AppResult;

const RECOVERY_MAGIC: &[u8; 8] = b"PTRNREC1";
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

pub(crate) struct RecoveryPaths<'a> {
    source: &'a Path,
    index: &'a Path,
    workspace: &'a Path,
    record: PathBuf,
}

impl<'a> RecoveryPaths<'a> {
    pub(crate) fn new(source: &'a Path, index: &'a Path, workspace: &'a Path) -> Self {
        Self {
            source,
            index,
            workspace,
            record: recovery_record_path(workspace),
        }
    }

    pub(crate) fn has_record(&self) -> bool {
        self.record.exists()
    }

    pub(crate) fn require_workspace_for_record(&self) -> Result<(), io::Error> {
        if self.has_record() && !self.workspace.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "recovery record {} exists without its Workspace",
                    self.record.display()
                ),
            ));
        }
        Ok(())
    }
}

pub(crate) fn commit_with_recovery(
    workspace: Workspace,
    request: CommitRequest,
    action: &'static str,
    operation: OperationId,
    recovery: &RecoveryPaths<'_>,
) -> AppResult<(Workspace, RevisionId)> {
    save_recovery_record(
        &recovery.record,
        RecoveryRecord {
            workspace: workspace.identity(),
            operation,
        },
    )?;
    let outcome = workspace
        .commit(request, CommitLimits::default())
        .blocking_wait()?;
    match outcome {
        CommitOutcome::Committed(receipt) => {
            clear_recovery_record(&recovery.record)?;
            Ok((workspace, receipt.revision()))
        }
        CommitOutcome::Rejected(reason) => {
            clear_recovery_record(&recovery.record)?;
            Err(io::Error::other(format!(
                "{action} Operation {operation} was rejected: {reason:?}"
            ))
            .into())
        }
        CommitOutcome::Indeterminate(uncertainty) => {
            drop(workspace);
            let (workspace, recovered_operation, revision) = reconcile_recovery_record(recovery)?;
            if recovered_operation != operation {
                return Err(io::Error::other(format!(
                    "{action} recovery resolved Operation {recovered_operation}, expected {operation}"
                ))
                .into());
            }
            println!(
                "{action} acknowledgement was uncertain at {:?}; recovery resolved Revision {revision}",
                uncertainty.phase()
            );
            Ok((workspace, revision))
        }
    }
}

pub(crate) fn reconcile_recovery_record(
    paths: &RecoveryPaths<'_>,
) -> AppResult<(Workspace, OperationId, RevisionId)> {
    let record = load_recovery_record(&paths.record)?;
    let workspace = reopen_workspace(paths)?;
    if workspace.identity() != record.workspace {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery record belongs to another Workspace",
        )
        .into());
    }
    let recovered = recover_operation(&workspace, record.operation)?;
    clear_recovery_record(&paths.record)?;
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
    Ok((workspace, record.operation, revision))
}

fn recover_operation(
    workspace: &Workspace,
    operation: OperationId,
) -> AppResult<RecoveredOperation> {
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

fn reopen_workspace(paths: &RecoveryPaths<'_>) -> AppResult<Workspace> {
    let source = source_las::open(paths.source).blocking_wait()?;
    let index =
        point_index::prepare(source, paths.index, PrepareLimits::default()).blocking_wait()?;
    Ok(point_workspace::open(paths.workspace, index, OpenLimits::default()).blocking_wait()?)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_record_round_trips_and_detects_corruption() {
        let record = RecoveryRecord {
            workspace: WorkspaceId::from_bytes([1; 16]).expect("fixture Workspace ID is valid"),
            operation: OperationId::from_bytes([2; 16]).expect("fixture Operation ID is valid"),
        };
        let bytes = encode_recovery_record(record);
        assert_eq!(decode_recovery_record(&bytes).unwrap(), record);

        let mut corrupt = bytes;
        corrupt[12] ^= 0x80;
        assert!(decode_recovery_record(&corrupt).is_err());
    }

    #[test]
    fn recovery_record_sits_next_to_the_workspace() {
        assert_eq!(
            recovery_record_path(Path::new("survey.pcw")),
            Path::new("survey.pcw.recovery")
        );
    }
}
