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
    IndexError, IndexLimit, IndexRecipe, InspectionAttributeIds, NodeReadBudget,
    PrepareDisposition, PrepareLimits, prepare, prepare_with_recipe,
};
use point_source::ReadLimit;

use support::{
    BLOCK_POINTS, CLASSIFICATION_ID, INTENSITY_ID, RGB_IDS, TemporaryTarget, clustered_ticks,
    open_attributed_source, open_controlled_attributed_source, open_controlled_source, open_source,
    read_node, samples,
};

const V1_ARTIFACT: &[u8] = include_bytes!("fixtures/v1/one-point.pidx");
const V1_WORK: &[u8] = include_bytes!("fixtures/v1/one-point.pidx.work");
const V2_ARTIFACT: &[u8] = include_bytes!("fixtures/v2/one-point.pidx");
const V2_WORK: &[u8] = include_bytes!("fixtures/v2/one-point.pidx.work");

fn inspection_recipe() -> IndexRecipe {
    IndexRecipe::InspectionV1(
        InspectionAttributeIds::new(INTENSITY_ID, CLASSIFICATION_ID, RGB_IDS).unwrap(),
    )
}

#[test]
fn golden_fixture_lengths_and_blake3_are_pinned() {
    for (name, bytes, expected_length, expected_hash) in [
        (
            "v1 artifact",
            V1_ARTIFACT,
            408,
            "d9e0769b00bfe5f35845f94fab5b67107b85d94feb54175e7b56ee3e6bf48954",
        ),
        (
            "v1 work",
            V1_WORK,
            344,
            "e81496a3e1f42526599394d12fd9234a13d1da9c88233c0b7fee83536c010810",
        ),
        (
            "v2 artifact",
            V2_ARTIFACT,
            440,
            "df525242a625203610b3b03988bd1af5c02532d7e713c4e722cb8c383f202648",
        ),
        (
            "v2 work",
            V2_WORK,
            386,
            "dd8e8177a7d89d091e4dcab7c3b90b2534ecc977558e17a350c33dc9ac1a3775",
        ),
    ] {
        assert_eq!(bytes.len(), expected_length, "{name} length");
        assert_eq!(
            blake3::hash(bytes).to_hex().as_str(),
            expected_hash,
            "{name} BLAKE3"
        );
    }
}

#[test]
fn prepare_report_observes_exact_temporary_disk_peak_for_v1_v2_and_warm_open() {
    let v1_target = TemporaryTarget::new("v1-temporary-peak");
    let v1_source = open_source(clustered_ticks(1));
    let v1 = prepare(
        v1_source.clone(),
        v1_target.path(),
        PrepareLimits::default(),
    )
    .blocking_wait()
    .unwrap();
    assert_eq!(v1.prepare_report().peak_temporary_disk_bytes(), 752);
    let v1_opened = prepare(v1_source, v1_target.path(), PrepareLimits::default())
        .blocking_wait()
        .unwrap();
    assert_eq!(v1_opened.prepare_report().peak_temporary_disk_bytes(), 0);

    let v2_target = TemporaryTarget::new("v2-temporary-peak");
    let v2_source = open_attributed_source(clustered_ticks(1), true);
    let v2 = prepare_with_recipe(
        v2_source.clone(),
        v2_target.path(),
        inspection_recipe(),
        PrepareLimits::default(),
    )
    .blocking_wait()
    .unwrap();
    assert_eq!(v2.prepare_report().peak_temporary_disk_bytes(), 826);
    let v2_opened = prepare_with_recipe(
        v2_source,
        v2_target.path(),
        inspection_recipe(),
        PrepareLimits::default(),
    )
    .blocking_wait()
    .unwrap();
    assert_eq!(v2_opened.prepare_report().peak_temporary_disk_bytes(), 0);
}

#[test]
fn v1_and_v2_targets_are_preserved_when_requested_through_the_other_recipe() {
    let attributed = open_attributed_source(clustered_ticks(1), true);

    let v1_target = TemporaryTarget::new("v1-requested-as-v2");
    prepare(
        attributed.clone(),
        v1_target.path(),
        PrepareLimits::default(),
    )
    .blocking_wait()
    .unwrap();
    let v1_bytes = fs::read(v1_target.path()).unwrap();
    assert!(matches!(
        prepare_with_recipe(
            attributed.clone(),
            v1_target.path(),
            inspection_recipe(),
            PrepareLimits::default(),
        )
        .blocking_wait(),
        Err(IndexError::IncompatibleArtifact { .. })
    ));
    assert_eq!(fs::read(v1_target.path()).unwrap(), v1_bytes);

    let v2_target = TemporaryTarget::new("v2-requested-as-v1");
    prepare_with_recipe(
        attributed.clone(),
        v2_target.path(),
        inspection_recipe(),
        PrepareLimits::default(),
    )
    .blocking_wait()
    .unwrap();
    let v2_bytes = fs::read(v2_target.path()).unwrap();
    assert!(matches!(
        prepare(attributed, v2_target.path(), PrepareLimits::default()).blocking_wait(),
        Err(IndexError::IncompatibleArtifact { .. })
    ));
    assert_eq!(fs::read(v2_target.path()).unwrap(), v2_bytes);
}

#[test]
fn inspection_v2_resumes_durable_frames_and_rejects_checksum_valid_header_corruption() {
    let point_count = BLOCK_POINTS + 64;
    let (source, faults) = open_controlled_attributed_source(clustered_ticks(point_count), true);
    let target = TemporaryTarget::new("v2-resume");
    faults.fail_at_ordinal(u64::try_from(BLOCK_POINTS + 3).unwrap());
    assert!(matches!(
        prepare_with_recipe(
            source.clone(),
            target.path(),
            inspection_recipe(),
            PrepareLimits::default(),
        )
        .blocking_wait(),
        Err(IndexError::Source(
            point_source::SourceError::CorruptSource { .. }
        ))
    ));
    assert!(fs::metadata(target.work_path()).unwrap().len() > 232);
    faults.clear_read_fault();
    let resumed = prepare_with_recipe(
        source.clone(),
        target.path(),
        inspection_recipe(),
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
    assert_eq!(resumed.descriptor().disk_version(), 2);

    let mut corrupt = fs::read(target.path()).unwrap();
    corrupt[212] |= 0x80;
    let checksum_offset = corrupt.len() - 32;
    let checksum = blake3::hash(&corrupt[..checksum_offset]);
    corrupt[checksum_offset..].copy_from_slice(checksum.as_bytes());
    let corrupt_target = target.copied_target("v2-corrupt-extension.pidx");
    fs::write(&corrupt_target, &corrupt).unwrap();
    assert!(matches!(
        prepare_with_recipe(
            source,
            &corrupt_target,
            inspection_recipe(),
            PrepareLimits::default(),
        )
        .blocking_wait(),
        Err(IndexError::CorruptArtifact { .. })
    ));
    assert_eq!(fs::read(corrupt_target).unwrap(), corrupt);
}

#[test]
fn cold_build_preserves_preexisting_adjacent_files() {
    let target = TemporaryTarget::new("preexisting-adjacent-files");
    let temporary = target.copied_target("fixture.pidx.tmp");
    let samples = target.copied_target("fixture.pidx.samples");
    let temporary_sentinel = b"caller-owned temporary file";
    let samples_sentinel = b"caller-owned sample file";
    fs::write(&temporary, temporary_sentinel).unwrap();
    fs::write(&samples, samples_sentinel).unwrap();

    prepare(
        open_source(clustered_ticks(1)),
        target.path(),
        PrepareLimits::default(),
    )
    .blocking_wait()
    .unwrap();

    assert_eq!(fs::read(temporary).unwrap(), temporary_sentinel);
    assert_eq!(fs::read(samples).unwrap(), samples_sentinel);
}

#[test]
fn cold_build_preserves_an_unowned_empty_work_path() {
    let target = TemporaryTarget::new("unowned-empty-work");
    fs::write(target.work_path(), []).unwrap();

    assert!(matches!(
        prepare(
            open_source(clustered_ticks(1)),
            target.path(),
            PrepareLimits::default(),
        )
        .blocking_wait(),
        Err(IndexError::CorruptWork { .. })
    ));
    assert!(!target.path().exists());
    assert_eq!(fs::metadata(target.work_path()).unwrap().len(), 0);
}

#[test]
fn disk_v1_golden_fixtures_open_and_resume_without_reencoding_the_input() {
    let source = open_source(clustered_ticks(1));

    let complete_target = TemporaryTarget::new("v1-golden-complete");
    fs::write(complete_target.path(), V1_ARTIFACT).unwrap();
    let opened = prepare(
        source.clone(),
        complete_target.path(),
        PrepareLimits::default(),
    )
    .blocking_wait()
    .unwrap();
    assert_eq!(
        opened.prepare_report().disposition(),
        PrepareDisposition::Opened
    );
    assert_eq!(opened.prepare_report().source_points_read(), 0);
    let root = opened.hierarchy().root().unwrap();
    let read = read_node(&opened, root.id(), NodeReadBudget::default());
    assert_eq!(samples(&read).len(), 1);
    assert_eq!(fs::read(complete_target.path()).unwrap(), V1_ARTIFACT);

    let work_target = TemporaryTarget::new("v1-golden-work");
    fs::write(work_target.work_path(), V1_WORK).unwrap();
    let resumed = prepare(source, work_target.path(), PrepareLimits::default())
        .blocking_wait()
        .unwrap();
    assert_eq!(
        resumed.prepare_report().disposition(),
        PrepareDisposition::Resumed
    );
    assert_eq!(resumed.prepare_report().durable_points_reused(), 1);
    assert_eq!(resumed.prepare_report().source_points_read(), 0);
    assert_eq!(fs::read(work_target.path()).unwrap(), V1_ARTIFACT);
}

#[test]
fn disk_v2_golden_fixtures_open_and_resume_without_reencoding_the_input() {
    let source = open_attributed_source(clustered_ticks(1), true);

    let complete_target = TemporaryTarget::new("v2-golden-complete");
    fs::write(complete_target.path(), V2_ARTIFACT).unwrap();
    let opened = prepare_with_recipe(
        source.clone(),
        complete_target.path(),
        inspection_recipe(),
        PrepareLimits::default(),
    )
    .blocking_wait()
    .unwrap();
    assert_eq!(
        opened.prepare_report().disposition(),
        PrepareDisposition::Opened
    );
    assert_eq!(opened.prepare_report().peak_temporary_disk_bytes(), 0);
    assert_eq!(fs::read(complete_target.path()).unwrap(), V2_ARTIFACT);
    let root = opened.hierarchy().root().unwrap();
    let read = read_node(&opened, root.id(), NodeReadBudget::default());
    assert_eq!(
        read.batches[0].display_attributes().unwrap()[0].intensity(),
        0
    );

    let work_target = TemporaryTarget::new("v2-golden-work");
    fs::write(work_target.work_path(), V2_WORK).unwrap();
    let resumed = prepare_with_recipe(
        source,
        work_target.path(),
        inspection_recipe(),
        PrepareLimits::default(),
    )
    .blocking_wait()
    .unwrap();
    assert_eq!(
        resumed.prepare_report().disposition(),
        PrepareDisposition::Resumed
    );
    assert_eq!(resumed.prepare_report().durable_points_reused(), 1);
    assert_eq!(resumed.prepare_report().source_points_read(), 0);
    assert_eq!(fs::read(work_target.path()).unwrap(), V2_ARTIFACT);
}

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
fn warm_open_rejects_checksum_valid_samples_outside_the_bottom_k_recipe() {
    let ticks = clustered_ticks(BLOCK_POINTS + 1);
    let source = open_source(ticks.clone());
    let target = TemporaryTarget::new("non-recipe-samples");
    let index = prepare(source.clone(), target.path(), PrepareLimits::default())
        .blocking_wait()
        .unwrap();
    let root = index.hierarchy().root().unwrap();
    let mut root_samples = samples(&read_node(&index, root.id(), NodeReadBudget::default()));
    let replacement = (0..u64::try_from(ticks.len()).unwrap())
        .find(|ordinal| {
            root_samples
                .binary_search_by_key(ordinal, |sample| sample.0)
                .is_err()
        })
        .expect("the bounded root sample excludes one Source ordinal");
    root_samples[0] = (replacement, ticks[usize::try_from(replacement).unwrap()]);
    root_samples.sort_unstable_by_key(|sample| sample.0);

    let mut artifact = fs::read(target.path()).unwrap();
    let root_record = 208;
    let sample_offset = usize::try_from(read_u64(&artifact, root_record + 112)).unwrap();
    let sample_end = sample_offset + root_samples.len() * 32;
    let encoded_samples = encode_sample_pair(&root_samples);
    artifact[sample_offset..sample_end].copy_from_slice(&encoded_samples);
    let mut sample_hasher = blake3::Hasher::new();
    sample_hasher.update(b"punctra-index-samples-v1");
    sample_hasher.update(&encoded_samples);
    artifact[root_record + 136..root_record + 168]
        .copy_from_slice(sample_hasher.finalize().as_bytes());
    let artifact_checksum_offset = artifact.len() - 32;
    let artifact_checksum = blake3::hash(&artifact[..artifact_checksum_offset]);
    artifact[artifact_checksum_offset..].copy_from_slice(artifact_checksum.as_bytes());

    let forged = target.copied_target("forged-samples.pidx");
    fs::write(&forged, artifact).unwrap();
    assert!(matches!(
        prepare(source, forged, PrepareLimits::default()).blocking_wait(),
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
    let limit = PrepareLimits::default().with_max_build_working_bytes(450_000);
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
    assert!(resumed_target.work_path().exists());

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
    assert!(target.work_path().exists());

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

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn assert_resource_error(result: &Result<point_index::PreparedIndex, IndexError>) {
    assert!(matches!(result, Err(IndexError::ResourceLimit { .. })));
}
