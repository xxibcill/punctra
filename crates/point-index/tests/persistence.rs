//! Complete-artifact validation and append-only recovery conformance.

mod support;

use std::{
    fs::{self, OpenOptions},
    io::{Seek, SeekFrom, Write},
    thread,
    time::{Duration, Instant},
};

use foundation_runtime::RuntimeError;
use point_index::{
    IndexError, IndexLimit, NodeReadBudget, PrepareDisposition, PrepareLimits, prepare,
};
use point_source::ReadLimit;

use support::{
    BLOCK_POINTS, TemporaryTarget, clustered_ticks, open_controlled_source, open_source, read_node,
    samples,
};

#[test]
fn complete_artifacts_open_warm_and_reject_incompatible_corrupt_or_truncated_bytes() {
    let ticks = clustered_ticks(BLOCK_POINTS + 11);
    let source = open_source(ticks.clone());
    let target = TemporaryTarget::new("complete-validation");
    let built = prepare(source.clone(), target.path(), PrepareLimits::default())
        .blocking_wait()
        .unwrap();
    let original = fs::read(target.path()).unwrap();

    let opened = prepare(source.clone(), target.path(), PrepareLimits::default())
        .blocking_wait()
        .unwrap();
    assert_eq!(
        opened.prepare_report().disposition(),
        PrepareDisposition::Opened
    );
    assert_eq!(opened.prepare_report().source_points_read(), 0);
    assert_eq!(opened.prepare_report().durable_points_reused(), 0);
    assert_eq!(opened.descriptor(), built.descriptor());
    assert_eq!(opened.hierarchy(), built.hierarchy());
    assert_eq!(fs::read(target.path()).unwrap(), original);

    let mut changed_ticks = ticks;
    changed_ticks[0][2] += 1;
    let incompatible_source = open_source(changed_ticks);
    assert!(matches!(
        prepare(incompatible_source, target.path(), PrepareLimits::default()).blocking_wait(),
        Err(IndexError::IncompatibleArtifact { .. })
    ));
    assert_eq!(fs::read(target.path()).unwrap(), original);

    let corrupt = target.copied_target("corrupt.pidx");
    fs::write(&corrupt, &original).unwrap();
    let mut corrupt_bytes = original.clone();
    let changed = corrupt_bytes.len() / 2;
    corrupt_bytes[changed] ^= 0x80;
    fs::write(&corrupt, &corrupt_bytes).unwrap();
    assert!(matches!(
        prepare(source.clone(), &corrupt, PrepareLimits::default()).blocking_wait(),
        Err(IndexError::CorruptArtifact { .. })
    ));
    assert_eq!(fs::read(&corrupt).unwrap(), corrupt_bytes);

    let truncated = target.copied_target("truncated.pidx");
    fs::write(&truncated, &original[..original.len() - 1]).unwrap();
    assert!(matches!(
        prepare(source.clone(), &truncated, PrepareLimits::default()).blocking_wait(),
        Err(IndexError::CorruptArtifact { .. })
    ));
    assert_eq!(
        fs::metadata(&truncated).unwrap().len(),
        u64::try_from(original.len() - 1).unwrap()
    );

    let artifact_limit = PrepareLimits::default()
        .with_max_artifact_bytes(u64::try_from(original.len() - 1).unwrap());
    assert_resource_error(&prepare(source.clone(), target.path(), artifact_limit).blocking_wait());
    let hierarchy_limit = PrepareLimits::default().with_max_hierarchy_nodes(0);
    assert_resource_error(&prepare(source.clone(), target.path(), hierarchy_limit).blocking_wait());
    let metadata_limit = PrepareLimits::default().with_max_resident_metadata_bytes(0);
    assert_resource_error(&prepare(source.clone(), target.path(), metadata_limit).blocking_wait());
    let verification_limit = PrepareLimits::default().with_max_build_working_bytes(65_535);
    assert_resource_error(&prepare(source, target.path(), verification_limit).blocking_wait());
    assert_eq!(fs::read(target.path()).unwrap(), original);
}

#[test]
fn mutation_after_open_is_detected_by_the_node_sample_checksum() {
    let source = open_source(clustered_ticks(BLOCK_POINTS + 1));
    let target = TemporaryTarget::new("post-open-mutation");
    let index = prepare(source, target.path(), PrepareLimits::default())
        .blocking_wait()
        .unwrap();
    let root = index.hierarchy().root().unwrap();
    assert!(!root.coverage_complete());
    let before = read_node(&index, root.id(), NodeReadBudget::default());
    let sample_facts = samples(&before);
    let needle = encode_sample_pair(&sample_facts[..2]);
    let artifact = fs::read(target.path()).unwrap();
    let offset = artifact
        .windows(needle.len())
        .position(|candidate| candidate == needle)
        .expect("root sample pair occurs in the artifact");

    let mutation_offset = u64::try_from(offset + 16).unwrap();
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(target.path())
        .unwrap();
    file.seek(SeekFrom::Start(mutation_offset)).unwrap();
    let original_byte = artifact[usize::try_from(mutation_offset).unwrap()];
    file.write_all(&[original_byte ^ 0x40]).unwrap();
    file.sync_data().unwrap();

    assert!(matches!(
        index.read_node(root.id(), NodeReadBudget::default()),
        Err(IndexError::CorruptArtifact { .. })
    ));

    file.set_len(mutation_offset + 1).unwrap();
    file.sync_data().unwrap();
    assert!(matches!(
        index.read_node(root.id(), NodeReadBudget::default()),
        Err(IndexError::CorruptArtifact { .. })
    ));
}

#[test]
fn cold_build_limits_fail_without_a_partial_target_and_preserve_only_valid_work() {
    assert!(matches!(
        PrepareLimits::new(0, 24),
        Err(IndexError::InvalidLimit {
            limit: IndexLimit::MaxSourceBatchPoints
        })
    ));
    assert!(matches!(
        PrepareLimits::new(1, 0),
        Err(IndexError::InvalidLimit {
            limit: IndexLimit::MaxSourceBatchPayloadBytes
        })
    ));

    let one_point = open_source(clustered_ticks(1));
    let incomplete_header = TemporaryTarget::new("limit-incomplete-header");
    let limit = PrepareLimits::default().with_max_incomplete_bytes(199);
    assert!(matches!(
        prepare(one_point.clone(), incomplete_header.path(), limit).blocking_wait(),
        Err(IndexError::ResourceLimit {
            limit: IndexLimit::IncompleteIndexBytes,
            required: 200,
            allowed: 199,
        })
    ));
    assert!(!incomplete_header.path().exists());
    assert!(!incomplete_header.work_path().exists());

    let build_memory = TemporaryTarget::new("limit-build-memory-early");
    let limit = PrepareLimits::default().with_max_build_working_bytes(199);
    assert_resource_error(&prepare(one_point.clone(), build_memory.path(), limit).blocking_wait());
    assert!(!build_memory.path().exists());
    assert!(!build_memory.work_path().exists());

    let incomplete_frame = TemporaryTarget::new("limit-incomplete-frame");
    let limit = PrepareLimits::default().with_max_incomplete_bytes(200);
    assert_resource_error(
        &prepare(one_point.clone(), incomplete_frame.path(), limit).blocking_wait(),
    );
    assert!(!incomplete_frame.path().exists());
    assert_eq!(
        fs::metadata(incomplete_frame.work_path()).unwrap().len(),
        200
    );

    let source_payload = TemporaryTarget::new("limit-source-payload");
    let limit = PrepareLimits::new(1, 23).unwrap();
    assert!(matches!(
        prepare(one_point.clone(), source_payload.path(), limit).blocking_wait(),
        Err(IndexError::Source(
            point_source::SourceError::ResourceLimit {
                limit: ReadLimit::BatchPayloadBytes,
                required: 24,
                allowed: 23,
            }
        ))
    ));
    assert!(!source_payload.path().exists());
    assert_eq!(fs::metadata(source_payload.work_path()).unwrap().len(), 200);

    let artifact = TemporaryTarget::new("limit-cold-artifact");
    let limit = PrepareLimits::default().with_max_artifact_bytes(400);
    assert_resource_error(&prepare(one_point.clone(), artifact.path(), limit).blocking_wait());
    assert!(!artifact.path().exists());
    assert!(fs::metadata(artifact.work_path()).unwrap().len() > 200);
    let resumed = prepare(one_point, artifact.path(), PrepareLimits::default())
        .blocking_wait()
        .unwrap();
    assert_eq!(
        resumed.prepare_report().disposition(),
        PrepareDisposition::Resumed
    );
    assert_eq!(resumed.prepare_report().durable_points_reused(), 1);

    let late_source = open_source(clustered_ticks(BLOCK_POINTS + 1));
    let late_build = TemporaryTarget::new("limit-build-memory-late");
    let limit = PrepareLimits::default().with_max_build_working_bytes(350_000);
    assert_resource_error(&prepare(late_source.clone(), late_build.path(), limit).blocking_wait());
    assert!(!late_build.path().exists());
    assert!(fs::metadata(late_build.work_path()).unwrap().len() > 200);
    let resumed = prepare(late_source, late_build.path(), PrepareLimits::default())
        .blocking_wait()
        .unwrap();
    assert_eq!(
        resumed.prepare_report().disposition(),
        PrepareDisposition::Resumed
    );
    assert_eq!(
        resumed.prepare_report().durable_points_reused(),
        u64::try_from(BLOCK_POINTS + 1).unwrap()
    );
    assert_eq!(resumed.prepare_report().source_points_read(), 0);
}

#[test]
fn concurrent_prepares_have_one_exclusive_writer() {
    let source = open_source(clustered_ticks(BLOCK_POINTS * 2 + 1));
    let target = TemporaryTarget::new("concurrent-prepare-owner");
    let slow_limits = PrepareLimits::new(1, 24).unwrap();
    let first = prepare(source.clone(), target.path(), slow_limits);
    let first_handle = first.handle();
    let deadline = Instant::now() + Duration::from_secs(20);
    while fs::metadata(target.work_path()).map_or(true, |metadata| metadata.len() < 200) {
        assert!(
            Instant::now() < deadline,
            "first prepare did not initialize its work file"
        );
        thread::sleep(Duration::from_millis(1));
    }

    assert!(matches!(
        prepare(source.clone(), target.path(), PrepareLimits::default()).blocking_wait(),
        Err(IndexError::PreparationInProgress { .. })
    ));
    assert!(!target.path().exists());

    first_handle.cancel();
    assert!(matches!(
        first.blocking_wait(),
        Err(IndexError::Runtime(RuntimeError::Cancelled))
    ));
    assert!(target.work_path().exists());

    let resumed = prepare(source, target.path(), PrepareLimits::default())
        .blocking_wait()
        .unwrap();
    assert_eq!(
        resumed.prepare_report().disposition(),
        PrepareDisposition::Built
    );
}

#[test]
fn faulted_build_recovers_valid_frames_discards_bad_suffix_and_matches_clean_bytes() {
    let point_count = BLOCK_POINTS + 64;
    let ticks = clustered_ticks(point_count);
    let (source, faults) = open_controlled_source(ticks);
    let resumed_target = TemporaryTarget::new("fault-resume");
    faults.fail_at_ordinal(u64::try_from(BLOCK_POINTS + 3).unwrap());

    assert!(matches!(
        prepare(
            source.clone(),
            resumed_target.path(),
            PrepareLimits::default()
        )
        .blocking_wait(),
        Err(IndexError::Source(
            point_source::SourceError::CorruptSource { .. }
        ))
    ));
    assert!(!resumed_target.path().exists());
    assert!(resumed_target.work_path().exists());
    let valid_work = fs::read(resumed_target.work_path()).unwrap();
    assert!(valid_work.len() > 200);

    let mut changed_ticks = clustered_ticks(point_count);
    changed_ticks[0][0] += 1;
    let incompatible_source = open_source(changed_ticks);
    assert!(matches!(
        prepare(
            incompatible_source,
            resumed_target.path(),
            PrepareLimits::default()
        )
        .blocking_wait(),
        Err(IndexError::IncompatibleWork { .. })
    ));
    assert_eq!(fs::read(resumed_target.work_path()).unwrap(), valid_work);

    let corrupt_target = resumed_target.copied_target("corrupt-work.pidx");
    let corrupt_work = resumed_target.copied_target("corrupt-work.pidx.work");
    let mut corrupt_bytes = valid_work.clone();
    corrupt_bytes[0] ^= 0x01;
    fs::write(&corrupt_work, &corrupt_bytes).unwrap();
    assert!(matches!(
        prepare(source.clone(), &corrupt_target, PrepareLimits::default()).blocking_wait(),
        Err(IndexError::CorruptWork { .. })
    ));
    assert_eq!(fs::read(&corrupt_work).unwrap(), corrupt_bytes);

    let truncated_target = resumed_target.copied_target("truncated-work.pidx");
    let truncated_work = resumed_target.copied_target("truncated-work.pidx.work");
    fs::write(&truncated_work, &valid_work[..100]).unwrap();
    assert!(matches!(
        prepare(source.clone(), &truncated_target, PrepareLimits::default()).blocking_wait(),
        Err(IndexError::CorruptWork { .. })
    ));
    assert_eq!(fs::metadata(&truncated_work).unwrap().len(), 100);

    let mut work = OpenOptions::new()
        .append(true)
        .open(resumed_target.work_path())
        .unwrap();
    work.write_all(b"incomplete-frame-suffix").unwrap();
    work.sync_data().unwrap();
    drop(work);
    faults.clear_read_fault();

    let resumed = prepare(
        source.clone(),
        resumed_target.path(),
        PrepareLimits::default(),
    )
    .blocking_wait()
    .unwrap();
    assert_eq!(
        resumed.prepare_report().disposition(),
        PrepareDisposition::Resumed
    );
    assert_eq!(
        resumed.prepare_report().durable_points_reused(),
        u64::try_from(BLOCK_POINTS).unwrap()
    );
    assert_eq!(resumed.prepare_report().source_points_read(), 64);
    assert!(!resumed_target.work_path().exists());

    let clean_target = TemporaryTarget::new("fault-resume-clean");
    let clean_limits = PrepareLimits::new(251, 251 * 24).unwrap();
    let clean = prepare(source, clean_target.path(), clean_limits)
        .blocking_wait()
        .unwrap();
    assert_eq!(resumed.descriptor(), clean.descriptor());
    assert_eq!(resumed.hierarchy(), clean.hierarchy());
    assert_eq!(
        fs::read(resumed_target.path()).unwrap(),
        fs::read(clean_target.path()).unwrap()
    );
    for node in resumed.hierarchy().nodes() {
        assert_eq!(
            samples(&read_node(&resumed, node.id(), NodeReadBudget::default())),
            samples(&read_node(&clean, node.id(), NodeReadBudget::default()))
        );
    }
}

#[test]
fn cancelled_prepare_leaves_only_resumable_work_and_never_a_partial_target() {
    let point_count = BLOCK_POINTS * 32 + 17;
    let source = open_source(clustered_ticks(point_count));
    let target = TemporaryTarget::new("cancel-resume");
    let slow_limits = PrepareLimits::new(64, 64 * 24).unwrap();
    let job = prepare(source.clone(), target.path(), slow_limits);
    let handle = job.handle();
    let deadline = Instant::now() + Duration::from_secs(20);
    while handle.progress().completed_units() < u64::try_from(BLOCK_POINTS).unwrap() {
        assert!(
            Instant::now() < deadline,
            "index build did not reach its first durable frame"
        );
        thread::sleep(Duration::from_millis(1));
    }
    handle.cancel();
    assert!(matches!(
        job.blocking_wait(),
        Err(IndexError::Runtime(RuntimeError::Cancelled))
    ));
    assert!(!target.path().exists());
    assert!(target.work_path().exists());

    let resumed = prepare(source.clone(), target.path(), PrepareLimits::default())
        .blocking_wait()
        .unwrap();
    assert_eq!(
        resumed.prepare_report().disposition(),
        PrepareDisposition::Resumed
    );
    assert!(resumed.prepare_report().durable_points_reused() >= BLOCK_POINTS as u64);
    assert!(resumed.prepare_report().durable_points_reused() < u64::try_from(point_count).unwrap());
    assert!(!target.work_path().exists());

    let clean_target = TemporaryTarget::new("cancel-resume-clean");
    let clean = prepare(source, clean_target.path(), PrepareLimits::default())
        .blocking_wait()
        .unwrap();
    assert_eq!(resumed.descriptor(), clean.descriptor());
    assert_eq!(resumed.hierarchy(), clean.hierarchy());
    assert_eq!(
        fs::read(target.path()).unwrap(),
        fs::read(clean_target.path()).unwrap()
    );
}

fn encode_sample_pair(samples: &[(u64, [i64; 3])]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * 32);
    for &(ordinal, ticks) in samples {
        bytes.extend_from_slice(&ordinal.to_le_bytes());
        for axis in ticks {
            bytes.extend_from_slice(&axis.to_le_bytes());
        }
    }
    bytes
}

fn assert_resource_error(result: &Result<point_index::PreparedIndex, IndexError>) {
    assert!(matches!(result, Err(IndexError::ResourceLimit { .. })));
}
