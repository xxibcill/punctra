//! Classify an exact LAS/LAZ Point Set, append a Revert, and reopen the result.

use std::{env, error::Error, io, path::Path};

use point_contracts::AttributeId;
use point_index::{PrepareLimits, prepare};
use point_workspace::{
    CommitLimits, CommitOutcome, CommitRequest, OpenLimits, OperationId, OperationResolution,
    PointQuery, PointSetLimits, RevisionId, Workspace, WorkspaceSchema, create, open,
};

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
    if workspace_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "example requires a new Workspace path; {} already exists",
                workspace_path.display()
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
    let set_outcome = workspace
        .commit(
            CommitRequest::set_classification(set_operation, ground, 1),
            CommitLimits::default(),
        )
        .blocking_wait()?;
    let set_revision = match set_outcome {
        CommitOutcome::Committed(receipt) => receipt.revision(),
        CommitOutcome::Rejected(reason) => {
            return Err(io::Error::other(format!(
                "classification Operation {set_operation} was rejected: {reason:?}"
            ))
            .into());
        }
        CommitOutcome::Indeterminate(uncertainty) => {
            drop(root);
            drop(workspace);
            let recovered =
                recover_operation(&source_path, &index_path, &workspace_path, set_operation)?;
            println!(
                "classification acknowledgement was uncertain at {:?}; recovery resolved Revision {recovered}",
                uncertainty.phase()
            );
            return Ok(());
        }
    };

    let revert_operation = OperationId::generate()?;
    let revert_outcome = workspace
        .commit(
            CommitRequest::revert_head(revert_operation, set_revision),
            CommitLimits::default(),
        )
        .blocking_wait()?;
    let revert_revision = match revert_outcome {
        CommitOutcome::Committed(receipt) => receipt.revision(),
        CommitOutcome::Rejected(reason) => {
            return Err(io::Error::other(format!(
                "Revert Operation {revert_operation} was rejected: {reason:?}"
            ))
            .into());
        }
        CommitOutcome::Indeterminate(uncertainty) => {
            drop(root);
            drop(workspace);
            let recovered =
                recover_operation(&source_path, &index_path, &workspace_path, revert_operation)?;
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

fn recover_operation(
    source_path: &Path,
    index_path: &Path,
    workspace_path: &Path,
    operation: OperationId,
) -> Result<RevisionId, Box<dyn Error>> {
    let workspace = reopen_workspace(source_path, index_path, workspace_path)?;
    match workspace.resolve_operation(operation)? {
        OperationResolution::Committed(receipt) => Ok(receipt.revision()),
        OperationResolution::Retryable(intent) => {
            let expected = intent.revision();
            match workspace
                .retry_operation(operation, CommitLimits::default())
                .blocking_wait()?
            {
                CommitOutcome::Committed(receipt) if receipt.revision() == expected => {
                    Ok(receipt.revision())
                }
                outcome => Err(io::Error::other(format!(
                    "retry of Operation {operation} did not resolve its recorded intent: {outcome:?}"
                ))
                .into()),
            }
        }
        resolution => Err(io::Error::other(format!(
            "Operation {operation} was not recoverably committed: {resolution:?}"
        ))
        .into()),
    }
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
