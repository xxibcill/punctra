use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::task::{Context, Poll};

use blake3::Hasher;
use foundation_runtime::{Job, OperationControl, OperationHandle};
use point_contracts::{AttributeId, ContentHash, PointId, SourceId};
use point_index::PreparedIndex;
use point_source::Source;

use crate::error::{WorkspaceDiagnostic, WorkspaceError};
use crate::limits::{CommitLimits, OpenLimits, PointIdReadLimits, PointSetLimits, require};
use crate::model::{
    CommitOutcome, CommitPhase, CommitReceipt, CommitRejection, CommitRequest, CommitRequestKind,
    CommitUncertainty, OperationId, OperationResolution, PointSetMetadata, RecordedIntent,
    RecordedRejection, RevisionId, RevisionInfo, RevisionKind, SnapshotProvenance, WorkspaceId,
    WorkspaceSchema,
};
pub(crate) use crate::persistence::OverlayUsage;
use crate::persistence::{
    CandidateFacts, Catalog, CatalogLimits, ClassificationRequestFacts, MANIFEST_BYTES,
    ManifestFacts, OperationRecord, OverlayLimits, PersistedPointSetFacts, PersistenceError,
    REJECTION_BYTES, ReadLimits, RejectionFacts, RevisionKind as PersistedRevisionKind,
    RevisionRow, RowReadLimits, SealedRevision, Store, ValidatedRevision, WriteLimits,
    classification_request_digest as persisted_classification_request_digest,
    revert_request_digest as persisted_revert_request_digest,
};
use crate::point_set::{PointSetRecord, PointSetRecordBatches};

const SOURCE_CONTRACT_DOMAIN: &[u8] = b"punctra-workspace-source-contract-v1";
const ROOT_REVISION_DOMAIN: &[u8] = b"punctra-workspace-root-revision-v1";

/// Background creation or reopen of one complete Workspace session.
pub type WorkspaceJob = Job<Workspace, WorkspaceError>;

/// Creates a new durable Workspace over one complete index and verified Source.
#[must_use]
pub fn create(
    root: impl AsRef<Path>,
    index: PreparedIndex,
    schema: WorkspaceSchema,
    limits: OpenLimits,
) -> WorkspaceJob {
    let root = root.as_ref().to_path_buf();
    Job::spawn(move |control| create_workspace(&root, index, schema, limits, &control))
}

/// Reopens one existing durable Workspace against the same verified Source.
#[must_use]
pub fn open(root: impl AsRef<Path>, index: PreparedIndex, limits: OpenLimits) -> WorkspaceJob {
    let root = root.as_ref().to_path_buf();
    Job::spawn(move |control| open_workspace(&root, index, limits, &control))
}

/// Exclusive local session over one durable Workspace.
#[derive(Clone)]
pub struct Workspace {
    session: Arc<Session>,
}

/// Cancellation-aware commit whose panic mapping preserves publication certainty.
pub struct CommitJob {
    operation: OperationId,
    phase: Arc<PublicationPhase>,
    session: Arc<Session>,
    inner: Job<CommitOutcome, WorkspaceError>,
}

impl Workspace {
    /// Returns the stable Workspace lineage identity.
    #[must_use]
    pub fn identity(&self) -> WorkspaceId {
        self.session.identity
    }

    /// Returns the immutable Source identity.
    #[must_use]
    pub fn source(&self) -> SourceId {
        self.session.source().identity()
    }

    /// Returns a Snapshot pinned to the current immutable head Revision.
    #[must_use]
    pub fn head(&self) -> Snapshot {
        let revision = self
            .session
            .catalog
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .head();
        self.session.snapshot(revision)
    }

    /// Returns a Snapshot pinned to one known immutable Revision.
    ///
    /// # Errors
    ///
    /// Returns an unknown-Revision or poisoned-session error.
    pub fn snapshot(&self, revision: RevisionId) -> Result<Snapshot, WorkspaceError> {
        self.session.require_revision(revision)?;
        Ok(self.session.snapshot(revision.into_bytes()))
    }

    /// Returns durable facts for one known immutable Revision.
    ///
    /// # Errors
    ///
    /// Returns an unknown-Revision or poisoned-session error.
    pub fn revision_info(&self, revision: RevisionId) -> Result<RevisionInfo, WorkspaceError> {
        self.session.revision_info(revision)
    }

    /// Starts one durable classification or immediate-head Revert operation.
    #[must_use]
    pub fn commit(&self, request: CommitRequest, limits: CommitLimits) -> CommitJob {
        let operation = request.operation();
        CommitJob::spawn(
            Arc::clone(&self.session),
            operation,
            move |session, phase, control| run_commit(&session, request, limits, &phase, &control),
        )
    }

    /// Retries one complete durable ready payload without a live Point Set.
    #[must_use]
    pub fn retry_operation(&self, operation: OperationId, limits: CommitLimits) -> CommitJob {
        CommitJob::spawn(
            Arc::clone(&self.session),
            operation,
            move |session, phase, control| run_retry(&session, operation, limits, &phase, &control),
        )
    }

    /// Reconciles one retained Operation Identity against durable records.
    ///
    /// # Errors
    ///
    /// Returns a lock, corruption, or poisoned-session error. Durability that
    /// cannot be established is represented by `OperationResolution::Indeterminate`.
    pub fn resolve_operation(
        &self,
        operation: OperationId,
    ) -> Result<OperationResolution, WorkspaceError> {
        resolve_operation(&self.session, operation)
    }
}

impl std::fmt::Debug for Workspace {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Workspace")
            .field("identity", &self.identity())
            .field("source", &self.source())
            .field("head", &self.head().provenance().revision())
            .finish_non_exhaustive()
    }
}

impl CommitJob {
    fn spawn<F>(session: Arc<Session>, operation: OperationId, work: F) -> Self
    where
        F: FnOnce(
                Arc<Session>,
                Arc<PublicationPhase>,
                OperationControl,
            ) -> Result<CommitOutcome, WorkspaceError>
            + Send
            + 'static,
    {
        let phase = Arc::new(PublicationPhase::new());
        let worker_session = Arc::clone(&session);
        let worker_phase = Arc::clone(&phase);
        let inner = Job::spawn(move |control| {
            let mut certainty = MutationCertaintyGuard::new(
                Arc::clone(&worker_session.poisoned),
                Arc::clone(&worker_phase),
            );
            let result = work(worker_session, worker_phase, control);
            certainty.observe(&result);
            result
        });
        Self {
            operation,
            phase,
            session,
            inner,
        }
    }

    /// Returns a cloneable runtime observation and cancellation capability.
    #[must_use]
    pub fn handle(&self) -> OperationHandle {
        self.inner.handle()
    }

    /// Waits for a certainty-preserving terminal commit result.
    ///
    /// # Errors
    ///
    /// Returns only failures known to precede durable publication. A failure
    /// after publication begins becomes `CommitOutcome::Indeterminate`.
    pub fn blocking_wait(self) -> Result<CommitOutcome, WorkspaceError> {
        let result = self.inner.blocking_wait();
        finish_commit_result(
            self.operation,
            self.phase.as_ref(),
            self.session.as_ref(),
            result,
        )
    }
}

impl Future for CommitJob {
    type Output = Result<CommitOutcome, WorkspaceError>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.inner).poll(context) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => Poll::Ready(finish_commit_result(
                self.operation,
                self.phase.as_ref(),
                self.session.as_ref(),
                result,
            )),
        }
    }
}

impl std::fmt::Debug for CommitJob {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CommitJob")
            .field("operation", &self.operation)
            .field("phase", &self.phase.current())
            .field("handle", &self.inner.handle())
            .finish_non_exhaustive()
    }
}

struct PublicationPhase(AtomicU8);

struct MutationCertaintyGuard {
    poisoned: Arc<AtomicBool>,
    phase: Arc<PublicationPhase>,
    armed: bool,
    force_poison: bool,
}

impl MutationCertaintyGuard {
    fn new(poisoned: Arc<AtomicBool>, phase: Arc<PublicationPhase>) -> Self {
        Self {
            poisoned,
            phase,
            armed: true,
            force_poison: false,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }

    fn force_poison(&mut self) {
        self.force_poison = true;
    }

    fn observe(&mut self, result: &Result<CommitOutcome, WorkspaceError>) {
        match result {
            Ok(CommitOutcome::Committed(_) | CommitOutcome::Rejected(_)) => self.disarm(),
            Ok(CommitOutcome::Indeterminate(_)) => self.force_poison(),
            Err(_) => {}
        }
    }
}

impl Drop for MutationCertaintyGuard {
    fn drop(&mut self) {
        if self.armed && (self.force_poison || self.phase.current().is_some()) {
            self.poisoned.store(true, Ordering::SeqCst);
        }
    }
}

impl PublicationPhase {
    const NONE: u8 = 0;
    const OPERATION: u8 = 1;
    const REVISION: u8 = 2;
    const REVISION_SYNC: u8 = 3;

    const fn new() -> Self {
        Self(AtomicU8::new(Self::NONE))
    }

    fn mark_operation(&self) {
        self.0.fetch_max(Self::OPERATION, Ordering::SeqCst);
    }

    fn mark_revision(&self) {
        self.0.fetch_max(Self::REVISION, Ordering::SeqCst);
    }

    fn mark_revision_sync(&self) {
        self.0.fetch_max(Self::REVISION_SYNC, Ordering::SeqCst);
    }

    fn current(&self) -> Option<CommitPhase> {
        match self.0.load(Ordering::SeqCst) {
            Self::NONE => None,
            Self::OPERATION => Some(CommitPhase::OperationPublication),
            Self::REVISION => Some(CommitPhase::RevisionPublication),
            _ => Some(CommitPhase::RevisionDirectorySync),
        }
    }
}

fn begin_publication(
    control: &OperationControl,
    phase: &PublicationPhase,
    revision: bool,
) -> Result<(), PersistenceError> {
    if control.check_cancelled().is_err() {
        return Err(PersistenceError::Cancelled);
    }
    if revision {
        phase.mark_revision();
    } else {
        phase.mark_operation();
    }
    Ok(())
}

fn finish_commit_result(
    operation: OperationId,
    phase: &PublicationPhase,
    session: &Session,
    result: Result<CommitOutcome, WorkspaceError>,
) -> Result<CommitOutcome, WorkspaceError> {
    finish_commit_result_with_poison(operation, phase, session.poisoned.as_ref(), result)
}

fn finish_commit_result_with_poison(
    operation: OperationId,
    phase: &PublicationPhase,
    poisoned: &AtomicBool,
    result: Result<CommitOutcome, WorkspaceError>,
) -> Result<CommitOutcome, WorkspaceError> {
    match result {
        Ok(CommitOutcome::Indeterminate(uncertainty)) => {
            poisoned.store(true, Ordering::SeqCst);
            Ok(CommitOutcome::Indeterminate(uncertainty))
        }
        Ok(outcome) => Ok(outcome),
        Err(error) => {
            let Some(commit_phase) = phase.current() else {
                return Err(error);
            };
            poisoned.store(true, Ordering::SeqCst);
            Ok(CommitOutcome::Indeterminate(CommitUncertainty::new(
                operation,
                commit_phase,
                error.to_string(),
            )))
        }
    }
}

/// Read-only view pinned to one immutable Workspace Revision.
#[derive(Clone)]
pub struct Snapshot {
    session: Arc<Session>,
    provenance: SnapshotProvenance,
}

impl Snapshot {
    /// Returns the exact Workspace, Source, and Revision provenance.
    #[must_use]
    pub const fn provenance(&self) -> &SnapshotProvenance {
        &self.provenance
    }

    /// Starts exact materialization of one supported Query at this Revision.
    #[must_use]
    pub fn select(&self, query: crate::PointQuery, limits: PointSetLimits) -> crate::PointSetJob {
        crate::selection::select(self, query, limits)
    }

    /// Starts exact materialization of bounded explicit Point Identities.
    ///
    /// Input is consumed under `limits` before the background Source read is
    /// started, so the iterator itself does not need to be `Send` or `'static`.
    #[must_use]
    pub fn select_point_ids(
        &self,
        ids: impl IntoIterator<Item = PointId>,
        limits: PointSetLimits,
    ) -> crate::PointSetJob {
        crate::selection::select_point_ids(self, ids, limits)
    }

    pub(crate) fn session(&self) -> Arc<Session> {
        Arc::clone(&self.session)
    }
}

impl std::fmt::Debug for Snapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Snapshot")
            .field("provenance", &self.provenance)
            .finish_non_exhaustive()
    }
}

pub(crate) struct Session {
    identity: WorkspaceId,
    schema: WorkspaceSchema,
    index: PreparedIndex,
    manifest: ManifestFacts,
    open_limits: OpenLimits,
    store: Store,
    catalog: RwLock<Catalog>,
    pub(crate) writer: Mutex<()>,
    poisoned: Arc<AtomicBool>,
    #[cfg(test)]
    writer_waiters: std::sync::atomic::AtomicUsize,
}

impl Session {
    fn ensure_mutable(&self) -> Result<(), WorkspaceError> {
        if self.poisoned.load(Ordering::SeqCst) {
            return Err(WorkspaceError::Poisoned);
        }
        Ok(())
    }

    pub(crate) const fn index(&self) -> &PreparedIndex {
        &self.index
    }

    pub(crate) fn source(&self) -> &Source {
        self.index.source()
    }

    pub(crate) const fn classification_attribute(&self) -> AttributeId {
        self.schema.classification()
    }

    pub(crate) fn scratch_path(&self) -> &Path {
        self.store.scratch()
    }

    pub(crate) fn apply_overlays(
        &self,
        revision: RevisionId,
        first_ordinal: u64,
        values: &mut [u8],
        limits: PointSetLimits,
        usage: &mut OverlayUsage,
        control: &OperationControl,
    ) -> Result<(), WorkspaceError> {
        control.check_cancelled()?;
        let overlay_limits = OverlayLimits {
            max_blocks: limits.max_overlay_segments(),
            max_payload_bytes: limits.max_overlay_bytes(),
            max_block_bytes: limits.max_working_bytes().min(limits.max_overlay_bytes()),
        };
        self.catalog
            .read()
            .map_err(|_| WorkspaceError::Poisoned)?
            .apply_overlays(
                revision.into_bytes(),
                first_ordinal,
                values,
                overlay_limits,
                usage,
                control,
            )
            .map_err(map_persistence)?;
        control.check_cancelled()?;
        Ok(())
    }

    fn snapshot(self: &Arc<Self>, revision: [u8; 32]) -> Snapshot {
        let revision = RevisionId::from_bytes(revision)
            .expect("validated persisted Revision identities are nonzero");
        Snapshot {
            session: Arc::clone(self),
            provenance: SnapshotProvenance::new(self.identity, self.source().identity(), revision),
        }
    }

    fn require_revision(&self, revision: RevisionId) -> Result<(), WorkspaceError> {
        let revision_bytes = revision.into_bytes();
        if revision_bytes == self.manifest.root_revision {
            return Ok(());
        }
        let catalog = self.catalog.read().map_err(|_| WorkspaceError::Poisoned)?;
        if catalog.revision(revision_bytes).is_none() {
            return Err(WorkspaceError::UnknownRevision { revision });
        }
        Ok(())
    }

    fn revision_info(&self, revision: RevisionId) -> Result<RevisionInfo, WorkspaceError> {
        self.require_revision(revision)?;
        if revision.into_bytes() == self.manifest.root_revision {
            return Ok(RevisionInfo::new(
                revision,
                None,
                0,
                None,
                RevisionKind::Root,
            ));
        }
        let catalog = self.catalog.read().map_err(|_| WorkspaceError::Poisoned)?;
        let persisted = catalog
            .revision(revision.into_bytes())
            .ok_or(WorkspaceError::UnknownRevision { revision })?;
        revision_info_from_persisted(persisted)
    }
}

fn run_commit(
    session: &Arc<Session>,
    request: CommitRequest,
    limits: CommitLimits,
    phase: &Arc<PublicationPhase>,
    control: &OperationControl,
) -> Result<CommitOutcome, WorkspaceError> {
    session.ensure_mutable()?;
    control.check_cancelled()?;
    #[cfg(test)]
    session.writer_waiters.fetch_add(1, Ordering::SeqCst);
    let writer = session.writer.lock();
    #[cfg(test)]
    session.writer_waiters.fetch_sub(1, Ordering::SeqCst);
    let _writer = writer.map_err(|_| WorkspaceError::Poisoned)?;
    let mut certainty =
        MutationCertaintyGuard::new(Arc::clone(&session.poisoned), Arc::clone(phase));
    let result = (|| {
        session.ensure_mutable()?;
        let (operation, kind) = request.into_parts();
        match kind {
            CommitRequestKind::SetClassification { points, value } => {
                commit_classification(session, operation, &points, value, limits, phase, control)
            }
            CommitRequestKind::Revert { expected_head } => {
                commit_revert(session, operation, expected_head, limits, phase, control)
            }
        }
    })();
    certainty.observe(&result);
    result
}

fn run_retry(
    session: &Arc<Session>,
    operation: OperationId,
    limits: CommitLimits,
    phase: &Arc<PublicationPhase>,
    control: &OperationControl,
) -> Result<CommitOutcome, WorkspaceError> {
    session.ensure_mutable()?;
    control.check_cancelled()?;
    #[cfg(test)]
    session.writer_waiters.fetch_add(1, Ordering::SeqCst);
    let writer = session.writer.lock();
    #[cfg(test)]
    session.writer_waiters.fetch_sub(1, Ordering::SeqCst);
    let _writer = writer.map_err(|_| WorkspaceError::Poisoned)?;
    let mut certainty =
        MutationCertaintyGuard::new(Arc::clone(&session.poisoned), Arc::clone(phase));
    let result = (|| {
        session.ensure_mutable()?;
        let (committed, record) = operation_state(session, operation)?;
        if let Some(committed) = committed {
            if let Some(outcome) = commit_sync_uncertainty(session, operation, false) {
                return Ok(outcome);
            }
            return Ok(CommitOutcome::Committed(receipt_from_revision(&committed)?));
        }
        let Some(record) = record else {
            return Err(WorkspaceError::OperationNotRetryable { operation });
        };
        if let Some(rejection) = record.rejection {
            if let Some(outcome) = commit_sync_uncertainty(session, operation, true) {
                return Ok(outcome);
            }
            return Ok(CommitOutcome::Rejected(rejection_reason(rejection)?));
        }
        let ready = record
            .ready
            .ok_or(WorkspaceError::OperationNotRetryable { operation })?;
        if let Some(outcome) = commit_sync_uncertainty(session, operation, true) {
            return Ok(outcome);
        }
        enforce_ready_limits(&ready, limits)?;
        commit_ready(session, &ready, limits, 0, phase, control)
    })();
    certainty.observe(&result);
    result
}

fn commit_classification(
    session: &Arc<Session>,
    operation: OperationId,
    points: &crate::PointSet,
    value: u8,
    limits: CommitLimits,
    phase: &PublicationPhase,
    control: &OperationControl,
) -> Result<CommitOutcome, WorkspaceError> {
    let metadata = *points.commit_metadata();
    let request_digest = classification_request_digest(session, metadata, value);
    if let Some(outcome) =
        existing_operation(session, operation, request_digest, limits, phase, control)?
    {
        return Ok(outcome);
    }

    let provenance = metadata.provenance();
    if provenance.workspace() != session.identity
        || provenance.source() != session.source().identity()
    {
        return record_rejection(
            session,
            operation,
            request_digest,
            CommitRejection::ForeignPointSet,
            limits,
            phase,
            control,
        );
    }
    let expected_head = provenance.revision();
    let actual_head = current_head(session)?;
    if expected_head != actual_head {
        return record_rejection(
            session,
            operation,
            request_digest,
            CommitRejection::StaleHead {
                expected: expected_head,
                actual: actual_head,
            },
            limits,
            phase,
            control,
        );
    }
    require(
        metadata.exact_count(),
        limits.max_selected_points(),
        "selected Points",
    )?;
    let (read_limits, write_limits) = classification_commit_budgets(metadata, limits)?;
    let batches = points.records(read_limits)?;
    let rows = PointSetRows::new(batches, value, limits, control);
    let candidate = CandidateFacts {
        workspace: session.identity.into_bytes(),
        source: session.source().identity().into_bytes(),
        source_contract: session.manifest.source_contract,
        operation: operation.into_bytes(),
        request_digest,
        parent: expected_head.into_bytes(),
        sequence: next_sequence(session)?,
        kind: PersistedRevisionKind::SetClassification(value),
        point_set: Some(PersistedPointSetFacts {
            exact_count: metadata.exact_count(),
            point_id_hash: metadata.point_id_hash().into_bytes(),
            content_hash: metadata.content_hash().into_bytes(),
        }),
    };
    let sealed = match session
        .store
        .seal_candidate(candidate, rows, write_limits, control)
    {
        Ok(sealed) => sealed,
        Err(PersistenceError::NoRows) => {
            return record_rejection(
                session,
                operation,
                request_digest,
                CommitRejection::NoChanges,
                limits,
                phase,
                control,
            );
        }
        Err(error) => return Err(map_persistence(error)),
    };
    publish_and_commit(session, sealed, limits, phase, control)
}

fn commit_revert(
    session: &Arc<Session>,
    operation: OperationId,
    expected_head: RevisionId,
    limits: CommitLimits,
    phase: &PublicationPhase,
    control: &OperationControl,
) -> Result<CommitOutcome, WorkspaceError> {
    let request_digest = revert_request_digest(session, expected_head);
    if let Some(outcome) =
        existing_operation(session, operation, request_digest, limits, phase, control)?
    {
        return Ok(outcome);
    }
    let actual_head = current_head(session)?;
    if expected_head != actual_head {
        return record_rejection(
            session,
            operation,
            request_digest,
            CommitRejection::StaleHead {
                expected: expected_head,
                actual: actual_head,
            },
            limits,
            phase,
            control,
        );
    }
    if expected_head.into_bytes() == session.manifest.root_revision {
        return record_rejection(
            session,
            operation,
            request_digest,
            CommitRejection::RootCannotBeReverted,
            limits,
            phase,
            control,
        );
    }
    let head = session
        .catalog
        .read()
        .map_err(|_| WorkspaceError::Poisoned)?
        .revision(expected_head.into_bytes())
        .cloned()
        .ok_or(WorkspaceError::UnknownRevision {
            revision: expected_head,
        })?;
    require(
        head.row_count(),
        limits.max_changed_points(),
        "changed Points",
    )?;
    let (row_read_limits, write_limits) = revert_commit_budgets(&head, limits)?;
    let source_rows = head
        .rows(row_read_limits, control)
        .map_err(map_persistence)?
        .map(|row| {
            row.map(|row| RevisionRow {
                ordinal: row.ordinal,
                before: row.after,
                after: row.before,
            })
        });
    let candidate = CandidateFacts {
        workspace: session.identity.into_bytes(),
        source: session.source().identity().into_bytes(),
        source_contract: session.manifest.source_contract,
        operation: operation.into_bytes(),
        request_digest,
        parent: expected_head.into_bytes(),
        sequence: next_sequence(session)?,
        kind: PersistedRevisionKind::Revert,
        point_set: None,
    };
    let sealed = session
        .store
        .seal_candidate(candidate, source_rows, write_limits, control)
        .map_err(map_persistence)?;
    publish_and_commit(session, sealed, limits, phase, control)
}

fn publish_and_commit(
    session: &Arc<Session>,
    sealed: SealedRevision,
    limits: CommitLimits,
    phase: &PublicationPhase,
    control: &OperationControl,
) -> Result<CommitOutcome, WorkspaceError> {
    require_durable_growth(session, sealed.file_bytes(), limits)?;
    let path_budget = remaining_transition_budget(
        limits.max_working_bytes(),
        sealed.retained_working_bytes(),
        "ready path with sealed Revision retained",
    )?;
    let prepared = session
        .store
        .prepare_ready(&sealed, path_budget)
        .map_err(map_persistence)?;
    let catalog_budget = remaining_transition_budget(
        limits.max_working_bytes(),
        sealed
            .retained_working_bytes()
            .saturating_add(prepared.retained_working_bytes()),
        "ready catalog growth with sealed Revision retained",
    )?;
    let operation_catalog_growth = session
        .catalog
        .write()
        .map_err(|_| WorkspaceError::Poisoned)?
        .reserve_operation(sealed.operation(), catalog_budget)
        .map_err(map_persistence)?;
    control.check_cancelled()?;
    let ready = session
        .store
        .publish_prepared_ready(&sealed, prepared, || {
            begin_publication(control, phase, false)
        })
        .map_err(map_persistence)?;
    drop(sealed);
    session
        .catalog
        .write()
        .map_err(|_| WorkspaceError::Poisoned)?
        .record_ready(Arc::clone(&ready));
    commit_ready(
        session,
        &ready,
        limits,
        operation_catalog_growth,
        phase,
        control,
    )
}

fn commit_ready(
    session: &Arc<Session>,
    ready: &ValidatedRevision,
    limits: CommitLimits,
    retained_catalog_growth: u64,
    phase: &PublicationPhase,
    control: &OperationControl,
) -> Result<CommitOutcome, WorkspaceError> {
    let expected = RevisionId::from_bytes(ready.parent())?;
    let actual = current_head(session)?;
    if expected != actual {
        return record_rejection(
            session,
            OperationId::from_bytes(ready.operation())?,
            ready.request_digest(),
            CommitRejection::StaleHead { expected, actual },
            limits,
            phase,
            control,
        );
    }
    if ready.sequence() != next_sequence(session)? {
        return Err(WorkspaceError::corrupt(
            "ready payload sequence is not the next linear Revision",
        ));
    }
    enforce_ready_limits(ready, limits)?;
    require_durable_growth(session, 0, limits)?;
    let path_budget = remaining_transition_budget(
        limits.max_working_bytes(),
        ready
            .retained_working_bytes()
            .saturating_add(retained_catalog_growth),
        "Revision path with ready and catalog growth retained",
    )?;
    let prepared = session
        .store
        .prepare_revision(ready, path_budget)
        .map_err(map_persistence)?;
    let catalog_budget = remaining_transition_budget(
        limits.max_working_bytes(),
        ready
            .retained_working_bytes()
            .saturating_add(prepared.retained_working_bytes())
            .saturating_add(retained_catalog_growth),
        "Revision catalog growth with ready payload retained",
    )?;
    session
        .catalog
        .write()
        .map_err(|_| WorkspaceError::Poisoned)?
        .reserve_revision(catalog_budget)
        .map_err(map_persistence)?;
    control.check_cancelled()?;
    let committed = session
        .store
        .publish_prepared_revision(
            ready,
            prepared,
            || begin_publication(control, phase, true),
            || phase.mark_revision_sync(),
        )
        .map_err(map_persistence)?;
    let receipt = receipt_from_revision(&committed)?;
    session
        .catalog
        .write()
        .map_err(|_| WorkspaceError::Poisoned)?
        .append_committed(committed);
    Ok(CommitOutcome::Committed(receipt))
}

fn existing_operation(
    session: &Arc<Session>,
    operation: OperationId,
    request_digest: [u8; 32],
    limits: CommitLimits,
    phase: &PublicationPhase,
    control: &OperationControl,
) -> Result<Option<CommitOutcome>, WorkspaceError> {
    let (committed, record) = operation_state(session, operation)?;
    if let Some(committed) = committed {
        if committed.request_digest() != request_digest {
            return Ok(Some(CommitOutcome::Rejected(
                CommitRejection::OperationConflict,
            )));
        }
        if let Some(outcome) = commit_sync_uncertainty(session, operation, false) {
            return Ok(Some(outcome));
        }
        return Ok(Some(CommitOutcome::Committed(receipt_from_revision(
            &committed,
        )?)));
    }
    let Some(record) = record else {
        return Ok(None);
    };
    if let Some(rejection) = record.rejection {
        if rejection.request_digest != request_digest {
            return Ok(Some(CommitOutcome::Rejected(
                CommitRejection::OperationConflict,
            )));
        }
        if let Some(outcome) = commit_sync_uncertainty(session, operation, true) {
            return Ok(Some(outcome));
        }
        return Ok(Some(CommitOutcome::Rejected(rejection_reason(rejection)?)));
    }
    let Some(ready) = record.ready else {
        return Ok(None);
    };
    if ready.request_digest() != request_digest {
        return Ok(Some(CommitOutcome::Rejected(
            CommitRejection::OperationConflict,
        )));
    }
    if let Some(outcome) = commit_sync_uncertainty(session, operation, true) {
        return Ok(Some(outcome));
    }
    Ok(Some(commit_ready(
        session, &ready, limits, 0, phase, control,
    )?))
}

fn operation_state(
    session: &Session,
    operation: OperationId,
) -> Result<(Option<Arc<ValidatedRevision>>, Option<OperationRecord>), WorkspaceError> {
    let catalog = session
        .catalog
        .read()
        .map_err(|_| WorkspaceError::Poisoned)?;
    Ok((
        catalog.committed_operation(operation.into_bytes()).cloned(),
        catalog.operation(operation.into_bytes()).cloned(),
    ))
}

fn commit_sync_uncertainty(
    session: &Session,
    operation: OperationId,
    operation_directory: bool,
) -> Option<CommitOutcome> {
    let result = if operation_directory {
        session.store.sync_operations()
    } else {
        session.store.sync_revisions()
    };
    result.err().map(|error| {
        CommitOutcome::Indeterminate(CommitUncertainty::new(
            operation,
            if operation_directory {
                CommitPhase::OperationPublication
            } else {
                CommitPhase::RevisionDirectorySync
            },
            map_persistence(error).to_string(),
        ))
    })
}

struct PointSetRows<'a> {
    batches: PointSetRecordBatches,
    current: std::vec::IntoIter<PointSetRecord>,
    value: u8,
    max_frames: u64,
    frames: u64,
    control: &'a OperationControl,
    terminal: bool,
}

impl<'a> PointSetRows<'a> {
    fn new(
        batches: PointSetRecordBatches,
        value: u8,
        limits: CommitLimits,
        control: &'a OperationControl,
    ) -> Self {
        Self {
            batches,
            current: Vec::new().into_iter(),
            value,
            max_frames: limits.max_input_frames(),
            frames: 0,
            control,
            terminal: false,
        }
    }

    fn fail(&mut self, error: PersistenceError) -> Result<RevisionRow, PersistenceError> {
        self.terminal = true;
        Err(error)
    }
}

impl Iterator for PointSetRows<'_> {
    type Item = Result<RevisionRow, PersistenceError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.terminal {
            return None;
        }
        loop {
            for record in self.current.by_ref() {
                if record.effective_classification != self.value {
                    return Some(Ok(RevisionRow {
                        ordinal: record.ordinal,
                        before: record.effective_classification,
                        after: self.value,
                    }));
                }
            }
            if self.control.check_cancelled().is_err() {
                return Some(self.fail(PersistenceError::Cancelled));
            }
            drop(std::mem::replace(&mut self.current, Vec::new().into_iter()));
            match self.batches.next() {
                Ok(Some(batch)) => {
                    let Some(frames) = self.frames.checked_add(1) else {
                        return Some(self.fail(PersistenceError::Limit {
                            resource: "Point Set input frames",
                            actual: u64::MAX,
                            limit: self.max_frames,
                        }));
                    };
                    if frames > self.max_frames {
                        return Some(self.fail(PersistenceError::Limit {
                            resource: "Point Set input frames",
                            actual: frames,
                            limit: self.max_frames,
                        }));
                    }
                    self.frames = frames;
                    self.current = batch.into_iter();
                }
                Ok(None) => {
                    self.terminal = true;
                    return None;
                }
                Err(error) => {
                    return Some(self.fail(PersistenceError::Input(Box::new(error))));
                }
            }
        }
    }
}

fn classification_request_digest(
    session: &Session,
    metadata: PointSetMetadata,
    value: u8,
) -> [u8; 32] {
    let provenance = metadata.provenance();
    persisted_classification_request_digest(ClassificationRequestFacts {
        workspace: session.identity.into_bytes(),
        source: session.source().identity().into_bytes(),
        classification_attribute: session.classification_attribute().get(),
        point_set_workspace: provenance.workspace().into_bytes(),
        point_set_source: provenance.source().into_bytes(),
        parent: provenance.revision().into_bytes(),
        point_set: PersistedPointSetFacts {
            exact_count: metadata.exact_count(),
            point_id_hash: metadata.point_id_hash().into_bytes(),
            content_hash: metadata.content_hash().into_bytes(),
        },
        value,
    })
}

fn revert_request_digest(session: &Session, expected_head: RevisionId) -> [u8; 32] {
    persisted_revert_request_digest(
        session.identity.into_bytes(),
        session.source().identity().into_bytes(),
        expected_head.into_bytes(),
    )
}

fn current_head(session: &Session) -> Result<RevisionId, WorkspaceError> {
    let head = session
        .catalog
        .read()
        .map_err(|_| WorkspaceError::Poisoned)?
        .head();
    RevisionId::from_bytes(head)
}

fn next_sequence(session: &Session) -> Result<u64, WorkspaceError> {
    Ok(session
        .catalog
        .read()
        .map_err(|_| WorkspaceError::Poisoned)?
        .next_sequence())
}

fn classification_commit_budgets(
    metadata: PointSetMetadata,
    limits: CommitLimits,
) -> Result<(PointIdReadLimits, WriteLimits), WorkspaceError> {
    let minimum_input_bytes =
        u64::try_from(std::mem::size_of::<PointSetRecord>().max(std::mem::size_of::<PointId>()))
            .unwrap_or(u64::MAX);
    let needs_input = metadata.exact_count() > 0;
    if needs_input {
        require(
            minimum_input_bytes,
            limits.max_working_bytes(),
            "Point Set commit input working bytes",
        )?;
    }
    let reserved_input = if needs_input { minimum_input_bytes } else { 0 };
    let rows_per_block = bounded_output_rows(limits, reserved_input);
    let block_bytes = u64::from(rows_per_block).saturating_mul(10);
    let input_bytes = limits.max_working_bytes().saturating_sub(block_bytes);
    let batch_points = if minimum_input_bytes == 0 {
        0
    } else {
        input_bytes / minimum_input_bytes
    };
    let read = PointIdReadLimits::new(
        metadata.exact_count().min(limits.max_selected_points()),
        batch_points,
        input_bytes,
        input_bytes,
        input_bytes,
    );
    Ok((read, write_limits(limits, input_bytes, rows_per_block)))
}

fn revert_commit_budgets(
    head: &ValidatedRevision,
    limits: CommitLimits,
) -> Result<(RowReadLimits, WriteLimits), WorkspaceError> {
    let input_bytes = head.max_block_bytes();
    require(
        input_bytes,
        limits.max_working_bytes(),
        "Revert input working bytes",
    )?;
    let rows_per_block = bounded_output_rows(limits, input_bytes);
    if rows_per_block == 0 {
        return Err(WorkspaceError::ResourceLimit {
            limit: "Revert output block working bytes",
            required: input_bytes.saturating_add(10),
            allowed: limits.max_working_bytes(),
        });
    }
    Ok((
        RowReadLimits {
            max_frames: limits.max_input_frames(),
            max_payload_bytes: limits.max_revision_bytes(),
            max_working_bytes: input_bytes,
        },
        write_limits(limits, input_bytes, rows_per_block),
    ))
}

fn bounded_output_rows(limits: CommitLimits, retained_input_bytes: u64) -> u32 {
    let by_working = limits
        .max_working_bytes()
        .saturating_sub(retained_input_bytes)
        / 10;
    let rows = limits
        .max_block_points()
        .min(limits.max_block_bytes() / 10)
        .min(by_working)
        .min(u64::from(u32::MAX));
    u32::try_from(rows).expect("bounded output row count fits u32")
}

fn write_limits(
    limits: CommitLimits,
    retained_input_bytes: u64,
    rows_per_block: u32,
) -> WriteLimits {
    let max_blocks = if limits.max_changed_points() == 0 {
        0
    } else if rows_per_block == 0 {
        limits.max_changed_points()
    } else {
        limits
            .max_changed_points()
            .saturating_add(u64::from(rows_per_block) - 1)
            / u64::from(rows_per_block)
    };
    WriteLimits {
        max_file_bytes: limits.max_revision_bytes(),
        max_rows: limits.max_changed_points(),
        max_blocks,
        max_block_bytes: limits.max_block_bytes(),
        rows_per_block,
        max_working_bytes: limits.max_working_bytes(),
        retained_input_bytes,
        max_temporary_bytes: limits.max_temporary_bytes(),
    }
}

fn require_durable_growth(
    session: &Session,
    additional: u64,
    limits: CommitLimits,
) -> Result<(), WorkspaceError> {
    let current = session
        .store
        .durable_payload_bytes()
        .map_err(map_persistence)?;
    let required = current
        .checked_add(additional)
        .ok_or(WorkspaceError::ResourceLimit {
            limit: "total durable Workspace bytes",
            required: u64::MAX,
            allowed: limits.max_total_durable_bytes(),
        })?;
    require(
        required,
        limits.max_total_durable_bytes(),
        "total durable Workspace bytes",
    )
}

fn remaining_transition_budget(
    allowed: u64,
    retained: u64,
    resource: &'static str,
) -> Result<u64, WorkspaceError> {
    allowed
        .checked_sub(retained)
        .ok_or(WorkspaceError::ResourceLimit {
            limit: resource,
            required: retained,
            allowed,
        })
}

fn require_recovery_growth(session: &Session, additional: u64) -> Result<(), WorkspaceError> {
    require(
        additional,
        session.open_limits.max_single_file_bytes(),
        "recovery rejection file bytes",
    )?;
    require(
        additional,
        session.open_limits.max_working_bytes(),
        "recovery rejection working bytes",
    )?;
    let operation_files = session
        .catalog
        .read()
        .map_err(|_| WorkspaceError::Poisoned)?
        .operation_file_count()
        .saturating_add(1);
    require(
        operation_files,
        session.open_limits.max_operation_records(),
        "recovery Operation files",
    )?;
    let current = session
        .store
        .durable_payload_bytes()
        .map_err(map_persistence)?;
    let required = current
        .checked_add(additional)
        .ok_or(WorkspaceError::ResourceLimit {
            limit: "recovery durable Workspace bytes",
            required: u64::MAX,
            allowed: session.open_limits.max_total_persisted_bytes(),
        })?;
    require(
        required,
        session.open_limits.max_total_persisted_bytes(),
        "recovery durable Workspace bytes",
    )
}

fn enforce_ready_limits(
    ready: &ValidatedRevision,
    limits: CommitLimits,
) -> Result<(), WorkspaceError> {
    if let Some(point_set) = ready.point_set() {
        require(
            point_set.exact_count,
            limits.max_selected_points(),
            "selected Points",
        )?;
    }
    require(
        ready.row_count(),
        limits.max_changed_points(),
        "changed Points",
    )?;
    require(
        ready.block_count(),
        limits.max_input_frames(),
        "ready input frames",
    )?;
    require(
        ready.file_bytes(),
        limits.max_revision_bytes(),
        "Revision bytes",
    )?;
    require(
        ready.max_block_rows(),
        limits.max_block_points(),
        "Revision block Points",
    )?;
    require(
        ready.max_block_bytes(),
        limits.max_block_bytes(),
        "Revision block bytes",
    )
}

fn receipt_from_revision(revision: &ValidatedRevision) -> Result<CommitReceipt, WorkspaceError> {
    let operation = OperationId::from_bytes(revision.operation())?;
    Ok(CommitReceipt::new(
        operation,
        revision_info_from_persisted(revision)?,
    ))
}

fn rejection_reason(facts: RejectionFacts) -> Result<CommitRejection, WorkspaceError> {
    let expected = optional_revision(facts.expected_head)?;
    let actual = optional_revision(facts.actual_head)?;
    CommitRejection::from_code(facts.reason_code, expected, actual)
}

fn optional_revision(bytes: [u8; 32]) -> Result<Option<RevisionId>, WorkspaceError> {
    if bytes == [0; 32] {
        Ok(None)
    } else {
        RevisionId::from_bytes(bytes).map(Some)
    }
}

fn rejection_facts(
    session: &Session,
    operation: OperationId,
    request_digest: [u8; 32],
    reason: CommitRejection,
) -> RejectionFacts {
    let (expected_head, actual_head) = match reason {
        CommitRejection::StaleHead { expected, actual } => {
            (expected.into_bytes(), actual.into_bytes())
        }
        _ => ([0; 32], [0; 32]),
    };
    RejectionFacts {
        workspace: session.identity.into_bytes(),
        operation: operation.into_bytes(),
        request_digest,
        reason_code: reason.code(),
        expected_head,
        actual_head,
    }
}

fn record_rejection(
    session: &Session,
    operation: OperationId,
    request_digest: [u8; 32],
    reason: CommitRejection,
    limits: CommitLimits,
    phase: &PublicationPhase,
    control: &OperationControl,
) -> Result<CommitOutcome, WorkspaceError> {
    require(
        REJECTION_BYTES as u64,
        limits.max_temporary_bytes(),
        "rejection temporary bytes",
    )?;
    require(
        REJECTION_BYTES as u64,
        limits.max_working_bytes(),
        "rejection working bytes",
    )?;
    require_durable_growth(session, REJECTION_BYTES as u64, limits)?;
    let facts = rejection_facts(session, operation, request_digest, reason);
    let prepared = session
        .store
        .prepare_rejection(facts, limits.max_working_bytes())
        .map_err(map_persistence)?;
    let catalog_budget = remaining_transition_budget(
        limits.max_working_bytes(),
        prepared.working_bytes(),
        "rejection catalog growth with stage retained",
    )?;
    session
        .catalog
        .write()
        .map_err(|_| WorkspaceError::Poisoned)?
        .reserve_operation(operation.into_bytes(), catalog_budget)
        .map_err(map_persistence)?;
    session
        .store
        .publish_prepared_rejection(prepared, || begin_publication(control, phase, false))
        .map_err(map_persistence)?;
    session
        .catalog
        .write()
        .map_err(|_| WorkspaceError::Poisoned)?
        .record_rejection(facts);
    Ok(CommitOutcome::Rejected(reason))
}

fn recorded_intent(
    session: &Session,
    ready: &ValidatedRevision,
) -> Result<RecordedIntent, WorkspaceError> {
    let operation = OperationId::from_bytes(ready.operation())?;
    let parent_revision = RevisionId::from_bytes(ready.parent())?;
    let parent = SnapshotProvenance::new(
        session.identity,
        session.source().identity(),
        parent_revision,
    );
    let revision = RevisionId::from_bytes(ready.revision())?;
    let kind = match ready.kind() {
        PersistedRevisionKind::SetClassification(value) => RevisionKind::SetClassification {
            value,
            changed_points: ready.row_count(),
        },
        PersistedRevisionKind::Revert => RevisionKind::Revert {
            reverted_revision: parent_revision,
            changed_points: ready.row_count(),
        },
    };
    let point_set = ready.point_set().map(|facts| {
        PointSetMetadata::new(
            parent,
            facts.exact_count,
            ContentHash::new(facts.point_id_hash),
            ContentHash::new(facts.content_hash),
        )
    });
    Ok(RecordedIntent::new(
        operation,
        ContentHash::new(ready.request_digest()),
        parent,
        revision,
        ready.sequence(),
        kind,
        point_set,
    ))
}

fn recorded_rejection(facts: RejectionFacts) -> Result<RecordedRejection, WorkspaceError> {
    Ok(RecordedRejection::new(
        OperationId::from_bytes(facts.operation)?,
        ContentHash::new(facts.request_digest),
        rejection_reason(facts)?,
    ))
}

// Keeping reconciliation in one linear precedence table makes its durability
// sync and certainty transitions auditable as a single protocol.
#[allow(clippy::too_many_lines)]
fn resolve_operation(
    session: &Arc<Session>,
    operation: OperationId,
) -> Result<OperationResolution, WorkspaceError> {
    session.ensure_mutable()?;
    let _writer = session
        .writer
        .lock()
        .map_err(|_| WorkspaceError::Poisoned)?;
    session.ensure_mutable()?;
    let (committed, record) = operation_state(session, operation)?;
    if let Some(committed) = committed {
        if let Err(error) = session.store.sync_revisions() {
            session.poisoned.store(true, Ordering::SeqCst);
            return Ok(OperationResolution::Indeterminate(CommitUncertainty::new(
                operation,
                CommitPhase::RevisionDirectorySync,
                map_persistence(error).to_string(),
            )));
        }
        return Ok(OperationResolution::Committed(receipt_from_revision(
            &committed,
        )?));
    }
    let Some(record) = record else {
        return Ok(OperationResolution::NotRecorded);
    };
    if let Some(rejection) = record.rejection {
        if let Err(error) = session.store.sync_operations() {
            session.poisoned.store(true, Ordering::SeqCst);
            return Ok(OperationResolution::Indeterminate(CommitUncertainty::new(
                operation,
                CommitPhase::OperationPublication,
                map_persistence(error).to_string(),
            )));
        }
        return Ok(OperationResolution::Rejected(recorded_rejection(
            rejection,
        )?));
    }
    let Some(ready) = record.ready else {
        return Ok(OperationResolution::NotRecorded);
    };
    if let Err(error) = session.store.sync_operations() {
        session.poisoned.store(true, Ordering::SeqCst);
        return Ok(OperationResolution::Indeterminate(CommitUncertainty::new(
            operation,
            CommitPhase::OperationPublication,
            map_persistence(error).to_string(),
        )));
    }
    let intent = recorded_intent(session, &ready)?;
    if intent.parent().revision() == current_head(session)? {
        return Ok(OperationResolution::Retryable(Box::new(intent)));
    }

    let reason = CommitRejection::StaleHead {
        expected: intent.parent().revision(),
        actual: current_head(session)?,
    };
    let facts = rejection_facts(
        session,
        operation,
        intent.request_hash().into_bytes(),
        reason,
    );
    require_recovery_growth(session, REJECTION_BYTES as u64)?;
    let prepared = session
        .store
        .prepare_rejection(facts, session.open_limits.max_working_bytes())
        .map_err(map_persistence)?;
    let catalog_budget = remaining_transition_budget(
        session.open_limits.max_working_bytes(),
        prepared.working_bytes(),
        "recovery rejection catalog growth with stage retained",
    )?;
    session
        .catalog
        .write()
        .map_err(|_| WorkspaceError::Poisoned)?
        .reserve_operation(operation.into_bytes(), catalog_budget)
        .map_err(map_persistence)?;
    let phase = PublicationPhase::new();
    if let Err(error) = session.store.publish_prepared_rejection(prepared, || {
        phase.mark_operation();
        Ok(())
    }) {
        if let Some(commit_phase) = phase.current() {
            session.poisoned.store(true, Ordering::SeqCst);
            return Ok(OperationResolution::Indeterminate(CommitUncertainty::new(
                operation,
                commit_phase,
                map_persistence(error).to_string(),
            )));
        }
        return Err(map_persistence(error));
    }
    session
        .catalog
        .write()
        .map_err(|_| WorkspaceError::Poisoned)?
        .record_rejection(facts);
    Ok(OperationResolution::Rejected(RecordedRejection::new(
        operation,
        intent.request_hash(),
        reason,
    )))
}

fn create_workspace(
    root: &Path,
    index: PreparedIndex,
    schema: WorkspaceSchema,
    limits: OpenLimits,
    control: &OperationControl,
) -> Result<Workspace, WorkspaceError> {
    control.check_cancelled()?;
    schema.validate_source(index.source())?;
    validate_root_limits(limits)?;
    let identity = WorkspaceId::generate()?;
    let source_contract = source_contract(index.source(), schema);
    let root_revision = root_revision(identity, index.source().identity(), source_contract)?;
    let manifest = manifest_facts(
        identity,
        index.source(),
        schema,
        root_revision,
        source_contract,
    );
    let mut store = Store::create(root).map_err(map_persistence)?;
    control.check_cancelled()?;
    store.publish_manifest(&manifest).map_err(map_persistence)?;
    let catalog = Catalog::empty(manifest.root_revision);
    Ok(Workspace {
        session: Arc::new(Session {
            identity,
            schema,
            index,
            manifest,
            open_limits: limits,
            store,
            catalog: RwLock::new(catalog),
            writer: Mutex::new(()),
            poisoned: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            writer_waiters: std::sync::atomic::AtomicUsize::new(0),
        }),
    })
}

fn open_workspace(
    root: &Path,
    index: PreparedIndex,
    limits: OpenLimits,
    control: &OperationControl,
) -> Result<Workspace, WorkspaceError> {
    control.check_cancelled()?;
    validate_root_limits(limits)?;
    let store = Store::open(root).map_err(map_persistence)?;
    let manifest = store.read_manifest().map_err(map_persistence)?;
    let identity = WorkspaceId::from_bytes(manifest.workspace)?;
    let classification = AttributeId::new(manifest.classification_attribute)?;
    let schema = WorkspaceSchema::new(classification);
    schema.validate_source(index.source())?;
    validate_manifest(&manifest, index.source(), schema)?;
    control.check_cancelled()?;
    let catalog = store
        .recover(&manifest, catalog_limits(limits)?, control)
        .map_err(map_persistence)?;
    sync_recovered_directories(&store)?;
    Ok(Workspace {
        session: Arc::new(Session {
            identity,
            schema,
            index,
            manifest,
            open_limits: limits,
            store,
            catalog: RwLock::new(catalog),
            writer: Mutex::new(()),
            poisoned: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            writer_waiters: std::sync::atomic::AtomicUsize::new(0),
        }),
    })
}

fn sync_recovered_directories(store: &Store) -> Result<(), WorkspaceError> {
    store
        .sync_parent()
        .and_then(|()| store.sync_root())
        .and_then(|()| store.sync_operations())
        .and_then(|()| store.sync_revisions())
        .map_err(|error| WorkspaceError::RecoveryIndeterminate {
            operation: None,
            reason: WorkspaceDiagnostic::new(map_persistence(error).to_string()),
        })
}

fn validate_root_limits(limits: OpenLimits) -> Result<(), WorkspaceError> {
    require(
        MANIFEST_BYTES as u64,
        limits.max_manifest_bytes(),
        "manifest bytes",
    )?;
    require(
        MANIFEST_BYTES as u64,
        limits.max_single_file_bytes(),
        "manifest single-file bytes",
    )?;
    require(
        1,
        limits.max_revision_files(),
        "Revision files including root",
    )?;
    require(
        MANIFEST_BYTES as u64,
        limits.max_total_persisted_bytes(),
        "total persisted bytes",
    )
}

fn catalog_limits(limits: OpenLimits) -> Result<CatalogLimits, WorkspaceError> {
    let remaining_persisted = limits
        .max_total_persisted_bytes()
        .checked_sub(MANIFEST_BYTES as u64)
        .ok_or(WorkspaceError::ResourceLimit {
            limit: "total persisted bytes",
            required: MANIFEST_BYTES as u64,
            allowed: limits.max_total_persisted_bytes(),
        })?;
    Ok(CatalogLimits {
        read: ReadLimits {
            max_file_bytes: limits.max_single_file_bytes(),
            max_rows: limits.max_revision_rows(),
            max_blocks: limits.max_revision_blocks(),
            max_block_bytes: limits
                .max_revision_block_bytes()
                .min(limits.max_working_bytes()),
            max_working_bytes: limits.max_working_bytes(),
        },
        max_revisions: limits.max_revision_files().saturating_sub(1),
        max_operation_files: limits.max_operation_records(),
        max_total_bytes: remaining_persisted,
        max_metadata_bytes: limits.max_resident_metadata_bytes(),
    })
}

fn manifest_facts(
    identity: WorkspaceId,
    source: &Source,
    schema: WorkspaceSchema,
    root_revision: RevisionId,
    source_contract: [u8; 32],
) -> ManifestFacts {
    let transform = source.metadata().position_transform();
    let offset = transform.offset();
    let scale = transform.scale();
    ManifestFacts {
        workspace: identity.into_bytes(),
        source: source.identity().into_bytes(),
        source_point_count: source.metadata().point_count(),
        position_transform_bits: [
            offset[0].to_bits(),
            offset[1].to_bits(),
            offset[2].to_bits(),
            scale[0].to_bits(),
            scale[1].to_bits(),
            scale[2].to_bits(),
        ],
        classification_attribute: schema.classification().get(),
        root_revision: root_revision.into_bytes(),
        source_contract,
    }
}

fn validate_manifest(
    manifest: &ManifestFacts,
    source: &Source,
    schema: WorkspaceSchema,
) -> Result<(), WorkspaceError> {
    let expected_contract = source_contract(source, schema);
    let expected_transform = source.metadata().position_transform();
    let expected_offset = expected_transform.offset();
    let expected_scale = expected_transform.scale();
    let expected_transform_bits = [
        expected_offset[0].to_bits(),
        expected_offset[1].to_bits(),
        expected_offset[2].to_bits(),
        expected_scale[0].to_bits(),
        expected_scale[1].to_bits(),
        expected_scale[2].to_bits(),
    ];
    if manifest.source != source.identity().into_bytes()
        || manifest.source_point_count != source.metadata().point_count()
        || manifest.position_transform_bits != expected_transform_bits
        || manifest.classification_attribute != schema.classification().get()
        || manifest.source_contract != expected_contract
    {
        return Err(WorkspaceError::incompatible(
            "Workspace manifest does not match the verified Source contract",
        ));
    }
    let identity = WorkspaceId::from_bytes(manifest.workspace)?;
    let expected_root = root_revision(identity, source.identity(), expected_contract)?;
    if manifest.root_revision != expected_root.into_bytes() {
        return Err(WorkspaceError::corrupt(
            "Workspace root Revision does not match its canonical manifest facts",
        ));
    }
    Ok(())
}

fn source_contract(source: &Source, schema: WorkspaceSchema) -> [u8; 32] {
    let transform = source.metadata().position_transform();
    let mut hasher = Hasher::new();
    hasher.update(SOURCE_CONTRACT_DOMAIN);
    hasher.update(source.identity().as_bytes());
    hasher.update(&source.metadata().point_count().to_le_bytes());
    for value in transform.offset().into_iter().chain(transform.scale()) {
        hasher.update(&value.to_bits().to_le_bytes());
    }
    hasher.update(&schema.classification().get().to_le_bytes());
    *hasher.finalize().as_bytes()
}

fn root_revision(
    workspace: WorkspaceId,
    source: SourceId,
    source_contract: [u8; 32],
) -> Result<RevisionId, WorkspaceError> {
    let mut hasher = Hasher::new();
    hasher.update(ROOT_REVISION_DOMAIN);
    hasher.update(workspace.as_bytes());
    hasher.update(source.as_bytes());
    hasher.update(&source_contract);
    RevisionId::from_hash(*hasher.finalize().as_bytes())
}

fn revision_info_from_persisted(
    persisted: &crate::persistence::ValidatedRevision,
) -> Result<RevisionInfo, WorkspaceError> {
    let id = RevisionId::from_bytes(persisted.revision())?;
    let parent = RevisionId::from_bytes(persisted.parent())?;
    let operation = crate::model::OperationId::from_bytes(persisted.operation())?;
    let kind = match persisted.kind() {
        crate::persistence::RevisionKind::SetClassification(value) => {
            RevisionKind::SetClassification {
                value,
                changed_points: persisted.row_count(),
            }
        }
        crate::persistence::RevisionKind::Revert => RevisionKind::Revert {
            reverted_revision: parent,
            changed_points: persisted.row_count(),
        },
    };
    Ok(RevisionInfo::new(
        id,
        Some(parent),
        persisted.sequence(),
        Some(operation),
        kind,
    ))
}

pub(crate) fn map_persistence(error: PersistenceError) -> WorkspaceError {
    match error {
        PersistenceError::Locked => WorkspaceError::Locked,
        PersistenceError::Io {
            action,
            path,
            source,
        } => WorkspaceError::io(action, path.display(), source),
        PersistenceError::Corrupt { path, reason } => {
            WorkspaceError::corrupt(format!("{}: {reason}", path.display()))
        }
        PersistenceError::Incompatible { path, reason } => {
            WorkspaceError::incompatible(format!("{}: {reason}", path.display()))
        }
        PersistenceError::Limit {
            resource,
            actual,
            limit,
        } => WorkspaceError::ResourceLimit {
            limit: resource,
            required: actual,
            allowed: limit,
        },
        PersistenceError::PublicationConflict => {
            WorkspaceError::corrupt("immutable publication target already exists")
        }
        PersistenceError::Entropy => WorkspaceError::random("scratch entropy unavailable"),
        PersistenceError::Cancelled => WorkspaceError::Cancelled,
        PersistenceError::NoRows => {
            WorkspaceError::corrupt("an empty candidate reached persistence unexpectedly")
        }
        PersistenceError::Input(error) => *error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::{FaultPoint, test_fault};

    struct TestDirectory(std::path::PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            for _ in 0..64 {
                let mut random = [0_u8; 16];
                getrandom::fill(&mut random).expect("test entropy");
                let name = format!("{:032x}", u128::from_le_bytes(random));
                let path = std::env::temp_dir().join(format!("punctra-workspace-{name}"));
                if !path.exists() {
                    return Self(path);
                }
            }
            panic!("could not choose a unique Workspace test directory");
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn manifest_requires_exact_single_file_capacity() {
        let below = OpenLimits::new(
            MANIFEST_BYTES as u64,
            0,
            1,
            0,
            0,
            0,
            MANIFEST_BYTES as u64 - 1,
            MANIFEST_BYTES as u64,
            0,
            0,
        );
        assert!(matches!(
            validate_root_limits(below),
            Err(WorkspaceError::ResourceLimit { .. })
        ));
        let exact = OpenLimits::new(
            MANIFEST_BYTES as u64,
            0,
            1,
            0,
            0,
            0,
            MANIFEST_BYTES as u64,
            MANIFEST_BYTES as u64,
            0,
            0,
        );
        validate_root_limits(exact).expect("exact manifest single-file boundary");
    }

    #[test]
    fn dropped_worker_guard_poisons_only_after_publication_begins() {
        let prepublication = Arc::new(AtomicBool::new(false));
        let phase = Arc::new(PublicationPhase::new());
        drop(MutationCertaintyGuard::new(
            Arc::clone(&prepublication),
            Arc::clone(&phase),
        ));
        assert!(!prepublication.load(Ordering::SeqCst));

        let postpublication = Arc::new(AtomicBool::new(false));
        phase.mark_operation();
        drop(MutationCertaintyGuard::new(
            Arc::clone(&postpublication),
            phase,
        ));
        assert!(postpublication.load(Ordering::SeqCst));
    }

    #[test]
    fn definitive_worker_result_disarms_poison_but_indeterminate_forces_it() {
        let definitive = Arc::new(AtomicBool::new(false));
        let mut guard =
            MutationCertaintyGuard::new(Arc::clone(&definitive), Arc::new(PublicationPhase::new()));
        guard.disarm();
        drop(guard);
        assert!(!definitive.load(Ordering::SeqCst));

        let indeterminate = Arc::new(AtomicBool::new(false));
        let mut guard = MutationCertaintyGuard::new(
            Arc::clone(&indeterminate),
            Arc::new(PublicationPhase::new()),
        );
        guard.force_poison();
        drop(guard);
        assert!(indeterminate.load(Ordering::SeqCst));
    }

    #[test]
    fn cancellation_maps_by_publication_phase() {
        let operation = OperationId::from_bytes([1; 16]).expect("nonzero Operation");
        let phase = PublicationPhase::new();
        let poisoned = AtomicBool::new(false);
        let before = finish_commit_result_with_poison(
            operation,
            &phase,
            &poisoned,
            Err(WorkspaceError::Cancelled),
        );
        assert!(matches!(before, Err(WorkspaceError::Cancelled)));
        assert!(!poisoned.load(Ordering::SeqCst));

        phase.mark_operation();
        let after_ready = finish_commit_result_with_poison(
            operation,
            &phase,
            &poisoned,
            Err(WorkspaceError::Cancelled),
        )
        .expect("post-publication cancellation is an outcome");
        assert!(matches!(after_ready, CommitOutcome::Indeterminate(_)));
        assert!(poisoned.load(Ordering::SeqCst));

        phase.mark_revision();
        let after_revision = finish_commit_result_with_poison(
            operation,
            &phase,
            &AtomicBool::new(false),
            Err(WorkspaceError::Cancelled),
        )
        .expect("post-Revision cancellation is an outcome");
        let CommitOutcome::Indeterminate(uncertainty) = after_revision else {
            panic!("expected indeterminate Revision publication");
        };
        assert_eq!(uncertainty.phase(), CommitPhase::RevisionPublication);
    }

    #[test]
    fn recovered_directory_sync_failure_is_indeterminate() {
        for point in [
            FaultPoint::RecoveryParentSync,
            FaultPoint::RecoveryRootSync,
            FaultPoint::RecoveryOperationsSync,
            FaultPoint::RecoveryRevisionsSync,
        ] {
            let directory = TestDirectory::new();
            let store = Store::create(&directory.0).expect("create test Store");
            let fault = test_fault::Guard::install(&directory.0, point, test_fault::Action::Error);
            assert!(matches!(
                sync_recovered_directories(&store),
                Err(WorkspaceError::RecoveryIndeterminate { .. })
            ));
            drop(fault);
            drop(store);
        }
    }

    // End-to-end fixture intentionally keeps create, reject, reopen, and
    // reconciliation in one test so the durable evidence cannot be partial.
    #[allow(clippy::too_many_lines)]
    #[test]
    fn foreign_point_set_is_durably_rejected_without_panicking() {
        use point_contracts::{
            AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition,
            AttributeValues, CoordinateReference, PositionTransform,
        };
        use point_index::{PrepareLimits, prepare};
        use source_memory::MemorySource;

        let directory = TestDirectory::new();
        std::fs::create_dir(&directory.0).expect("fixture directory");
        let classification = AttributeId::new(7).expect("classification identity");
        let definition =
            AttributeDefinition::new(classification, "classification", AttributeDataType::U8)
                .expect("classification definition");
        let columns = AttributeColumns::new(
            vec![
                AttributeColumn::new(definition, AttributeValues::u8(vec![0]))
                    .expect("classification column"),
            ],
            1,
        )
        .expect("aligned columns");
        let transform = PositionTransform::new([0.0; 3], [1.0; 3]).expect("transform");
        let source = source_memory::open(
            MemorySource::from_columns(
                transform,
                CoordinateReference::Unknown,
                vec![[0, 0, 0]],
                columns,
            )
            .expect("memory Source"),
        )
        .blocking_wait()
        .expect("open Source");
        let index = prepare(
            source,
            directory.0.join("foreign.pidx"),
            PrepareLimits::default(),
        )
        .blocking_wait()
        .expect("prepare index");
        let first = create(
            directory.0.join("first.pcw"),
            index.clone(),
            WorkspaceSchema::new(classification),
            OpenLimits::default(),
        )
        .blocking_wait()
        .expect("first Workspace");
        let second_path = directory.0.join("second.pcw");
        let second = create(
            &second_path,
            index.clone(),
            WorkspaceSchema::new(classification),
            OpenLimits::default(),
        )
        .blocking_wait()
        .expect("second Workspace");
        let points = first
            .head()
            .select(crate::PointQuery::all(), PointSetLimits::default())
            .blocking_wait()
            .expect("foreign Point Set");
        let operation = OperationId::from_bytes([11; 16]).expect("Operation Identity");
        let outcome = second
            .commit(
                CommitRequest::set_classification(operation, points, 1),
                CommitLimits::default(),
            )
            .blocking_wait()
            .expect("definitive foreign rejection");
        assert_eq!(
            outcome,
            CommitOutcome::Rejected(CommitRejection::ForeignPointSet)
        );
        drop(second);

        let reopened = open(&second_path, index.clone(), OpenLimits::default())
            .blocking_wait()
            .expect("reopen second Workspace");
        let OperationResolution::Rejected(recorded) = reopened
            .resolve_operation(operation)
            .expect("resolve durable rejection")
        else {
            panic!("expected durable foreign rejection");
        };
        assert_eq!(recorded.reason(), CommitRejection::ForeignPointSet);

        let stale_operation = OperationId::from_bytes([12; 16]).expect("stale Operation");
        let unknown = RevisionId::from_bytes([13; 32]).expect("opaque expected Revision");
        let stale = reopened
            .commit(
                CommitRequest::revert_head(stale_operation, unknown),
                CommitLimits::default(),
            )
            .blocking_wait()
            .expect("definitive stale rejection");
        assert!(matches!(
            stale,
            CommitOutcome::Rejected(CommitRejection::StaleHead { expected, .. })
                if expected == unknown
        ));
        drop(reopened);
        let reopened = open(&second_path, index, OpenLimits::default())
            .blocking_wait()
            .expect("reopen after stale rejection");
        let OperationResolution::Rejected(recorded) = reopened
            .resolve_operation(stale_operation)
            .expect("resolve durable stale rejection")
        else {
            panic!("expected durable stale rejection");
        };
        assert!(matches!(
            recorded.reason(),
            CommitRejection::StaleHead { expected, .. } if expected == unknown
        ));
    }

    #[test]
    fn dropping_commit_during_post_ready_panic_poisoned_retained_workspace() {
        use point_contracts::{
            AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition,
            AttributeValues, CoordinateReference, PositionTransform,
        };
        use point_index::{PrepareLimits, prepare};
        use source_memory::MemorySource;

        let directory = TestDirectory::new();
        std::fs::create_dir(&directory.0).expect("fixture directory");
        let classification = AttributeId::new(7).expect("classification identity");
        let definition =
            AttributeDefinition::new(classification, "classification", AttributeDataType::U8)
                .expect("classification definition");
        let columns = AttributeColumns::new(
            vec![
                AttributeColumn::new(definition, AttributeValues::u8(vec![0]))
                    .expect("classification column"),
            ],
            1,
        )
        .expect("aligned columns");
        let transform = PositionTransform::new([0.0; 3], [1.0; 3]).expect("transform");
        let input = MemorySource::from_columns(
            transform,
            CoordinateReference::Unknown,
            vec![[0, 0, 0]],
            columns,
        )
        .expect("memory Source");
        let source = source_memory::open(input)
            .blocking_wait()
            .expect("open Source");
        let index = prepare(
            source,
            directory.0.join("fixture.pidx"),
            PrepareLimits::default(),
        )
        .blocking_wait()
        .expect("prepare index");
        let workspace_path = directory.0.join("fixture.pcw");
        let workspace = create(
            &workspace_path,
            index,
            WorkspaceSchema::new(classification),
            OpenLimits::default(),
        )
        .blocking_wait()
        .expect("create Workspace");
        let points = workspace
            .head()
            .select(crate::PointQuery::all(), PointSetLimits::default())
            .blocking_wait()
            .expect("select Point Set");

        let fault = test_fault::Guard::install(
            &workspace_path,
            FaultPoint::OperationLostAcknowledgement,
            test_fault::Action::PauseThenPanic,
        );
        let job = workspace.commit(
            CommitRequest::set_classification(
                OperationId::from_bytes([1; 16]).expect("first Operation"),
                points.clone(),
                1,
            ),
            CommitLimits::default(),
        );
        fault.wait_until_hit();
        let queued = workspace.commit(
            CommitRequest::set_classification(
                OperationId::from_bytes([2; 16]).expect("second Operation"),
                points,
                2,
            ),
            CommitLimits::default(),
        );
        let queue_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while workspace.session.writer_waiters.load(Ordering::SeqCst) == 0
            && std::time::Instant::now() < queue_deadline
        {
            std::thread::yield_now();
        }
        assert_eq!(workspace.session.writer_waiters.load(Ordering::SeqCst), 1);
        drop(job);
        fault.release();
        let queued_result = queued.blocking_wait();
        drop(fault);
        assert!(workspace.session.poisoned.load(Ordering::SeqCst));
        assert!(matches!(queued_result, Err(WorkspaceError::Poisoned)));
    }
}
