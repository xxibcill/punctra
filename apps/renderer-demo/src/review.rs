use std::{
    error::Error,
    future::Future,
    mem,
    path::Path,
    pin::Pin,
    task::{Context, Poll, Waker},
};

use point_contracts::PointId;
use point_index::PreparedIndex;
use point_review::{
    ConfirmedPoint, Inspection, PickConfirmationJob, ReviewError, ScreenRect, ScreenReviewJob,
    ScreenReviewLimits, ScreenSelection, confirm_pick, screen_through,
};
use point_workspace::{
    CommitLimits, CommitOutcome, CommitRejection, CommitRequest, OpenLimits, OperationId,
    PointIdReadLimits, PointSet, RevisionAudit, RevisionAuditLimits, RevisionId, RevisionKind,
    Snapshot, SnapshotProvenance, Workspace, open,
};
use render_protocol::{BatchKey, BatchVersion, Camera, ViewGenerationKey, Viewport};
use render_wgpu::PickHit;

pub(crate) const MAX_HIGHLIGHT_POINTS: u64 = 600_000;
const MAX_HIGHLIGHT_BYTES: u64 = 32 * 1_024 * 1_024;
const HIGHLIGHT_BATCH_POINTS: u64 = 65_536;
const HIGHLIGHT_BATCH_BYTES: u64 = 4 * 1_024 * 1_024;
const HIGHLIGHT_READ_BUFFER_BYTES: u64 = 4 * 1_024 * 1_024;
const HIGHLIGHT_WORKING_BYTES: u64 = 8 * 1_024 * 1_024;

pub(crate) type ReviewResult<T> = Result<T, Box<dyn Error>>;

/// The complete renderer input produced only after an exact Point Set reaches
/// terminal bounded identity-read success.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExactHighlights {
    point_ids: Vec<PointId>,
}

impl ExactHighlights {
    fn from_point_set(points: &PointSet) -> ReviewResult<Self> {
        Self::from_point_set_with_limits_and_observer(points, highlight_read_limits(), |_| {})
    }

    fn from_point_set_with_limits_and_observer(
        points: &PointSet,
        limits: PointIdReadLimits,
        observer: impl FnMut(HighlightReadProgress),
    ) -> ReviewResult<Self> {
        Ok(Self {
            point_ids: collect_highlight_ids_with_limits(points, limits, observer)?,
        })
    }

    pub(crate) fn as_slice(&self) -> &[PointId] {
        &self.point_ids
    }

    pub(crate) fn into_vec(self) -> Vec<PointId> {
        self.point_ids
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HighlightReadProgress {
    Batch { collected: u64 },
    Terminal { collected: u64 },
}

/// The renderer-demo fields consumed from the public `PickHit` interface.
///
/// Keeping this value separate from exact confirmation makes it impossible to
/// accidentally treat display metadata as Workspace authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProvisionalPickHint {
    view_generation: ViewGenerationKey,
    batch: BatchKey,
    version: BatchVersion,
    point: PointId,
}

#[cfg(test)]
impl ProvisionalPickHint {
    const fn faithful_public_seam(
        view_generation: ViewGenerationKey,
        batch: BatchKey,
        version: BatchVersion,
        point: PointId,
    ) -> Self {
        Self {
            view_generation,
            batch,
            version,
            point,
        }
    }
}

impl From<PickHit> for ProvisionalPickHint {
    fn from(hit: PickHit) -> Self {
        Self {
            view_generation: hit.view_generation(),
            batch: hit.batch(),
            version: hit.version(),
            point: hit.point(),
        }
    }
}

impl ProvisionalPickHint {
    pub(crate) const fn view_generation(self) -> ViewGenerationKey {
        self.view_generation
    }

    pub(crate) const fn batch(self) -> BatchKey {
        self.batch
    }

    pub(crate) const fn version(self) -> BatchVersion {
        self.version
    }

    pub(crate) const fn point(self) -> PointId {
        self.point
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ClassificationEdit {
    pub(crate) operation: OperationId,
    pub(crate) value: u8,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ReviewOptions {
    pub(crate) classification_filter: Option<u8>,
    pub(crate) classification_edit: Option<ClassificationEdit>,
    pub(crate) revert_operation: Option<OperationId>,
    pub(crate) resolve_operation: Option<OperationId>,
}

#[derive(Clone)]
pub(crate) struct ReviewCapture {
    snapshot: Snapshot,
    provenance: SnapshotProvenance,
    view: CaptureView,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CaptureView {
    view_generation: ViewGenerationKey,
    interaction_generation: u64,
}

impl CaptureView {
    pub(crate) const fn new(
        view_generation: ViewGenerationKey,
        interaction_generation: u64,
    ) -> Self {
        Self {
            view_generation,
            interaction_generation,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SelectionFreshness {
    Current(CaptureView),
    Stale(CaptureView),
}

impl SelectionFreshness {
    const fn from_capture(capture: &ReviewCapture) -> Self {
        Self::Current(capture.view)
    }

    fn invalidate(&mut self, active: CaptureView) -> bool {
        let Self::Current(captured) = *self else {
            return false;
        };
        if captured == active {
            return false;
        }
        *self = Self::Stale(captured);
        true
    }

    fn is_current(self, active: CaptureView) -> bool {
        matches!(self, Self::Current(captured) if captured == active)
    }
}

struct ExactSelection {
    points: PointSet,
    freshness: SelectionFreshness,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MutationKind {
    Classification,
    Revert,
}

impl MutationKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Classification => "classification",
            Self::Revert => "Revert",
        }
    }

    const fn status(self, revision: RevisionId, changed_points: u64) -> ReviewStatus {
        match self {
            Self::Classification => ReviewStatus::Committed {
                revision,
                changed_points,
            },
            Self::Revert => ReviewStatus::Reverted {
                revision,
                changed_points,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MutationDisposition {
    Committed {
        operation: OperationId,
        revision: RevisionId,
        audit_verified: bool,
    },
    Rejected {
        operation: OperationId,
        reason: CommitRejection,
    },
    Indeterminate {
        operation: OperationId,
    },
}

impl MutationDisposition {
    pub(crate) fn require_committed(self, requested: &str) -> ReviewResult<()> {
        match self {
            Self::Committed {
                audit_verified: true,
                ..
            } => Ok(()),
            Self::Committed {
                operation,
                revision,
                audit_verified: false,
            } => Err(format!(
                "requested {requested} Operation {operation} committed as Revision {revision}, but its audit could not be verified; no dependent mutation was attempted"
            )
            .into()),
            Self::Rejected {
                operation,
                reason,
            } => Err(format!(
                "requested {requested} Operation {operation} was definitively rejected: {reason:?}"
            )
            .into()),
            Self::Indeterminate { operation } => Err(format!(
                "requested {requested} Operation {operation} is indeterminate and must be explicitly resolved"
            )
            .into()),
        }
    }
}

impl ReviewCapture {
    pub(crate) const fn view_generation(&self) -> ViewGenerationKey {
        self.view.view_generation
    }
}

enum PendingExactReview {
    Pick {
        capture: ReviewCapture,
        job: Pin<Box<PickConfirmationJob>>,
    },
    Screen {
        capture: ReviewCapture,
        job: Pin<Box<ScreenReviewJob>>,
    },
}

pub(crate) enum CompletedReview {
    Pick {
        capture: ReviewCapture,
        confirmed: ConfirmedPoint,
    },
    Screen {
        capture: ReviewCapture,
        inspection: Inspection,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReviewStatus {
    Ready {
        revision: RevisionId,
    },
    ProvisionalPick,
    ConfirmingPick,
    SelectingScreen,
    Selected {
        revision: RevisionId,
        points: u64,
    },
    SelectionStale {
        revision: RevisionId,
        points: u64,
    },
    Committed {
        revision: RevisionId,
        changed_points: u64,
    },
    Reverted {
        revision: RevisionId,
        changed_points: u64,
    },
    CommittedUnverified {
        revision: RevisionId,
        changed_points: u64,
    },
    StaleDiscarded,
    Rejected,
    Indeterminate,
    Failed,
}

impl ReviewStatus {
    pub(crate) fn title(self) -> String {
        match self {
            Self::Ready { revision } => format!("review:ready@{}", short_revision(revision)),
            Self::ProvisionalPick => "review:gpu-pick".to_owned(),
            Self::ConfirmingPick => "review:confirming".to_owned(),
            Self::SelectingScreen => "review:screen-scan".to_owned(),
            Self::Selected { revision, points } => {
                format!("review:{points} exact@{}", short_revision(revision))
            }
            Self::SelectionStale { revision, points } => format!(
                "review:stale {points} exact@{} rerun-or-clear",
                short_revision(revision)
            ),
            Self::Committed {
                revision,
                changed_points,
            } => format!(
                "review:committed {changed_points}@{}",
                short_revision(revision)
            ),
            Self::Reverted {
                revision,
                changed_points,
            } => format!(
                "review:reverted {changed_points}@{}",
                short_revision(revision)
            ),
            Self::CommittedUnverified {
                revision,
                changed_points,
            } => format!(
                "review:committed-unverified {changed_points}@{}",
                short_revision(revision)
            ),
            Self::StaleDiscarded => "review:stale-discarded".to_owned(),
            Self::Rejected => "review:rejected".to_owned(),
            Self::Indeterminate => "review:indeterminate".to_owned(),
            Self::Failed => "review:failed".to_owned(),
        }
    }
}

pub(crate) struct ReviewSession {
    workspace: Workspace,
    snapshot: Snapshot,
    options: ReviewOptions,
    selected: Option<ExactSelection>,
    pending: Option<PendingExactReview>,
    status: ReviewStatus,
    commit_operation_used: bool,
    revert_operation_used: bool,
}

impl ReviewSession {
    pub(crate) fn open(
        root: &Path,
        index: PreparedIndex,
        options: ReviewOptions,
    ) -> ReviewResult<Self> {
        let workspace = open(root, index, OpenLimits::default()).blocking_wait()?;
        let snapshot = workspace.head();
        let revision = snapshot.provenance().revision();
        println!(
            "Exact review Workspace opened\n  root: {}\n  Workspace: {}\n  Source: {}\n  head Revision: {}\n  policy: no automatic create, retry, repin, or Revert",
            root.display(),
            workspace.identity(),
            workspace.source(),
            revision,
        );
        if let Some(operation) = options.resolve_operation {
            print_operation_resolution(operation, workspace.resolve_operation(operation)?);
        }
        Ok(Self {
            workspace,
            snapshot,
            options,
            selected: None,
            pending: None,
            status: ReviewStatus::Ready { revision },
            commit_operation_used: false,
            revert_operation_used: false,
        })
    }

    pub(crate) const fn status(&self) -> ReviewStatus {
        self.status
    }

    pub(crate) fn close_if_indeterminate(
        session: &mut Option<Self>,
        disposition: MutationDisposition,
    ) -> bool {
        let MutationDisposition::Indeterminate { operation } = disposition else {
            return false;
        };
        let closed = session.take();
        debug_assert!(
            closed.is_some(),
            "an interactive mutation has a review session"
        );
        eprintln!(
            "Exact review session closed after indeterminate Operation {operation}; reopen explicitly with --resolve-operation-id {operation} before further review or mutation"
        );
        true
    }

    pub(crate) const fn is_busy(&self) -> bool {
        self.pending.is_some()
    }

    pub(crate) const fn has_classification_edit(&self) -> bool {
        self.options.classification_edit.is_some()
    }

    pub(crate) const fn has_revert(&self) -> bool {
        self.options.revert_operation.is_some()
    }

    pub(crate) fn capture(&self, view: CaptureView) -> ReviewCapture {
        ReviewCapture {
            snapshot: self.snapshot.clone(),
            provenance: *self.snapshot.provenance(),
            view,
        }
    }

    pub(crate) fn note_provisional_pick(&mut self) {
        self.status = ReviewStatus::ProvisionalPick;
    }

    pub(crate) fn note_pick_miss(&mut self) {
        println!(
            "Provisional GPU pick missed resident display samples; no exact Query was inferred"
        );
        self.status = ReviewStatus::Ready {
            revision: self.snapshot.provenance().revision(),
        };
    }

    pub(crate) fn is_capture_current(&self, capture: &ReviewCapture, active: CaptureView) -> bool {
        self.capture_is_current(capture, active)
    }

    pub(crate) fn confirm_provisional(
        &mut self,
        capture: ReviewCapture,
        hint: ProvisionalPickHint,
    ) -> ReviewResult<()> {
        self.require_idle()?;
        if hint.view_generation() != capture.view.view_generation {
            return Err(
                "provisional PickHit does not belong to its captured View generation".into(),
            );
        }
        let job = confirm_pick(
            &capture.snapshot,
            hint.point(),
            ScreenReviewLimits::default(),
        );
        self.pending = Some(PendingExactReview::Pick {
            capture,
            job: Box::pin(job),
        });
        self.status = ReviewStatus::ConfirmingPick;
        Ok(())
    }

    pub(crate) fn select_screen(
        &mut self,
        capture: ReviewCapture,
        camera: Camera,
        viewport: Viewport,
        first: [f64; 2],
        second: [f64; 2],
    ) -> ReviewResult<()> {
        self.require_idle()?;
        let rect = ScreenRect::new(first, second)?;
        let mut selection = ScreenSelection::new(rect, camera, viewport)?;
        if let Some(value) = self.options.classification_filter {
            selection = selection.classification_is(value);
        }
        let job = screen_through(&capture.snapshot, selection, ScreenReviewLimits::default());
        self.pending = Some(PendingExactReview::Screen {
            capture,
            job: Box::pin(job),
        });
        self.status = ReviewStatus::SelectingScreen;
        Ok(())
    }

    pub(crate) fn select_full_view_blocking(
        &mut self,
        camera: Camera,
        viewport: Viewport,
    ) -> ReviewResult<ExactHighlights> {
        let capture = self.capture(CaptureView::new(
            ViewGenerationKey::new(render_protocol::ViewId::new(1), 1),
            0,
        ));
        let rect = ScreenRect::new(
            [0.0, 0.0],
            [f64::from(viewport.width()), f64::from(viewport.height())],
        )?;
        let mut selection = ScreenSelection::new(rect, camera, viewport)?;
        if let Some(value) = self.options.classification_filter {
            selection = selection.classification_is(value);
        }
        let inspection =
            screen_through(&capture.snapshot, selection, ScreenReviewLimits::default())
                .blocking_wait()?;
        self.accept(
            CompletedReview::Screen {
                capture,
                inspection,
            },
            CaptureView::new(
                ViewGenerationKey::new(render_protocol::ViewId::new(1), 1),
                0,
            ),
        )
    }

    pub(crate) fn confirm_headless(&self, point: PointId) -> ReviewResult<ExactHighlights> {
        let confirmed =
            confirm_pick(&self.snapshot, point, ScreenReviewLimits::default()).blocking_wait()?;
        let highlights = ExactHighlights::from_point_set(confirmed.points())?;
        println!(
            "Exact CPU confirmation smoke\n  Revision: {}\n  Point: {:?}\n  ticks: {:?}\n  world: {:?}\n  effective classification: {}\n  exact Point Set: {}",
            confirmed.provenance().revision(),
            confirmed.point_id(),
            confirmed.ticks(),
            confirmed.world_position(),
            confirmed.effective_classification(),
            confirmed.points().metadata().exact_count(),
        );
        Ok(highlights)
    }

    pub(crate) fn poll(&mut self) -> Option<Result<CompletedReview, ReviewError>> {
        let pending = self.pending.as_mut()?;
        match pending {
            PendingExactReview::Pick { job, .. } => {
                let Poll::Ready(result) = poll_future(job.as_mut()) else {
                    return None;
                };
                let PendingExactReview::Pick { capture, .. } =
                    self.pending.take().expect("ready pick remains pending")
                else {
                    unreachable!();
                };
                Some(result.map(|confirmed| CompletedReview::Pick { capture, confirmed }))
            }
            PendingExactReview::Screen { job, .. } => {
                let Poll::Ready(result) = poll_future(job.as_mut()) else {
                    return None;
                };
                let PendingExactReview::Screen { capture, .. } =
                    self.pending.take().expect("ready screen remains pending")
                else {
                    unreachable!();
                };
                Some(result.map(|inspection| CompletedReview::Screen {
                    capture,
                    inspection,
                }))
            }
        }
    }

    pub(crate) fn accept(
        &mut self,
        completed: CompletedReview,
        active: CaptureView,
    ) -> ReviewResult<ExactHighlights> {
        let capture = match &completed {
            CompletedReview::Pick { capture, .. } | CompletedReview::Screen { capture, .. } => {
                capture
            }
        };
        if !self.capture_is_current(capture, active) {
            self.stale_exact_result_discarded(
                "exact CPU result no longer matches the active View or Revision",
            );
            return Err("exact review completed against stale View or Revision state".into());
        }

        let (capture, points, provenance, candidates, examined, point_id_hash, kind) =
            match completed {
                CompletedReview::Pick { capture, confirmed } => {
                    println!(
                        "CPU-confirmed exact Point\n  Point: {:?}\n  ticks: {:?}\n  world: {:?}\n  effective classification: {}",
                        confirmed.point_id(),
                        confirmed.ticks(),
                        confirmed.world_position(),
                        confirmed.effective_classification(),
                    );
                    (
                        capture,
                        confirmed.points().clone(),
                        confirmed.provenance(),
                        1,
                        1,
                        confirmed.points().metadata().point_id_hash(),
                        "CPU-confirmed provisional pick",
                    )
                }
                CompletedReview::Screen {
                    capture,
                    inspection,
                } => {
                    let summary = *inspection.summary();
                    let (points, _) = inspection.into_parts();
                    (
                        capture,
                        points,
                        summary.provenance(),
                        summary.candidate_point_count(),
                        summary.examined_point_count(),
                        summary.point_id_hash(),
                        "exact inclusive screen-through rectangle",
                    )
                }
            };
        let highlights = ExactHighlights::from_point_set(&points)?;
        let revision = provenance.revision();
        let exact_count = points.metadata().exact_count();
        println!(
            "Exact review complete\n  kind: {kind}\n  Revision: {revision}\n  candidates: {candidates}\n  examined: {examined}\n  exact Point Set: {exact_count}\n  Point ID hash: {point_id_hash:?}",
        );
        self.status = ReviewStatus::Selected {
            revision,
            points: exact_count,
        };
        self.selected = Some(ExactSelection {
            points,
            freshness: SelectionFreshness::from_capture(&capture),
        });
        Ok(highlights)
    }

    pub(crate) fn selected_highlights(&self) -> ReviewResult<Option<ExactHighlights>> {
        self.selected
            .as_ref()
            .map(|selection| ExactHighlights::from_point_set(&selection.points))
            .transpose()
    }

    pub(crate) fn invalidate_selection_view(&mut self, active: CaptureView) {
        let Some(selection) = self.selected.as_mut() else {
            return;
        };
        if !selection.freshness.invalidate(active) {
            return;
        }
        let revision = selection.points.metadata().provenance().revision();
        let points = selection.points.metadata().exact_count();
        println!(
            "Completed exact selection is now stale after View invalidation\n  Revision: {revision}\n  exact Points retained: {points}\n  action: rerun or clear the selection before commit"
        );
        self.status = ReviewStatus::SelectionStale { revision, points };
    }

    pub(crate) fn clear_selection(&mut self) {
        self.selected = None;
        self.status = ReviewStatus::Ready {
            revision: self.snapshot.provenance().revision(),
        };
    }

    pub(crate) fn commit_selected(
        &mut self,
        active: CaptureView,
    ) -> ReviewResult<MutationDisposition> {
        self.require_idle()?;
        let edit = self
            .options
            .classification_edit
            .ok_or("ground correction requires --operation-id and --classification")?;
        if self.commit_operation_used {
            return Err("the caller-owned --operation-id was already submitted".into());
        }
        let selection = self
            .selected
            .as_ref()
            .ok_or("ground correction requires a completed exact Point Set")?;
        if !selection.freshness.is_current(active) {
            self.show_stale_selection();
            return Err(
                "completed exact selection is stale; rerun or clear it before ground correction"
                    .into(),
            );
        }
        let points = selection.points.clone();
        if let Err(error) = self.require_current_point_set(&points) {
            self.show_stale_selection();
            return Err(error);
        }
        self.commit_operation_used = true;
        println!(
            "Ground correction requested\n  Operation: {}\n  explicit classification: {}\n  selected Points: {}",
            edit.operation,
            edit.value,
            points.metadata().exact_count(),
        );
        let outcome = self
            .workspace
            .commit(
                CommitRequest::set_classification(edit.operation, points, edit.value),
                CommitLimits::default(),
            )
            .blocking_wait()?;
        Ok(self.finish_mutation(MutationKind::Classification, edit.operation, outcome, None))
    }

    pub(crate) fn revert_head(&mut self) -> ReviewResult<MutationDisposition> {
        self.require_idle()?;
        let operation = self
            .options
            .revert_operation
            .ok_or("Revert requires --revert-operation-id")?;
        if self.revert_operation_used {
            return Err("the caller-owned --revert-operation-id was already submitted".into());
        }
        self.revert_operation_used = true;
        let expected_head = self.workspace.head().provenance().revision();
        let reverted_audit = self
            .workspace
            .revision_audit(expected_head, RevisionAuditLimits::default())
            .blocking_wait()?;
        println!(
            "Immediate-head Revert requested\n  Operation: {operation}\n  expected head: {expected_head}"
        );
        let outcome = self
            .workspace
            .commit(
                CommitRequest::revert_head(operation, expected_head),
                CommitLimits::default(),
            )
            .blocking_wait()?;
        Ok(self.finish_mutation(
            MutationKind::Revert,
            operation,
            outcome,
            Some(&reverted_audit),
        ))
    }

    pub(crate) fn fail(&mut self, error: &dyn std::fmt::Display) {
        eprintln!("exact review failed: {error}");
        self.status = ReviewStatus::Failed;
    }

    pub(crate) const fn is_stale(&self) -> bool {
        matches!(
            self.status,
            ReviewStatus::StaleDiscarded | ReviewStatus::SelectionStale { .. }
        )
    }

    pub(crate) fn stale_provisional_discarded(&mut self, reason: &str) {
        println!("Stale review result discarded without Source access or mutation: {reason}");
        self.status = ReviewStatus::StaleDiscarded;
    }

    fn stale_exact_result_discarded(&mut self, reason: &str) {
        println!(
            "Stale exact CPU result discarded after pinned Source access and without mutation: {reason}"
        );
        self.status = ReviewStatus::StaleDiscarded;
    }

    pub(crate) fn head_revision(&self) -> RevisionId {
        self.workspace.head().provenance().revision()
    }

    fn finish_mutation(
        &mut self,
        kind: MutationKind,
        operation: OperationId,
        outcome: CommitOutcome,
        reverted_audit: Option<&RevisionAudit>,
    ) -> MutationDisposition {
        match outcome {
            CommitOutcome::Committed(receipt) => {
                let revision = receipt.revision();
                let changed_points = receipt.revision_info().kind().changed_points();
                self.snapshot = self.workspace.head();
                let audit = self
                    .workspace
                    .revision_audit(revision, RevisionAuditLimits::default())
                    .blocking_wait();
                match audit.map_err(|error| error.to_string()).and_then(|audit| {
                    verify_mutation_audit(
                        operation,
                        revision,
                        changed_points,
                        &audit,
                        reverted_audit,
                    )?;
                    Ok(audit)
                }) {
                    Ok(audit) => {
                        self.status = kind.status(revision, changed_points);
                        print_audit(kind.label(), operation, &audit);
                        MutationDisposition::Committed {
                            operation,
                            revision,
                            audit_verified: true,
                        }
                    }
                    Err(error) => {
                        self.status = ReviewStatus::CommittedUnverified {
                            revision,
                            changed_points,
                        };
                        eprintln!(
                            "Workspace mutation is durably committed but its Revision Audit could not be verified\n  Operation: {operation}\n  Revision: {revision}\n  changed Points from receipt: {changed_points}\n  audit error: {error}\n  recovery state: committed; no automatic retry or dependent mutation"
                        );
                        MutationDisposition::Committed {
                            operation,
                            revision,
                            audit_verified: false,
                        }
                    }
                }
            }
            CommitOutcome::Rejected(reason) => {
                println!(
                    "Workspace mutation definitively rejected\n  Operation: {operation}\n  reason: {reason:?}\n  no automatic retry was attempted"
                );
                self.status = ReviewStatus::Rejected;
                MutationDisposition::Rejected { operation, reason }
            }
            CommitOutcome::Indeterminate(uncertainty) => {
                println!(
                    "Workspace mutation is indeterminate\n  Operation: {}\n  phase: {:?}\n  reason: {}\n  required action: close, reopen explicitly, and resolve this same Operation; no retry was attempted",
                    uncertainty.operation(),
                    uncertainty.phase(),
                    uncertainty.reason(),
                );
                self.status = ReviewStatus::Indeterminate;
                MutationDisposition::Indeterminate { operation }
            }
        }
    }

    fn show_stale_selection(&mut self) {
        let Some(selection) = self.selected.as_mut() else {
            return;
        };
        let captured = match selection.freshness {
            SelectionFreshness::Current(captured) | SelectionFreshness::Stale(captured) => captured,
        };
        selection.freshness = SelectionFreshness::Stale(captured);
        let revision = selection.points.metadata().provenance().revision();
        let points = selection.points.metadata().exact_count();
        self.status = ReviewStatus::SelectionStale { revision, points };
    }

    fn require_idle(&self) -> ReviewResult<()> {
        if self.pending.is_some() {
            return Err("one exact review is already running".into());
        }
        Ok(())
    }

    fn capture_is_current(&self, capture: &ReviewCapture, active: CaptureView) -> bool {
        capture.view == active
            && capture.provenance == *self.snapshot.provenance()
            && capture.provenance.revision() == self.workspace.head().provenance().revision()
    }

    fn require_current_point_set(&self, points: &PointSet) -> ReviewResult<()> {
        let expected = points.metadata().provenance();
        let actual = self.workspace.head().provenance().revision();
        if expected != *self.snapshot.provenance() || expected.revision() != actual {
            return Err(format!(
                "exact Point Set is stale: expected Revision {}, current head {actual}",
                expected.revision()
            )
            .into());
        }
        Ok(())
    }
}

pub(crate) fn reopen_head(root: &Path, index: PreparedIndex) -> ReviewResult<RevisionId> {
    let workspace = open(root, index, OpenLimits::default()).blocking_wait()?;
    let revision = workspace.head().provenance().revision();
    println!(
        "Exact review Workspace reopened without retry\n  Workspace: {}\n  head Revision: {revision}",
        workspace.identity(),
    );
    Ok(revision)
}

fn highlight_read_limits() -> PointIdReadLimits {
    PointIdReadLimits::new(
        MAX_HIGHLIGHT_POINTS,
        HIGHLIGHT_BATCH_POINTS,
        HIGHLIGHT_BATCH_BYTES,
        HIGHLIGHT_READ_BUFFER_BYTES,
        HIGHLIGHT_WORKING_BYTES,
    )
}

fn collect_highlight_ids_with_limits(
    points: &PointSet,
    limits: PointIdReadLimits,
    mut observer: impl FnMut(HighlightReadProgress),
) -> ReviewResult<Vec<PointId>> {
    let count = points.metadata().exact_count();
    if count > MAX_HIGHLIGHT_POINTS {
        return Err(format!(
            "exact Point Set contains {count} Points; renderer-demo highlight limit is {MAX_HIGHLIGHT_POINTS}"
        )
        .into());
    }
    let required_bytes = count
        .checked_mul(u64::try_from(mem::size_of::<PointId>()).unwrap_or(u64::MAX))
        .ok_or("highlight retained-byte accounting overflowed")?;
    if required_bytes > MAX_HIGHLIGHT_BYTES {
        return Err(format!(
            "exact highlight vector requires {required_bytes} bytes; limit is {MAX_HIGHLIGHT_BYTES}"
        )
        .into());
    }

    let capacity = usize::try_from(count).map_err(|_| "highlight count does not fit usize")?;
    let mut ids = Vec::new();
    ids.try_reserve_exact(capacity)
        .map_err(|_| "could not reserve bounded highlight vector")?;
    let allocated_bytes = u64::try_from(ids.capacity())
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(mem::size_of::<PointId>()).unwrap_or(u64::MAX));
    if allocated_bytes > MAX_HIGHLIGHT_BYTES {
        return Err(format!(
            "exact highlight vector allocated {allocated_bytes} bytes; limit is {MAX_HIGHLIGHT_BYTES}"
        )
        .into());
    }
    let mut batches = points.ids(limits)?;
    while let Some(batch) = batches.next()? {
        ids.extend_from_slice(batch.ids());
        observer(HighlightReadProgress::Batch {
            collected: u64::try_from(ids.len()).unwrap_or(u64::MAX),
        });
    }
    if u64::try_from(ids.len()).unwrap_or(u64::MAX) != count {
        return Err("Point Set identity iteration ended at the wrong exact count".into());
    }
    observer(HighlightReadProgress::Terminal { collected: count });
    Ok(ids)
}

fn poll_future<T, E>(future: Pin<&mut impl Future<Output = Result<T, E>>>) -> Poll<Result<T, E>> {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    future.poll(&mut context)
}

fn verify_mutation_audit(
    operation: OperationId,
    revision: RevisionId,
    changed_points: u64,
    audit: &RevisionAudit,
    reverted_audit: Option<&RevisionAudit>,
) -> Result<(), String> {
    if audit.revision().id() != revision || audit.provenance().revision() != revision {
        return Err("Revision Audit does not identify the committed Revision".to_owned());
    }
    if audit.revision().operation() != Some(operation) {
        return Err("Revision Audit does not identify the committed Operation".to_owned());
    }
    if audit.changed_point_count() != changed_points
        || audit.revision().kind().changed_points() != changed_points
    {
        return Err("Revision Audit changed-Point count disagrees with the receipt".to_owned());
    }
    if let Some(reverted_audit) = reverted_audit {
        verify_revert_audit(reverted_audit, audit)
    } else {
        Ok(())
    }
}

fn verify_revert_audit(
    reverted_audit: &RevisionAudit,
    revert_audit: &RevisionAudit,
) -> Result<(), String> {
    let reverted_revision = reverted_audit.revision();
    let revert_revision = revert_audit.revision();
    if reverted_audit.provenance().workspace() != revert_audit.provenance().workspace()
        || reverted_audit.provenance().source() != revert_audit.provenance().source()
    {
        return Err("Revert audit changed Workspace or Source identity".to_owned());
    }
    if reverted_audit.provenance().revision() != reverted_revision.id()
        || revert_audit.provenance().revision() != revert_revision.id()
    {
        return Err("Revert audit provenance disagrees with its Revision identity".to_owned());
    }
    if revert_revision.parent() != Some(reverted_revision.id()) {
        return Err("Revert Revision is not a child of the audited immediate head".to_owned());
    }
    if reverted_revision.sequence().checked_add(1) != Some(revert_revision.sequence()) {
        return Err("Revert Revision sequence does not immediately follow its parent".to_owned());
    }
    let RevisionKind::Revert {
        reverted_revision: target,
        changed_points,
    } = revert_revision.kind()
    else {
        return Err("committed Revision Audit is not a Revert".to_owned());
    };
    if target != reverted_revision.id() {
        return Err("Revert Revision targets a different Revision".to_owned());
    }
    if changed_points != reverted_audit.changed_point_count()
        || changed_points != revert_audit.changed_point_count()
    {
        return Err("Revert changed-Point count is not the exact inverse count".to_owned());
    }
    if reverted_audit.edit_footprint() != revert_audit.edit_footprint() {
        return Err("Revert Edit Footprint differs from the reverted Revision".to_owned());
    }
    if reverted_audit.point_id_hash() != revert_audit.point_id_hash() {
        return Err("Revert Point ID hash differs from the reverted Revision".to_owned());
    }

    let mut expected_transitions = reverted_audit
        .transitions()
        .iter()
        .map(|transition| (transition.after(), transition.before(), transition.count()))
        .collect::<Vec<_>>();
    let mut actual_transitions = revert_audit
        .transitions()
        .iter()
        .map(|transition| (transition.before(), transition.after(), transition.count()))
        .collect::<Vec<_>>();
    expected_transitions.sort_unstable();
    actual_transitions.sort_unstable();
    if expected_transitions != actual_transitions {
        return Err("Revert classification transitions are not exact inverses".to_owned());
    }
    if reverted_audit.content_hash().into_bytes() == [0; 32]
        || revert_audit.content_hash().into_bytes() == [0; 32]
        || reverted_audit.content_hash() == revert_audit.content_hash()
    {
        return Err("Revert content hashes do not distinguish both audited states".to_owned());
    }
    Ok(())
}

fn print_audit(label: &str, operation: OperationId, audit: &RevisionAudit) {
    let footprint = audit.edit_footprint().map_or_else(
        || "none".to_owned(),
        |bounds| format!("{:?} through {:?}", bounds.min(), bounds.max()),
    );
    let transitions = audit
        .transitions()
        .iter()
        .map(|transition| {
            format!(
                "{}->{}:{}",
                transition.before(),
                transition.after(),
                transition.count()
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "Workspace Revision Audit\n  kind: {label}\n  Operation: {operation}\n  Revision: {}\n  sequence: {}\n  changed Points: {}\n  transitions: [{}]\n  Edit Footprint: {footprint}\n  Point ID hash: {:?}\n  content hash: {:?}",
        audit.revision().id(),
        audit.revision().sequence(),
        audit.changed_point_count(),
        transitions,
        audit.point_id_hash(),
        audit.content_hash(),
    );
}

fn print_operation_resolution(
    operation: OperationId,
    resolution: point_workspace::OperationResolution,
) {
    use point_workspace::OperationResolution;

    match resolution {
        OperationResolution::Committed(receipt) => println!(
            "Explicit Operation resolution\n  Operation: {operation}\n  state: committed\n  Revision: {}\n  action: none",
            receipt.revision()
        ),
        OperationResolution::Rejected(rejected) => println!(
            "Explicit Operation resolution\n  Operation: {operation}\n  state: rejected\n  reason: {:?}\n  action: none",
            rejected.reason()
        ),
        OperationResolution::Retryable(intent) => println!(
            "Explicit Operation resolution\n  Operation: {operation}\n  state: retryable\n  proposed Revision: {}\n  action: explicit retry is available but renderer-demo did not perform it",
            intent.revision()
        ),
        OperationResolution::NotRecorded => println!(
            "Explicit Operation resolution\n  Operation: {operation}\n  state: not-recorded\n  action: none"
        ),
        OperationResolution::Indeterminate(uncertainty) => println!(
            "Explicit Operation resolution\n  Operation: {operation}\n  state: indeterminate\n  phase: {:?}\n  reason: {}\n  action: preserve the same Operation Identity; renderer-demo did not retry",
            uncertainty.phase(),
            uncertainty.reason()
        ),
    }
}

fn short_revision(revision: RevisionId) -> String {
    revision.to_string().chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, mem};

    use point_contracts::{
        AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
        AttributeValues, CoordinateReference, PositionTransform,
    };
    use point_index::{PrepareLimits, prepare};
    use point_workspace::{PointQuery, PointSetLimits, WorkspaceSchema, create};
    use render_protocol::{
        ESTIMATED_GPU_BYTES_PER_POINT, RenderLimits, RenderStateModel, RenderUpdate, SourceId,
        ViewId,
    };
    use source_memory::MemorySource;

    use super::*;

    #[test]
    fn completed_selection_becomes_stale_on_view_or_interaction_change() {
        let generation = ViewGenerationKey::new(ViewId::new(1), 7);
        let captured = CaptureView::new(generation, 3);
        let mut state = SelectionFreshness::Current(captured);

        assert!(state.is_current(captured));
        assert!(!state.invalidate(captured));
        assert!(state.is_current(captured));

        let moved = CaptureView::new(generation, 4);
        assert!(state.invalidate(moved));
        assert!(!state.is_current(captured));
        assert!(!state.is_current(moved));
        assert!(!state.invalidate(CaptureView::new(generation, 5)));
    }

    #[test]
    fn stale_provisional_view_or_interaction_is_rejected_before_exact_work() {
        let captured_view = ViewGenerationKey::new(ViewId::new(10), 3);
        let captured_interaction = 7;
        for (label, active_view, active_interaction) in [
            (
                "stale-provisional-view",
                ViewGenerationKey::new(ViewId::new(10), 4),
                captured_interaction,
            ),
            (
                "stale-provisional-interaction",
                captured_view,
                captured_interaction + 1,
            ),
        ] {
            let (mut session, _directory, _source) = review_session(label);
            let captured = CaptureView::new(captured_view, captured_interaction);
            let capture = session.capture(captured);
            assert!(session.is_capture_current(&capture, captured));

            reject_stale_provisional_without_exact_work(
                &mut session,
                &capture,
                CaptureView::new(active_view, active_interaction),
            );
        }
    }

    #[test]
    fn stale_provisional_snapshot_is_rejected_after_workspace_head_changes() {
        let (mut session, _directory, source) = review_session("stale-provisional-head");
        let view = ViewGenerationKey::new(ViewId::new(11), 2);
        let interaction = 5;
        let captured = CaptureView::new(view, interaction);
        let capture = session.capture(captured);
        assert!(session.is_capture_current(&capture, captured));

        let point_set = session
            .snapshot
            .select_point_ids([PointId::new(source, 0)], PointSetLimits::default())
            .blocking_wait()
            .unwrap();
        let outcome = session
            .workspace
            .commit(
                CommitRequest::set_classification(
                    OperationId::from_bytes([0x61; 16]).unwrap(),
                    point_set,
                    7,
                ),
                CommitLimits::default(),
            )
            .blocking_wait()
            .unwrap();
        let CommitOutcome::Committed(receipt) = outcome else {
            panic!("head-change fixture must commit one classification Revision");
        };
        assert_ne!(
            receipt.revision(),
            capture.provenance.revision(),
            "the captured Snapshot must become historical before stale-pick rejection"
        );

        reject_stale_provisional_without_exact_work(&mut session, &capture, captured);
    }

    #[test]
    fn stale_selection_title_requires_visible_rerun_or_clear() {
        let revision = RevisionId::from_bytes([9; 32]).unwrap();
        let title = ReviewStatus::SelectionStale {
            revision,
            points: 41,
        }
        .title();

        assert!(title.contains("stale 41 exact"));
        assert!(title.contains("rerun-or-clear"));
    }

    #[test]
    fn requested_headless_mutations_require_a_committed_disposition() {
        let operation = OperationId::from_bytes([4; 16]).unwrap();
        let revision = RevisionId::from_bytes([5; 32]).unwrap();
        assert!(
            MutationDisposition::Committed {
                operation,
                revision,
                audit_verified: true,
            }
            .require_committed("classification edit")
            .is_ok()
        );

        let missing_audit = MutationDisposition::Committed {
            operation,
            revision,
            audit_verified: false,
        }
        .require_committed("classification edit")
        .unwrap_err()
        .to_string();
        assert!(missing_audit.contains("committed as Revision"));
        assert!(missing_audit.contains("no dependent mutation"));

        let rejected = MutationDisposition::Rejected {
            operation,
            reason: CommitRejection::NoChanges,
        }
        .require_committed("classification edit")
        .unwrap_err()
        .to_string();
        assert!(rejected.contains("definitively rejected"));
        assert!(rejected.contains("NoChanges"));

        let indeterminate = MutationDisposition::Indeterminate { operation }
            .require_committed("immediate-head Revert")
            .unwrap_err()
            .to_string();
        assert!(indeterminate.contains("indeterminate"));
        assert!(indeterminate.contains("explicitly resolved"));
    }

    #[test]
    fn indeterminate_interactive_mutation_closes_the_complete_review_session() {
        let operation = OperationId::from_bytes([0x91; 16]).unwrap();
        let (session, _directory, _source) = review_session("indeterminate-close");
        let mut session = Some(session);

        assert!(!ReviewSession::close_if_indeterminate(
            &mut session,
            MutationDisposition::Rejected {
                operation,
                reason: CommitRejection::NoChanges,
            },
        ));
        assert!(session.is_some());
        assert!(ReviewSession::close_if_indeterminate(
            &mut session,
            MutationDisposition::Indeterminate { operation },
        ));
        assert!(session.is_none());
    }

    #[test]
    fn interactive_revert_reports_success_only_after_inverse_audit_verification() {
        let (mut session, _directory, source) = review_session("verified-revert");
        let point = PointId::new(source, 0);
        let points = session
            .workspace
            .head()
            .select_point_ids([point], PointSetLimits::default())
            .blocking_wait()
            .unwrap();
        let edit = session
            .workspace
            .commit(
                CommitRequest::set_classification(
                    OperationId::from_bytes([0x92; 16]).unwrap(),
                    points,
                    7,
                ),
                CommitLimits::default(),
            )
            .blocking_wait()
            .unwrap();
        let CommitOutcome::Committed(edit_receipt) = edit else {
            panic!("classification fixture must commit");
        };
        let operation = OperationId::from_bytes([0x93; 16]).unwrap();
        session.options.revert_operation = Some(operation);

        let disposition = session.revert_head().unwrap();

        let MutationDisposition::Committed {
            operation: actual_operation,
            revision,
            audit_verified,
        } = disposition
        else {
            panic!("verified immediate-head Revert must commit");
        };
        assert_eq!(actual_operation, operation);
        assert!(audit_verified);
        assert_eq!(session.snapshot.provenance().revision(), revision);
        assert_eq!(
            session.status,
            ReviewStatus::Reverted {
                revision,
                changed_points: 1,
            }
        );
        let revert_info = session.workspace.revision_info(revision).unwrap();
        assert_eq!(revert_info.parent(), Some(edit_receipt.revision()));
    }

    #[test]
    fn highlights_publish_only_after_terminal_exact_iteration_and_fail_atomically() {
        let fixture = ReviewFixture::new("atomic-highlights");
        let point_set = fixture
            .workspace
            .head()
            .select(PointQuery::all(), PointSetLimits::default())
            .blocking_wait()
            .unwrap();
        let view = ViewGenerationKey::new(ViewId::new(8), 3);
        let sentinel = PointId::new(fixture.source, 99);
        let mut model = RenderStateModel::new(
            RenderLimits::new(ESTIMATED_GPU_BYTES_PER_POINT, 1, 1)
                .with_max_highlight_points(MAX_HIGHLIGHT_POINTS),
        );
        model
            .apply(&RenderUpdate::Reset {
                view_generation: view,
            })
            .unwrap();
        model
            .apply(&RenderUpdate::SetHighlights {
                view_generation: view,
                point_ids: vec![sentinel],
            })
            .unwrap();

        let observed_state = RefCell::new(Vec::new());
        let completed = ExactHighlights::from_point_set_with_limits_and_observer(
            &point_set,
            PointIdReadLimits::new(4, 1, u64::MAX, u64::MAX, u64::MAX),
            |progress| {
                observed_state.borrow_mut().push(progress);
                assert_eq!(model.snapshot().highlights(), [sentinel]);
            },
        )
        .unwrap();
        assert_eq!(
            *observed_state.borrow(),
            [
                HighlightReadProgress::Batch { collected: 1 },
                HighlightReadProgress::Batch { collected: 2 },
                HighlightReadProgress::Batch { collected: 3 },
                HighlightReadProgress::Batch { collected: 4 },
                HighlightReadProgress::Terminal { collected: 4 },
            ]
        );

        let expected = collect_ids(&point_set, 2);
        assert_eq!(completed.as_slice(), expected);
        model
            .apply(&RenderUpdate::SetHighlights {
                view_generation: view,
                point_ids: completed.into_vec(),
            })
            .unwrap();
        assert_eq!(model.snapshot().highlights(), expected);

        let prior = model.snapshot();
        let too_few = PointIdReadLimits::new(3, 1, u64::MAX, u64::MAX, u64::MAX);
        assert!(
            ExactHighlights::from_point_set_with_limits_and_observer(
                &point_set,
                too_few,
                |_| panic!("limit failure must happen before any identity batch is exposed"),
            )
            .is_err()
        );
        assert_eq!(model.snapshot(), prior);

        let too_little_working = PointIdReadLimits::new(4, 4, u64::MAX, u64::MAX, 0);
        assert!(
            ExactHighlights::from_point_set_with_limits_and_observer(
                &point_set,
                too_little_working,
                |_| panic!("read failure must not expose a publishable ExactHighlights value"),
            )
            .is_err()
        );
        assert_eq!(model.snapshot(), prior);
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "the evidence intentionally keeps one Point identity visible through every public stage"
    )]
    fn one_identity_survives_pick_confirmation_spill_render_edit_audit_revert_and_reopen() {
        let fixture = ReviewFixture::new("identity-chain");
        let root = fixture.workspace.head();
        let root_revision = root.provenance().revision();
        let point = PointId::new(fixture.source, 2);
        let view = ViewGenerationKey::new(ViewId::new(9), 4);
        let hint = ProvisionalPickHint::faithful_public_seam(
            view,
            BatchKey::new(17),
            BatchVersion::new(3),
            point,
        );
        assert_eq!(hint.point(), point);
        assert_eq!(hint.view_generation(), view);

        let confirmed = confirm_pick(&root, hint.point(), ScreenReviewLimits::default())
            .blocking_wait()
            .unwrap();
        assert_eq!(confirmed.point_id(), point);
        assert_eq!(confirmed.ticks(), fixture.ticks[2]);
        assert_eq!(confirmed.provenance(), *root.provenance());
        assert_eq!(confirmed.effective_classification(), 2);
        let confirmed_ticks = confirmed.ticks();

        let resident = root
            .select_point_ids([point], PointSetLimits::default())
            .blocking_wait()
            .unwrap();
        let defaults = PointSetLimits::default();
        let forced_spill_limits = PointSetLimits::new(
            defaults.candidate_limits(),
            defaults.source_read_budget(),
            defaults.max_input_point_ids(),
            defaults.max_output_points(),
            defaults.max_overlay_segments(),
            defaults.max_overlay_bytes(),
            defaults.max_working_bytes(),
            0,
            defaults.max_temporary_bytes(),
        );
        let spilled = root
            .select_point_ids([point], forced_spill_limits)
            .blocking_wait()
            .unwrap();
        assert_eq!(confirmed.points().metadata(), resident.metadata());
        assert_eq!(resident.metadata(), spilled.metadata());
        assert_eq!(collect_ids(confirmed.points(), 1), [point]);
        assert_eq!(collect_ids(&resident, 1), [point]);
        assert_eq!(collect_ids(&spilled, 1), [point]);

        let mut renderer = RenderStateModel::new(
            RenderLimits::new(ESTIMATED_GPU_BYTES_PER_POINT, 1, 1).with_max_highlight_points(1),
        );
        renderer
            .apply(&RenderUpdate::Reset {
                view_generation: view,
            })
            .unwrap();
        let highlights = ExactHighlights::from_point_set(&spilled).unwrap();
        renderer
            .apply(&RenderUpdate::SetHighlights {
                view_generation: view,
                point_ids: highlights.into_vec(),
            })
            .unwrap();
        assert_eq!(renderer.snapshot().highlights(), [point]);

        let edit_operation = OperationId::from_bytes([0x31; 16]).unwrap();
        let edit = fixture
            .workspace
            .commit(
                CommitRequest::set_classification(edit_operation, spilled.clone(), 7),
                CommitLimits::default(),
            )
            .blocking_wait()
            .unwrap();
        let CommitOutcome::Committed(edit_receipt) = edit else {
            panic!("single changed Point must commit");
        };
        let edit_revision = edit_receipt.revision();
        assert_eq!(edit_receipt.operation(), edit_operation);
        assert_eq!(edit_receipt.revision_info().parent(), Some(root_revision));
        let edit_audit = fixture
            .workspace
            .revision_audit(edit_revision, RevisionAuditLimits::default())
            .blocking_wait()
            .unwrap();
        assert_eq!(edit_audit.provenance().source(), point.source());
        assert_eq!(edit_audit.changed_point_count(), 1);
        assert_eq!(edit_audit.transitions().len(), 1);
        assert_eq!(edit_audit.transitions()[0].before(), 2);
        assert_eq!(edit_audit.transitions()[0].after(), 7);
        assert_eq!(edit_audit.transitions()[0].count(), 1);
        assert_eq!(
            edit_audit.point_id_hash(),
            spilled.metadata().point_id_hash()
        );
        let footprint = edit_audit.edit_footprint().unwrap();
        let world = confirmed.world_position();
        assert_world_coordinates(footprint.min(), world);
        assert_world_coordinates(footprint.max(), world);
        assert_ne!(edit_audit.point_id_hash().into_bytes(), [0; 32]);
        assert_ne!(edit_audit.content_hash().into_bytes(), [0; 32]);

        let edited_snapshot = fixture.workspace.head();
        let edited = confirm_pick(&edited_snapshot, point, ScreenReviewLimits::default())
            .blocking_wait()
            .unwrap();
        assert_eq!(edited.point_id(), point);
        assert_eq!(edited.ticks(), confirmed_ticks);
        assert_eq!(edited.effective_classification(), 7);

        let revert_operation = OperationId::from_bytes([0x32; 16]).unwrap();
        let reverted = fixture
            .workspace
            .commit(
                CommitRequest::revert_head(revert_operation, edit_revision),
                CommitLimits::default(),
            )
            .blocking_wait()
            .unwrap();
        let CommitOutcome::Committed(revert_receipt) = reverted else {
            panic!("immediate-head Revert must commit");
        };
        assert_eq!(revert_receipt.operation(), revert_operation);
        assert_eq!(revert_receipt.revision_info().parent(), Some(edit_revision));
        let revert_audit = fixture
            .workspace
            .revision_audit(revert_receipt.revision(), RevisionAuditLimits::default())
            .blocking_wait()
            .unwrap();
        assert_eq!(revert_audit.changed_point_count(), 1);
        assert_eq!(revert_audit.transitions().len(), 1);
        assert_eq!(revert_audit.transitions()[0].before(), 7);
        assert_eq!(revert_audit.transitions()[0].after(), 2);
        assert_eq!(revert_audit.transitions()[0].count(), 1);
        assert_eq!(revert_audit.edit_footprint(), Some(footprint));
        assert_eq!(revert_audit.point_id_hash(), edit_audit.point_id_hash());
        assert_ne!(revert_audit.content_hash(), edit_audit.content_hash());
        verify_revert_audit(&edit_audit, &revert_audit).unwrap();
        assert!(verify_revert_audit(&revert_audit, &edit_audit).is_err());
        let terminal_revision = revert_receipt.revision();

        drop(renderer);
        drop(resident);
        drop(spilled);
        drop(edited);
        drop(edited_snapshot);
        drop(confirmed);
        drop(root);
        drop(fixture.workspace);
        let reopened = open(
            &fixture.workspace_path,
            fixture.index.clone(),
            OpenLimits::default(),
        )
        .blocking_wait()
        .unwrap();
        assert_eq!(reopened.head().provenance().revision(), terminal_revision);
        assert_eq!(
            reopened.revision_info(edit_revision).unwrap().parent(),
            Some(root_revision)
        );
        assert_eq!(
            reopened.revision_info(terminal_revision).unwrap().parent(),
            Some(edit_revision)
        );
        let reselected = reopened
            .head()
            .select_point_ids([point], forced_spill_limits)
            .blocking_wait()
            .unwrap();
        assert_eq!(collect_ids(&reselected, 1), [point]);
        let restored = confirm_pick(&reopened.head(), point, ScreenReviewLimits::default())
            .blocking_wait()
            .unwrap();
        assert_eq!(restored.point_id(), point);
        assert_eq!(restored.ticks(), confirmed_ticks);
        assert_eq!(restored.effective_classification(), 2);
    }

    fn collect_ids(points: &PointSet, batch_points: u64) -> Vec<PointId> {
        let point_bytes = u64::try_from(mem::size_of::<PointId>()).unwrap();
        let mut batches = points
            .ids(PointIdReadLimits::new(
                points.metadata().exact_count(),
                batch_points,
                batch_points.saturating_mul(point_bytes),
                4 * 1_024 * 1_024,
                8 * 1_024 * 1_024,
            ))
            .unwrap();
        let mut ids = Vec::new();
        while let Some(batch) = batches.next().unwrap() {
            ids.extend_from_slice(batch.ids());
        }
        ids
    }

    fn assert_world_coordinates(actual: [f64; 3], expected: [f64; 3]) {
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert!((actual - expected).abs() <= f64::EPSILON);
        }
    }

    fn review_session(label: &str) -> (ReviewSession, tempfile::TempDir, SourceId) {
        let ReviewFixture {
            workspace,
            _directory: directory,
            source,
            ..
        } = ReviewFixture::new(label);
        let snapshot = workspace.head();
        let revision = snapshot.provenance().revision();
        (
            ReviewSession {
                workspace,
                snapshot,
                options: ReviewOptions::default(),
                selected: None,
                pending: None,
                status: ReviewStatus::Ready { revision },
                commit_operation_used: false,
                revert_operation_used: false,
            },
            directory,
            source,
        )
    }

    fn reject_stale_provisional_without_exact_work(
        session: &mut ReviewSession,
        capture: &ReviewCapture,
        active: CaptureView,
    ) {
        let snapshot_before = *session.snapshot.provenance();
        let head_before = session.workspace.head().provenance().revision();
        assert!(!session.is_capture_current(capture, active));
        assert_no_exact_review_or_mutation_submission(session);

        session.stale_provisional_discarded(
            "test host rejected stale capture before CPU confirmation",
        );

        assert_eq!(session.status(), ReviewStatus::StaleDiscarded);
        assert_eq!(*session.snapshot.provenance(), snapshot_before);
        assert_eq!(
            session.workspace.head().provenance().revision(),
            head_before
        );
        assert_no_exact_review_or_mutation_submission(session);
    }

    fn assert_no_exact_review_or_mutation_submission(session: &ReviewSession) {
        assert!(session.pending.is_none());
        assert!(session.selected.is_none());
        assert!(!session.commit_operation_used);
        assert!(!session.revert_operation_used);
    }

    struct ReviewFixture {
        workspace: Workspace,
        index: PreparedIndex,
        workspace_path: std::path::PathBuf,
        _directory: tempfile::TempDir,
        source: SourceId,
        ticks: Vec<[i64; 3]>,
    }

    impl ReviewFixture {
        fn new(label: &str) -> Self {
            let directory = tempfile::Builder::new()
                .prefix(&format!("punctra-render-review-{label}-"))
                .tempdir()
                .unwrap();
            let classification = AttributeId::new(6).unwrap();
            let definition =
                AttributeDefinition::new(classification, "classification", AttributeDataType::U8)
                    .unwrap();
            let ticks = vec![[-3, 0, -5], [0, 0, -5], [3, 2, -5], [6, 4, -5]];
            let column =
                AttributeColumn::new(definition, AttributeValues::u8(vec![2, 2, 2, 2])).unwrap();
            let memory = MemorySource::from_columns(
                PositionTransform::new([500_000.0, 4_600_000.0, 120.0], [0.01; 3]).unwrap(),
                CoordinateReference::Unknown,
                ticks.clone(),
                AttributeColumns::new(vec![column], ticks.len()).unwrap(),
            )
            .unwrap();
            let source = source_memory::open(memory).blocking_wait().unwrap();
            let source_id = source.identity();
            let index = prepare(
                source,
                directory.path().join("fixture.pidx"),
                PrepareLimits::default(),
            )
            .blocking_wait()
            .unwrap();
            let workspace_path = directory.path().join("fixture.pcw");
            let workspace = create(
                &workspace_path,
                index.clone(),
                WorkspaceSchema::new(classification),
                OpenLimits::default(),
            )
            .blocking_wait()
            .unwrap();
            Self {
                workspace,
                index,
                workspace_path,
                _directory: directory,
                source: source_id,
                ticks,
            }
        }
    }
}
