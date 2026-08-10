//! Interface-level execution, progress, and cancellation contracts.

use std::{collections::VecDeque, sync::mpsc, time::Duration};

use foundation_runtime::{
    BatchStream, CancellationToken, Job, OperationControl, OperationHandle, ProgressPhase,
    ProgressSnapshot, RuntimeError,
};

#[test]
fn blocking_job_returns_success_and_progress() {
    let job = Job::<u64, RuntimeError>::spawn(|control| {
        control.report_progress(ProgressSnapshot::new(ProgressPhase::RUNNING, 1, Some(1))?)?;
        control.complete_progress(1)?;
        Ok(42)
    });
    let handle: OperationHandle = job.handle();

    assert_eq!(job.blocking_wait(), Ok(42));
    assert_eq!(handle.progress().phase(), ProgressPhase::COMPLETE);
    assert_eq!(handle.progress().completed_units(), 1);
}

#[test]
fn job_wakes_a_standard_future_executor() {
    let job = Job::<u64, RuntimeError>::spawn(|_| Ok(42));
    let handle = job.handle();

    assert_eq!(pollster::block_on(job), Ok(42));
    assert!(!handle.token().is_cancelled());
}

#[test]
fn cancellation_is_shared_with_running_work() {
    let (started_sender, started_receiver) = mpsc::sync_channel(0);
    let job = Job::<(), RuntimeError>::spawn(move |control| {
        started_sender
            .send(())
            .expect("the test should still be waiting for startup");
        loop {
            control.check_cancelled()?;
            std::thread::park_timeout(Duration::from_millis(1));
        }
    });
    let handle = job.handle();
    started_receiver
        .recv()
        .expect("the worker should report startup");

    handle.cancel();

    assert_eq!(job.blocking_wait(), Err(RuntimeError::Cancelled));
    assert!(handle.token().is_cancelled());
}

#[test]
fn parent_cancelled_before_link_reaches_the_active_child() {
    let parent = CancellationToken::new();
    parent.cancel();
    let job = Job::<(), RuntimeError>::spawn(|control| {
        loop {
            control.check_cancelled()?;
            std::thread::park_timeout(Duration::from_millis(1));
        }
    });

    assert_eq!(
        job.blocking_wait_cancelled_by(&parent),
        Err(RuntimeError::Cancelled)
    );
}

#[test]
fn parent_cancellation_reaches_a_linked_active_child() {
    let parent = CancellationToken::new();
    let waiter_parent = parent.clone();
    let (started_sender, started_receiver) = mpsc::sync_channel(0);
    let job = Job::<(), RuntimeError>::spawn(move |control| {
        started_sender
            .send(())
            .expect("the cancellation test should still be waiting");
        loop {
            control.check_cancelled()?;
            std::thread::park_timeout(Duration::from_millis(1));
        }
    });
    let waiter = std::thread::spawn(move || job.blocking_wait_cancelled_by(&waiter_parent));
    started_receiver
        .recv()
        .expect("the child should report startup");

    parent.cancel();

    assert_eq!(
        waiter.join().expect("the blocking waiter should not panic"),
        Err(RuntimeError::Cancelled)
    );
}

#[test]
fn child_cancellation_does_not_cancel_its_parent() {
    let parent = CancellationToken::new();
    let job = Job::<(), RuntimeError>::spawn(|control| {
        loop {
            control.check_cancelled()?;
            std::thread::park_timeout(Duration::from_millis(1));
        }
    });
    let child = job.handle();

    child.cancel();

    assert_eq!(
        job.blocking_wait_cancelled_by(&parent),
        Err(RuntimeError::Cancelled)
    );
    assert!(!parent.is_cancelled());
}

#[test]
fn parent_cancellation_after_child_success_is_harmless() {
    let parent = CancellationToken::new();
    let waiter_parent = parent.clone();
    let (release_sender, release_receiver) = mpsc::sync_channel(0);
    let job = Job::<u64, RuntimeError>::spawn(move |control| {
        release_receiver
            .recv()
            .expect("the success test should release its child");
        control.complete_progress(1)?;
        Ok(42)
    });
    let child = job.handle();
    let waiter = std::thread::spawn(move || job.blocking_wait_cancelled_by(&waiter_parent));

    release_sender
        .send(())
        .expect("the child should still be waiting for release");
    assert_eq!(
        waiter.join().expect("the blocking waiter should not panic"),
        Ok(42)
    );
    parent.cancel();

    assert!(!child.token().is_cancelled());
    assert_eq!(child.progress().phase(), ProgressPhase::COMPLETE);
}

#[test]
fn linked_child_progress_remains_independent() {
    let parent = OperationControl::new();
    let parent_progress = ProgressSnapshot::new(ProgressPhase::new(7), 2, Some(5))
        .expect("parent fixture progress should be coherent");
    parent
        .report_progress(parent_progress)
        .expect("parent fixture progress should advance");
    let job = Job::<u64, RuntimeError>::spawn(|control| {
        control.complete_progress(1)?;
        Ok(42)
    });
    let child = job.handle();

    assert_eq!(job.blocking_wait_cancelled_by(&parent.token()), Ok(42));
    assert_eq!(parent.progress(), parent_progress);
    assert_eq!(child.progress().phase(), ProgressPhase::COMPLETE);
}

#[test]
fn dropping_a_running_job_requests_cancellation() {
    let (started_sender, started_receiver) = mpsc::sync_channel(0);
    let (cancelled_sender, cancelled_receiver) = mpsc::sync_channel(0);
    let job = Job::<(), RuntimeError>::spawn(move |control| {
        started_sender
            .send(())
            .expect("the test should still be waiting for startup");
        loop {
            if let Err(error) = control.check_cancelled() {
                cancelled_sender
                    .send(())
                    .expect("the test should still observe cancellation");
                return Err(error);
            }
            std::thread::park_timeout(Duration::from_millis(1));
        }
    });
    let handle = job.handle();
    started_receiver
        .recv()
        .expect("the worker should report startup");

    drop(job);

    cancelled_receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("a dropped running Job should cancel its worker");
    assert!(handle.token().is_cancelled());
}

#[test]
fn progress_rejects_invalid_and_regressing_snapshots() {
    let owner = OperationControl::new();
    let reporter = owner.reporter();
    let accepted = ProgressSnapshot::new(ProgressPhase::new(4), 5, Some(10))
        .expect("fixture progress should be valid");
    reporter
        .report_progress(accepted)
        .expect("forward progress should be accepted");

    let regressing = ProgressSnapshot::new(ProgressPhase::new(4), 4, Some(10))
        .expect("the individual snapshot is valid");
    assert_eq!(
        reporter.report_progress(regressing),
        Err(RuntimeError::ProgressRegression {
            previous: accepted,
            attempted: regressing,
        })
    );

    let earlier_phase = ProgressSnapshot::new(ProgressPhase::new(3), 5, Some(10))
        .expect("the individual snapshot is valid");
    assert!(matches!(
        reporter.report_progress(earlier_phase),
        Err(RuntimeError::ProgressRegression { .. })
    ));

    let smaller_total = ProgressSnapshot::new(ProgressPhase::new(4), 5, Some(9))
        .expect("the individual snapshot is valid");
    assert!(matches!(
        reporter.report_progress(smaller_total),
        Err(RuntimeError::ProgressRegression { .. })
    ));

    assert_eq!(
        ProgressSnapshot::new(ProgressPhase::RUNNING, 11, Some(10)),
        Err(RuntimeError::InvalidProgress {
            completed_units: 11,
            total_units: 10,
        })
    );

    assert_eq!(
        ProgressSnapshot::new(ProgressPhase::COMPLETE, 9, Some(10)),
        Err(RuntimeError::IncompleteTerminalProgress {
            completed_units: 9,
            total_units: Some(10),
        })
    );
    assert_eq!(
        ProgressSnapshot::new(ProgressPhase::COMPLETE, 10, None),
        Err(RuntimeError::IncompleteTerminalProgress {
            completed_units: 10,
            total_units: None,
        })
    );
}

#[test]
fn completed_progress_is_frozen_but_exact_repetition_is_idempotent() {
    let owner = OperationControl::new();
    let reporter = owner.reporter();
    let complete = ProgressSnapshot::new(ProgressPhase::COMPLETE, 10, Some(10))
        .expect("coherent terminal progress should be valid");

    owner
        .complete_progress(10)
        .expect("first terminal report should be accepted");
    owner
        .complete_progress(10)
        .expect("the exact terminal report should be idempotent");

    let altered = ProgressSnapshot::new(ProgressPhase::COMPLETE, 11, Some(11))
        .expect("the individual terminal snapshot should be valid");
    assert_eq!(
        owner.complete_progress(11),
        Err(RuntimeError::ProgressAlreadyComplete {
            completed: complete,
            attempted: altered,
        })
    );

    let active = ProgressSnapshot::new(ProgressPhase::RUNNING, 10, Some(10))
        .expect("the individual active snapshot should be valid");
    assert_eq!(
        reporter.report_progress(active),
        Err(RuntimeError::ProgressAlreadyComplete {
            completed: complete,
            attempted: active,
        })
    );
}

#[test]
fn producer_reporter_cannot_complete_owner_progress() {
    let owner = OperationControl::new();
    let producer = owner.reporter();
    let active = ProgressSnapshot::new(ProgressPhase::RUNNING, 1, Some(1))
        .expect("fixture progress should be valid");
    producer
        .report_progress(active)
        .expect("a producer may publish active progress");

    let complete = ProgressSnapshot::new(ProgressPhase::COMPLETE, 1, Some(1))
        .expect("coherent terminal progress should be valid");
    assert_eq!(
        producer.report_progress(complete),
        Err(RuntimeError::TerminalProgressRequiresOwner)
    );

    owner
        .complete_progress(1)
        .expect("the owner may complete progress");
    owner
        .complete_progress(1)
        .expect("owner completion should be idempotent");
    assert_eq!(owner.handle().progress(), complete);
}

#[test]
fn worker_panic_is_mapped_to_a_runtime_error() {
    let job = Job::<(), RuntimeError>::spawn(|_| panic!("injected worker panic"));

    assert_eq!(job.blocking_wait(), Err(RuntimeError::WorkerPanicked));
}

#[test]
fn independent_controls_receive_distinct_job_ids() {
    let first = OperationControl::new();
    let second = OperationControl::new();

    assert_ne!(first.job_id(), second.job_id());
}

#[test]
fn batch_stream_exposes_batches_then_summary_and_an_observer_handle() {
    let owner = OperationControl::new();
    let expected_job = owner.job_id();
    let mut stream = FixtureStream {
        handle: owner.handle(),
        batches: VecDeque::from([2_u64, 3]),
        emitted: 0,
        summary: None,
        terminal: false,
    };

    assert_eq!(stream.handle().job_id(), expected_job);
    assert_eq!(stream.summary(), None);
    assert_eq!(stream.next(), Ok(Some(2)));
    assert_eq!(stream.next(), Ok(Some(3)));
    assert_eq!(stream.summary(), None);
    assert_eq!(stream.next(), Ok(None));
    assert_eq!(stream.summary(), Some(&5));
    assert_eq!(stream.next(), Ok(None));
}

struct FixtureStream {
    handle: OperationHandle,
    batches: VecDeque<u64>,
    emitted: u64,
    summary: Option<u64>,
    terminal: bool,
}

impl BatchStream for FixtureStream {
    type Batch = u64;
    type Summary = u64;
    type Error = RuntimeError;

    fn next(&mut self) -> Result<Option<Self::Batch>, Self::Error> {
        if self.terminal {
            return Ok(None);
        }
        self.handle.check_cancelled()?;
        if let Some(batch) = self.batches.pop_front() {
            self.emitted += batch;
            return Ok(Some(batch));
        }
        self.summary = Some(self.emitted);
        self.terminal = true;
        Ok(None)
    }

    fn summary(&self) -> Option<&Self::Summary> {
        self.summary.as_ref()
    }

    fn handle(&self) -> OperationHandle {
        self.handle.clone()
    }
}
