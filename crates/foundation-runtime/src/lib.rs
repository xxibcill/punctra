//! Runtime-neutral execution control for bounded foundation work.
//!
//! This crate provides cooperative cancellation, monotonic progress, owned
//! background jobs, and a pull-based batch-stream contract. It depends only on
//! the standard library for execution and does not select an async runtime.
//!
//! # Example
//!
//! ```
//! use foundation_runtime::{Job, ProgressPhase, ProgressSnapshot, RuntimeError};
//!
//! let job = Job::<u64, RuntimeError>::spawn(|control| {
//!     let active = ProgressSnapshot::new(ProgressPhase::RUNNING, 1, Some(1))?;
//!     control.report_progress(active)?;
//!     control.complete_progress(1)?;
//!     Ok(42)
//! });
//! let handle = job.handle();
//!
//! assert_eq!(job.blocking_wait()?, 42);
//! assert_eq!(handle.progress().phase(), ProgressPhase::COMPLETE);
//! # Ok::<(), RuntimeError>(())
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::{
    fmt,
    future::Future,
    num::NonZeroU64,
    panic::{self, AssertUnwindSafe},
    pin::Pin,
    sync::{
        Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    task::{Context, Poll, Waker},
};

use thiserror::Error;

static NEXT_JOB_ID: AtomicU64 = AtomicU64::new(1);

/// Cooperative cancellation shared between callers and running work.
#[derive(Clone)]
pub struct CancellationToken {
    state: Arc<CancellationState>,
}

impl CancellationToken {
    /// Creates a token whose cancellation has not been requested.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation.
    ///
    /// Cancellation is idempotent and remains observable by every clone.
    pub fn cancel(&self) {
        self.state.cancelled.store(true, Ordering::Release);
    }

    /// Reports whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.state.is_cancelled()
    }

    /// Fails when cancellation has been requested.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::Cancelled`] after this token or any clone has
    /// been cancelled.
    pub fn check(&self) -> Result<(), RuntimeError> {
        if self.is_cancelled() {
            Err(RuntimeError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn link_parent(&self, parent: &Self) -> ParentCancellationLink {
        ParentCancellationLink::install(self, parent)
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self {
            state: Arc::new(CancellationState::default()),
        }
    }
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
struct CancellationState {
    cancelled: AtomicBool,
    parent: OnceLock<LinkedParentCancellation>,
}

impl CancellationState {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
            || self
                .parent
                .get()
                .is_some_and(LinkedParentCancellation::is_cancelled)
    }
}

struct LinkedParentCancellation {
    state: Weak<CancellationState>,
    active: AtomicBool,
}

impl LinkedParentCancellation {
    fn is_cancelled(&self) -> bool {
        self.active.load(Ordering::Acquire)
            && self
                .state
                .upgrade()
                .is_some_and(|parent| parent.cancelled.load(Ordering::Acquire))
    }
}

struct ParentCancellationLink {
    child: Weak<CancellationState>,
}

impl ParentCancellationLink {
    fn install(child: &CancellationToken, parent: &CancellationToken) -> Self {
        if Arc::ptr_eq(&child.state, &parent.state) {
            return Self { child: Weak::new() };
        }

        assert!(
            child
                .state
                .parent
                .set(LinkedParentCancellation {
                    state: Arc::downgrade(&parent.state),
                    active: AtomicBool::new(true),
                })
                .is_ok(),
            "a child cancellation token may link only one direct parent"
        );
        Self {
            child: Arc::downgrade(&child.state),
        }
    }
}

impl Drop for ParentCancellationLink {
    fn drop(&mut self) {
        let Some(child) = self.child.upgrade() else {
            return;
        };
        if let Some(parent) = child.parent.get() {
            parent.active.store(false, Ordering::Release);
        }
    }
}

/// Process-local identity of one operation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct JobId(NonZeroU64);

impl JobId {
    /// Returns the process-local nonzero identifier.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }

    fn next() -> Self {
        let value = NEXT_JOB_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(if current == u64::MAX { 1 } else { current + 1 })
            })
            .expect("the job identity generator always supplies a next value");
        Self(NonZeroU64::new(value).expect("the job identity generator never returns zero"))
    }
}

/// Ordered phase of one operation's progress.
///
/// Behavior modules may define named constants with [`ProgressPhase::new`]. A
/// reporter accepts the same phase or a higher phase and rejects regression.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProgressPhase(u16);

impl ProgressPhase {
    /// Initial phase used by a new operation.
    pub const PENDING: Self = Self(0);

    /// Conventional phase for active work.
    pub const RUNNING: Self = Self(1);

    /// Terminal success phase higher than every caller-defined phase.
    pub const COMPLETE: Self = Self(u16::MAX);

    /// Creates an ordered phase from a behavior-module-defined value.
    #[must_use]
    pub const fn new(order: u16) -> Self {
        Self(order)
    }

    /// Returns the phase's ordering value.
    #[must_use]
    pub const fn order(self) -> u16 {
        self.0
    }
}

/// Validated progress counters at one instant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProgressSnapshot {
    phase: ProgressPhase,
    completed_units: u64,
    total_units: Option<u64>,
}

impl ProgressSnapshot {
    /// Creates progress whose completed count does not exceed a known total.
    ///
    /// [`ProgressPhase::COMPLETE`] additionally requires a known total whose
    /// value exactly matches `completed_units`.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::InvalidProgress`] when `completed_units` is
    /// greater than `total_units`, or
    /// [`RuntimeError::IncompleteTerminalProgress`] when terminal progress
    /// does not exactly complete a known total.
    pub const fn new(
        phase: ProgressPhase,
        completed_units: u64,
        total_units: Option<u64>,
    ) -> Result<Self, RuntimeError> {
        if let Some(total_units) = total_units
            && completed_units > total_units
        {
            return Err(RuntimeError::InvalidProgress {
                completed_units,
                total_units,
            });
        }
        if phase.0 == ProgressPhase::COMPLETE.0 {
            match total_units {
                Some(total_units) if completed_units == total_units => {}
                _ => {
                    return Err(RuntimeError::IncompleteTerminalProgress {
                        completed_units,
                        total_units,
                    });
                }
            }
        }
        Ok(Self {
            phase,
            completed_units,
            total_units,
        })
    }

    /// Returns the ordered progress phase.
    #[must_use]
    pub const fn phase(self) -> ProgressPhase {
        self.phase
    }

    /// Returns completed work units.
    #[must_use]
    pub const fn completed_units(self) -> u64 {
        self.completed_units
    }

    /// Returns the known total work units, if one has been established.
    #[must_use]
    pub const fn total_units(self) -> Option<u64> {
        self.total_units
    }

    const fn pending() -> Self {
        Self {
            phase: ProgressPhase::PENDING,
            completed_units: 0,
            total_units: None,
        }
    }

    fn advances_from(self, previous: Self) -> bool {
        self.phase >= previous.phase
            && self.completed_units >= previous.completed_units
            && total_advances(previous.total_units, self.total_units)
    }
}

impl Default for ProgressSnapshot {
    fn default() -> Self {
        Self::pending()
    }
}

#[derive(Clone, Debug)]
struct ProgressReporter {
    progress: Arc<Mutex<ProgressSnapshot>>,
}

impl ProgressReporter {
    fn new() -> Self {
        Self {
            progress: Arc::new(Mutex::new(ProgressSnapshot::pending())),
        }
    }

    fn report(&self, attempted: ProgressSnapshot) -> Result<(), RuntimeError> {
        let mut current = lock_recovering(&self.progress);
        if attempted == *current {
            return Ok(());
        }
        if current.phase() == ProgressPhase::COMPLETE {
            return Err(RuntimeError::ProgressAlreadyComplete {
                completed: *current,
                attempted,
            });
        }
        if !attempted.advances_from(*current) {
            return Err(RuntimeError::ProgressRegression {
                previous: *current,
                attempted,
            });
        }
        *current = attempted;
        Ok(())
    }

    fn snapshot(&self) -> ProgressSnapshot {
        *lock_recovering(&self.progress)
    }
}

#[derive(Debug)]
struct OperationState {
    job_id: JobId,
    cancellation: CancellationToken,
    progress: ProgressReporter,
}

impl OperationState {
    fn new() -> Self {
        Self {
            job_id: JobId::next(),
            cancellation: CancellationToken::new(),
            progress: ProgressReporter::new(),
        }
    }
}

/// Cloneable caller capability for observing or cancelling one operation.
///
/// A handle deliberately cannot publish progress. Operation owners retain an
/// [`OperationControl`] and give producers an [`OperationReporter`].
#[derive(Clone, Debug)]
pub struct OperationHandle {
    state: Arc<OperationState>,
}

impl OperationHandle {
    /// Returns this operation's process-local identity.
    #[must_use]
    pub fn job_id(&self) -> JobId {
        self.state.job_id
    }

    /// Returns the latest progress snapshot.
    #[must_use]
    pub fn progress(&self) -> ProgressSnapshot {
        self.state.progress.snapshot()
    }

    /// Requests cooperative cancellation of this operation.
    pub fn cancel(&self) {
        self.state.cancellation.cancel();
    }

    /// Returns a clone of this operation's cancellation token.
    #[must_use]
    pub fn token(&self) -> CancellationToken {
        self.state.cancellation.clone()
    }

    /// Fails when cancellation has been requested.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::Cancelled`] after cancellation.
    pub fn check_cancelled(&self) -> Result<(), RuntimeError> {
        self.state.cancellation.check()
    }
}

/// Cloneable producer capability for active progress and cancellation checks.
///
/// A reporter cannot request cancellation or publish terminal progress. The
/// owning [`OperationControl`] alone decides when progress is complete.
#[derive(Clone, Debug)]
pub struct OperationReporter {
    state: Arc<OperationState>,
}

impl OperationReporter {
    /// Returns this operation's process-local identity.
    #[must_use]
    pub fn job_id(&self) -> JobId {
        self.state.job_id
    }

    /// Returns the latest progress snapshot.
    #[must_use]
    pub fn progress(&self) -> ProgressSnapshot {
        self.state.progress.snapshot()
    }

    /// Publishes monotonic non-terminal progress for this operation.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::TerminalProgressRequiresOwner`] for
    /// [`ProgressPhase::COMPLETE`]. Other invalid transitions return the
    /// corresponding progress-contract error.
    pub fn report_progress(&self, progress: ProgressSnapshot) -> Result<(), RuntimeError> {
        if progress.phase() == ProgressPhase::COMPLETE {
            return Err(RuntimeError::TerminalProgressRequiresOwner);
        }
        self.state.progress.report(progress)
    }

    /// Reports whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.state.cancellation.is_cancelled()
    }

    /// Fails when cancellation has been requested.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::Cancelled`] after cancellation.
    pub fn check_cancelled(&self) -> Result<(), RuntimeError> {
        self.state.cancellation.check()
    }
}

/// Unique owner capability for one operation's lifecycle.
///
/// Owners can derive restricted caller and producer capabilities, publish
/// active progress, and make the operation's sole terminal-progress decision.
#[derive(Debug)]
pub struct OperationControl {
    state: Arc<OperationState>,
}

impl OperationControl {
    /// Creates independent control for a new operation.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Arc::new(OperationState::new()),
        }
    }

    /// Returns a caller capability for observing and cancelling the operation.
    #[must_use]
    pub fn handle(&self) -> OperationHandle {
        OperationHandle {
            state: Arc::clone(&self.state),
        }
    }

    /// Returns a producer capability for active progress and cancellation.
    #[must_use]
    pub fn reporter(&self) -> OperationReporter {
        OperationReporter {
            state: Arc::clone(&self.state),
        }
    }

    /// Returns this operation's process-local identity.
    #[must_use]
    pub fn job_id(&self) -> JobId {
        self.state.job_id
    }

    /// Returns the latest progress snapshot.
    #[must_use]
    pub fn progress(&self) -> ProgressSnapshot {
        self.state.progress.snapshot()
    }

    /// Publishes monotonic non-terminal progress for this operation.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::TerminalProgressRequiresOwner`] when
    /// `progress` is terminal. Use [`Self::complete_progress`] for the owner's
    /// explicit terminal transition.
    pub fn report_progress(&self, progress: ProgressSnapshot) -> Result<(), RuntimeError> {
        self.reporter().report_progress(progress)
    }

    /// Publishes terminal progress with a coherent completed and known total.
    ///
    /// Repeating the same `total_units` after completion is idempotent. Every
    /// other report after completion is rejected.
    ///
    /// # Errors
    ///
    /// Returns a progress-contract error if `total_units` would regress a
    /// previously accepted counter or alter already-completed progress.
    pub fn complete_progress(&self, total_units: u64) -> Result<(), RuntimeError> {
        let completed =
            ProgressSnapshot::new(ProgressPhase::COMPLETE, total_units, Some(total_units))?;
        self.state.progress.report(completed)
    }

    /// Requests cooperative cancellation of this operation.
    pub fn cancel(&self) {
        self.state.cancellation.cancel();
    }

    /// Returns a clone of this operation's cancellation token.
    #[must_use]
    pub fn token(&self) -> CancellationToken {
        self.state.cancellation.clone()
    }

    /// Fails when cancellation has been requested.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::Cancelled`] after cancellation.
    pub fn check_cancelled(&self) -> Result<(), RuntimeError> {
        self.state.cancellation.check()
    }
}

impl Default for OperationControl {
    fn default() -> Self {
        Self::new()
    }
}

/// Runtime-neutral background work driven as a future or blocking wait.
///
/// Dropping an unfinished job requests cooperative cancellation but does not
/// block to join a worker that ignores cancellation.
pub struct Job<T, E> {
    handle: OperationHandle,
    shared: Arc<JobShared<T, E>>,
}

impl<T, E> Job<T, E>
where
    T: Send + 'static,
    E: From<RuntimeError> + Send + 'static,
{
    /// Starts one closure on an owned standard-library worker thread.
    ///
    /// Worker panics and worker-thread creation failures are returned as
    /// [`RuntimeError::WorkerPanicked`] through `E::from`.
    #[must_use]
    pub fn spawn<F>(work: F) -> Self
    where
        F: FnOnce(OperationControl) -> Result<T, E> + Send + 'static,
    {
        let control = OperationControl::new();
        let handle = control.handle();
        let shared = Arc::new(JobShared::new());
        let worker_shared = Arc::clone(&shared);
        let worker_name = format!("punctra-job-{}", handle.job_id().get());

        let spawn_result = std::thread::Builder::new()
            .name(worker_name)
            .spawn(move || run_worker(work, control, &worker_shared));
        if spawn_result.is_err() {
            shared.complete(Err(E::from(RuntimeError::WorkerPanicked)));
        }

        Self { handle, shared }
    }

    /// Returns a cloneable handle for observing or cancelling the job.
    #[must_use]
    pub fn handle(&self) -> OperationHandle {
        self.handle.clone()
    }

    /// Blocks the current thread until the job finishes.
    ///
    /// # Errors
    ///
    /// Returns the worker closure's error, including a converted
    /// [`RuntimeError`] when the worker panics or cannot be started.
    pub fn blocking_wait(self) -> Result<T, E> {
        self.shared.wait()
    }

    /// Blocks until the job finishes while observing one root cancellation token.
    ///
    /// The direct parent link exists only for this wait. Cancellation of
    /// `parent` becomes visible to the child worker's existing cooperative
    /// cancellation checks. Cancelling the child does not cancel `parent`, and
    /// the child retains its own progress lifecycle.
    ///
    /// A child that finishes before parent cancellation preserves its result.
    /// This method starts no watcher thread, timer, or async runtime.
    ///
    /// # Errors
    ///
    /// Returns the worker closure's error, including cooperative cancellation
    /// observed from the child or linked parent and converted runtime failures.
    pub fn blocking_wait_cancelled_by(self, parent: &CancellationToken) -> Result<T, E> {
        let _parent_link = self.handle.token().link_parent(parent);
        self.shared.wait()
    }
}

impl<T, E> Future for Job<T, E> {
    type Output = Result<T, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.shared.poll(cx)
    }
}

impl<T, E> fmt::Debug for Job<T, E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Job")
            .field("job_id", &self.handle.job_id())
            .field("progress", &self.handle.progress())
            .finish_non_exhaustive()
    }
}

impl<T, E> Drop for Job<T, E> {
    fn drop(&mut self) {
        self.shared.cancel_if_running(&self.handle);
    }
}

/// Pull-based bounded stream with a terminal success summary.
///
/// Implementations return zero or more bounded batches from [`Self::next`]. A
/// successful terminal `Ok(None)` makes [`Self::summary`] available and is
/// fused. An error, including cancellation, is returned once, leaves no
/// summary, and is also followed by fused `Ok(None)`. The observer handle lets
/// callers cancel without receiving progress-publishing authority.
pub trait BatchStream: Send {
    /// One bounded batch.
    type Batch;

    /// Exact facts available only after successful completion.
    type Summary;

    /// Error returned by stream production or cancellation.
    type Error: From<RuntimeError>;

    /// Pulls the next bounded batch, or terminal `None`.
    ///
    /// # Errors
    ///
    /// Returns the implementation's data, resource, or cancellation error.
    fn next(&mut self) -> Result<Option<Self::Batch>, Self::Error>;

    /// Returns exact terminal facts only after successful completion.
    #[must_use]
    fn summary(&self) -> Option<&Self::Summary>;

    /// Returns a caller capability for progress observation and cancellation.
    #[must_use]
    fn handle(&self) -> OperationHandle;
}

/// Runtime execution or progress-contract failure.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum RuntimeError {
    /// Cooperative cancellation was requested.
    #[error("operation was cancelled")]
    Cancelled,
    /// The worker panicked or could not be started.
    #[error("operation worker panicked or could not be started")]
    WorkerPanicked,
    /// Completed work exceeded the declared total.
    #[error("completed progress {completed_units} exceeds total {total_units}")]
    InvalidProgress {
        /// Invalid completed-unit count.
        completed_units: u64,
        /// Declared total-unit count.
        total_units: u64,
    },
    /// Terminal progress did not exactly complete a known total.
    #[error(
        "terminal progress completed {completed_units} units with incoherent total {total_units:?}"
    )]
    IncompleteTerminalProgress {
        /// Terminal completed-unit count.
        completed_units: u64,
        /// Missing or unequal declared total-unit count.
        total_units: Option<u64>,
    },
    /// A producer attempted the owner-only terminal transition.
    #[error("terminal progress can only be published by the operation owner")]
    TerminalProgressRequiresOwner,
    /// A published phase or counter moved backward.
    #[error("operation progress regressed from {previous:?} to {attempted:?}")]
    ProgressRegression {
        /// Last accepted progress.
        previous: ProgressSnapshot,
        /// Rejected progress.
        attempted: ProgressSnapshot,
    },
    /// Progress was changed after its terminal snapshot.
    #[error("operation progress is complete at {completed:?}; rejected {attempted:?}")]
    ProgressAlreadyComplete {
        /// Frozen terminal progress.
        completed: ProgressSnapshot,
        /// Rejected non-idempotent report.
        attempted: ProgressSnapshot,
    },
}

fn total_advances(previous: Option<u64>, attempted: Option<u64>) -> bool {
    match (previous, attempted) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(previous), Some(attempted)) => attempted >= previous,
    }
}

fn run_worker<T, E, F>(work: F, control: OperationControl, shared: &JobShared<T, E>)
where
    E: From<RuntimeError>,
    F: FnOnce(OperationControl) -> Result<T, E>,
{
    let result = panic::catch_unwind(AssertUnwindSafe(|| work(control)))
        .unwrap_or_else(|_| Err(E::from(RuntimeError::WorkerPanicked)));
    shared.complete(result);
}

struct JobShared<T, E> {
    state: Mutex<JobState<T, E>>,
    ready: Condvar,
}

impl<T, E> JobShared<T, E> {
    fn new() -> Self {
        Self {
            state: Mutex::new(JobState::Running { waker: None }),
            ready: Condvar::new(),
        }
    }

    fn complete(&self, result: Result<T, E>) {
        let waker = {
            let mut state = lock_recovering(&self.state);
            let waker = match &mut *state {
                JobState::Running { waker } => waker.take(),
                JobState::Complete(_) | JobState::Consumed => None,
            };
            *state = JobState::Complete(Some(result));
            waker
        };
        self.ready.notify_all();
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    fn cancel_if_running(&self, handle: &OperationHandle) {
        let state = lock_recovering(&self.state);
        if matches!(&*state, JobState::Running { .. }) {
            handle.cancel();
        }
    }

    fn wait(&self) -> Result<T, E> {
        let mut state = lock_recovering(&self.state);
        loop {
            if let Some(result) = take_result(&mut state) {
                return result;
            }
            state = wait_recovering(&self.ready, state);
        }
    }

    fn poll(&self, cx: &Context<'_>) -> Poll<Result<T, E>> {
        let mut state = lock_recovering(&self.state);
        if let Some(result) = take_result(&mut state) {
            return Poll::Ready(result);
        }

        let JobState::Running { waker } = &mut *state else {
            panic!("a completed Job must not be polled again");
        };
        if waker
            .as_ref()
            .is_none_or(|registered| !registered.will_wake(cx.waker()))
        {
            *waker = Some(cx.waker().clone());
        }
        Poll::Pending
    }
}

enum JobState<T, E> {
    Running { waker: Option<Waker> },
    Complete(Option<Result<T, E>>),
    Consumed,
}

fn take_result<T, E>(state: &mut JobState<T, E>) -> Option<Result<T, E>> {
    let JobState::Complete(result) = state else {
        return None;
    };
    let result = result
        .take()
        .expect("a completed Job retains its result until first observation");
    *state = JobState::Consumed;
    Some(result)
}

fn lock_recovering<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn wait_recovering<'a, T>(condvar: &Condvar, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
    condvar
        .wait(guard)
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
