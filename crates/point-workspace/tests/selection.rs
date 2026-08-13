//! Exact Query and spill behavior through the public Workspace seam.

#[path = "support/evidence.rs"]
mod evidence;
mod support;

use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    mem,
    time::{Duration, Instant},
};

use las::{
    Builder, Point, Transform, Vector, Writer,
    point::{Classification, Format},
};
use point_contracts::{PointId, WorldBounds};
use point_index::{PrepareLimits, prepare};
use point_workspace::{
    CommitLimits, CommitOutcome, CommitRequest, OpenLimits, OperationId, PointIdReadLimits,
    PointQuery, PointSetLimits, WorkspaceError, WorkspaceSchema, create, open,
};

use evidence::{
    collect_ids, create_fixture_workspace, forced_spill_limits, ordinals, selection_limits,
};
use support::{
    TemporaryFixture, classification_attribute, classification_for_ordinal, fixture_rows,
    inclusive, open_source, transform,
};

const MIB: u64 = 1024 * 1024;

#[test]
fn all_box_and_classification_queries_match_the_source_oracle_across_batches() {
    let (_temporary, _index, workspace, ticks, classifications) =
        create_fixture_workspace("exact-query", 5_003);
    let root = workspace.head();

    let all = root
        .select(PointQuery::all(), selection_limits(1, 8 * MIB))
        .blocking_wait()
        .expect("one-Point Source batches still produce a complete Query");
    assert_eq!(ordinals(&all, 13), (0_u64..5_003).collect::<Vec<_>>());

    let bounds = WorldBounds::new(
        transform().world_f64([-5, -3, -2]),
        transform().world_f64([5, 6, 4]),
    )
    .expect("literal evidence bounds are ordered");
    let expected = ticks
        .iter()
        .zip(&classifications)
        .enumerate()
        .filter_map(|(ordinal, (&ticks, &classification))| {
            (inclusive(bounds, transform().world_f64(ticks)) && classification == 3)
                .then_some(u64::try_from(ordinal).expect("fixture ordinal fits u64"))
        })
        .collect::<Vec<_>>();
    assert!(
        ticks.iter().any(|&ticks| {
            let world = transform().world_f64(ticks);
            inclusive(bounds, world)
                && (0..3).any(|axis| {
                    world[axis].to_bits() == bounds.min()[axis].to_bits()
                        || world[axis].to_bits() == bounds.max()[axis].to_bits()
                })
        }),
        "the oracle includes at least one exact inclusive boundary case"
    );

    let selected = root
        .select(
            PointQuery::within(bounds).classification_is(3),
            selection_limits(37, 8 * MIB),
        )
        .blocking_wait()
        .expect("bounded classification Query completes");
    assert_eq!(ordinals(&selected, 7), expected);
}

#[test]
fn seeded_cloud_queries_match_a_brute_force_oracle() {
    const POINT_COUNT: usize = 4_099;
    const QUERY_COUNT: usize = 32;

    let temporary = TemporaryFixture::new("seeded-query-oracle");
    let mut random = 0x9e37_79b9_7f4a_7c15_u64;
    let mut ticks = Vec::with_capacity(POINT_COUNT);
    let mut classifications = Vec::with_capacity(POINT_COUNT);
    for _ in 0..POINT_COUNT {
        let x = random_tick(&mut random, 20_001) - 10_000;
        let y = random_tick(&mut random, 16_001) - 8_000;
        let z = random_tick(&mut random, 4_001) - 2_000;
        ticks.push([x, y, z]);
        classifications.push(u8::try_from(next_random(&mut random) % 8).unwrap());
    }

    let source = open_source(ticks.clone(), classifications.clone());
    let index = prepare(source, temporary.index_path(), PrepareLimits::default())
        .blocking_wait()
        .expect("seeded fixture index prepares");
    let workspace = create(
        temporary.workspace_path(),
        index,
        WorkspaceSchema::new(classification_attribute()),
        OpenLimits::default(),
    )
    .blocking_wait()
    .expect("seeded fixture Workspace creates");
    let root = workspace.head();

    for query_index in 0..QUERY_COUNT {
        let first = [
            random_tick(&mut random, 20_001) - 10_000,
            random_tick(&mut random, 16_001) - 8_000,
            random_tick(&mut random, 4_001) - 2_000,
        ];
        let second = [
            random_tick(&mut random, 20_001) - 10_000,
            random_tick(&mut random, 16_001) - 8_000,
            random_tick(&mut random, 4_001) - 2_000,
        ];
        let min_ticks = std::array::from_fn(|axis| first[axis].min(second[axis]));
        let max_ticks = std::array::from_fn(|axis| first[axis].max(second[axis]));
        let bounds = WorldBounds::new(
            transform().world_f64(min_ticks),
            transform().world_f64(max_ticks),
        )
        .expect("ordered tick bounds produce ordered finite world bounds");
        let classification = u8::try_from(next_random(&mut random) % 8).unwrap();
        let expected = ticks
            .iter()
            .zip(&classifications)
            .enumerate()
            .filter_map(|(ordinal, (&ticks, &actual_classification))| {
                (inclusive(bounds, transform().world_f64(ticks))
                    && actual_classification == classification)
                    .then_some(u64::try_from(ordinal).expect("fixture ordinal fits u64"))
            })
            .collect::<Vec<_>>();

        let source_batch_points = 1 + u64::try_from(query_index % 63).unwrap();
        let selected = root
            .select(
                PointQuery::within(bounds).classification_is(classification),
                selection_limits(source_batch_points, 8 * MIB),
            )
            .blocking_wait()
            .expect("seeded exact Query completes");
        assert_eq!(
            ordinals(&selected, 1 + source_batch_points % 29),
            expected,
            "seeded Query {query_index} diverged from the brute-force oracle"
        );
    }
}

fn next_random(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state
}

fn random_tick(state: &mut u64, modulus: u64) -> i64 {
    i64::try_from(next_random(state) % modulus).expect("bounded random fixture tick fits i64")
}

#[test]
fn explicit_point_identity_selection_is_source_checked_sorted_and_deduplicated() {
    let (_temporary, _index, workspace, _ticks, _classifications) =
        create_fixture_workspace("explicit-identities", 257);
    let root = workspace.head();
    let source = workspace.source();
    let input = [91, 2, 90, 2, 256, 3, 4, 90].map(|ordinal| PointId::new(source, ordinal));

    let selected = root
        .select_point_ids(input, selection_limits(2, 8 * MIB))
        .blocking_wait()
        .expect("valid explicit Point Identities materialize");
    assert_eq!(ordinals(&selected, 2), vec![2, 3, 4, 90, 91, 256]);
    assert!(
        collect_ids(&selected, 3)
            .iter()
            .all(|point_id| point_id.source() == source)
    );

    let out_of_range = root
        .select_point_ids([PointId::new(source, 257)], selection_limits(1, 8 * MIB))
        .blocking_wait()
        .unwrap_err();
    assert!(matches!(
        out_of_range,
        WorkspaceError::InvalidArgument { .. }
    ));
}

#[test]
fn resident_and_forced_spill_point_sets_have_identical_repeatable_public_meaning() {
    let (_temporary, _index, workspace, _ticks, _classifications) =
        create_fixture_workspace("spill-equivalence", 9_017);
    let root = workspace.head();
    let query = PointQuery::all().classification_is(4);

    let resident = root
        .select(query, selection_limits(113, 8 * MIB))
        .blocking_wait()
        .expect("resident Point Set materializes");
    let spilled = root
        .select(query, forced_spill_limits(29))
        .blocking_wait()
        .expect("forced-spill Point Set materializes");

    assert_eq!(resident.metadata(), spilled.metadata());
    let expected = (0..9_017)
        .filter(|&ordinal| classification_for_ordinal(ordinal) == 4)
        .map(|ordinal| u64::try_from(ordinal).expect("fixture ordinal fits u64"))
        .collect::<Vec<_>>();
    assert_eq!(ordinals(&resident, 127), expected);
    assert_eq!(ordinals(&spilled, 1), expected);
    assert_eq!(ordinals(&spilled, 257), expected);
}

#[test]
fn forced_spill_point_sets_fail_closed_when_temporary_storage_is_missing_or_changed() {
    let (temporary, _index, workspace, _ticks, _classifications) =
        create_fixture_workspace("spill-integrity", 513);
    let root = workspace.head();

    let missing = root
        .select(PointQuery::all(), forced_spill_limits(17))
        .blocking_wait()
        .expect("missing-spill fixture materializes");
    let missing_path = only_spill_file(temporary.workspace_path().join("scratch"));
    fs::remove_file(missing_path).expect("fault fixture removes its own temporary spill");
    let Err(error) = missing.ids(PointIdReadLimits::default()) else {
        panic!("missing spill must fail before publishing an identity stream");
    };
    assert!(matches!(error, WorkspaceError::InvalidPointSet { .. }));
    drop(missing);

    let changed = root
        .select(PointQuery::all(), forced_spill_limits(17))
        .blocking_wait()
        .expect("changed-spill fixture materializes");
    let changed_path = only_spill_file(temporary.workspace_path().join("scratch"));
    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(changed_path)
        .expect("fault fixture opens its own spill");
    file.seek(SeekFrom::End(-1)).unwrap();
    let mut byte = [0_u8; 1];
    file.read_exact(&mut byte).unwrap();
    byte[0] ^= 0x80;
    file.seek(SeekFrom::End(-1)).unwrap();
    file.write_all(&byte).unwrap();
    file.sync_all().unwrap();
    drop(file);
    let error = match changed.ids(PointIdReadLimits::default()) {
        Err(error) => error,
        Ok(mut batches) => loop {
            match batches.next() {
                Err(error) => break error,
                Ok(Some(_)) => {}
                Ok(None) => panic!("changed spill must not validate to completion"),
            }
        },
    };
    assert!(matches!(error, WorkspaceError::InvalidPointSet { .. }));
}

fn spill_files(scratch: impl AsRef<std::path::Path>) -> Vec<std::path::PathBuf> {
    let mut paths = fs::read_dir(scratch)
        .expect("read fixture scratch")
        .map(|entry| entry.expect("read fixture scratch entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(std::ffi::OsStr::to_str)
                .and_then(|name| name.strip_prefix("point-set-"))
                .and_then(|name| name.strip_suffix(".pset"))
                .is_some()
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn only_spill_file(scratch: impl AsRef<std::path::Path>) -> std::path::PathBuf {
    let mut paths = spill_files(scratch);
    assert_eq!(paths.len(), 1, "fixture owns exactly one temporary spill");
    paths.pop().expect("one spill path exists")
}

#[test]
fn final_point_set_drop_preserves_a_spill_path_replacement() {
    let (temporary, _index, workspace, _ticks, _classifications) =
        create_fixture_workspace("spill-replacement", 513);
    let selected = workspace
        .head()
        .select(PointQuery::all(), forced_spill_limits(17))
        .blocking_wait()
        .expect("forced-spill Point Set materializes");
    let spill = only_spill_file(temporary.workspace_path().join("scratch"));
    let original = spill.with_extension("owned-pset");
    fs::rename(&spill, &original).expect("move the owned spill away from its captured name");
    fs::write(&spill, b"caller replacement").expect("install caller replacement");

    drop(selected);

    assert_eq!(fs::read(&spill).unwrap(), b"caller replacement");
    assert_eq!(fs::metadata(original).unwrap().len(), 0);
}

#[test]
fn selection_and_identity_read_limits_fail_instead_of_publishing_prefixes() {
    let (_temporary, _index, workspace, _ticks, _classifications) =
        create_fixture_workspace("selection-limits", 101);
    let root = workspace.head();
    let source = workspace.source();

    let defaults = selection_limits(17, 8 * MIB);
    let input_limited = PointSetLimits::new(
        defaults.candidate_limits(),
        defaults.source_read_budget(),
        2,
        defaults.max_output_points(),
        defaults.max_overlay_segments(),
        defaults.max_overlay_bytes(),
        defaults.max_working_bytes(),
        defaults.max_resident_bytes(),
        defaults.max_temporary_bytes(),
    );
    let error = root
        .select_point_ids(
            [
                PointId::new(source, 1),
                PointId::new(source, 2),
                PointId::new(source, 3),
            ],
            input_limited,
        )
        .blocking_wait()
        .unwrap_err();
    assert!(matches!(
        error,
        WorkspaceError::ResourceLimit {
            limit: "input Point Identities",
            required: 3,
            allowed: 2
        }
    ));

    let output_limited = PointSetLimits::new(
        defaults.candidate_limits(),
        defaults.source_read_budget(),
        defaults.max_input_point_ids(),
        100,
        defaults.max_overlay_segments(),
        defaults.max_overlay_bytes(),
        defaults.max_working_bytes(),
        defaults.max_resident_bytes(),
        defaults.max_temporary_bytes(),
    );
    let error = root
        .select(PointQuery::all(), output_limited)
        .blocking_wait()
        .unwrap_err();
    assert!(matches!(
        error,
        WorkspaceError::ResourceLimit {
            limit: "selected Points",
            required: 101,
            allowed: 100
        }
    ));

    let complete = root
        .select(PointQuery::all(), defaults)
        .blocking_wait()
        .expect("control selection succeeds");
    let point_id_bytes = u64::try_from(mem::size_of::<PointId>()).expect("PointId size fits u64");
    let Err(error) = complete.ids(PointIdReadLimits::new(
        100,
        10,
        10 * point_id_bytes,
        MIB,
        2 * MIB,
    )) else {
        panic!("read ceiling below exact count must fail before a stream is published");
    };
    assert!(matches!(error, WorkspaceError::ResourceLimit { .. }));
}

#[test]
fn cancelling_an_in_flight_forced_spill_selection_publishes_no_point_set_and_retains_spill() {
    let (temporary, _index, workspace, _ticks, _classifications) =
        create_fixture_workspace("selection-cancellation", 100_003);
    let job = workspace
        .head()
        .select(PointQuery::all(), forced_spill_limits(1));
    let handle = job.handle();
    let deadline = Instant::now() + Duration::from_secs(5);
    while handle.progress().completed_units() == 0 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(
        handle.progress().completed_units() > 0,
        "selection made progress before the cancellation deadline"
    );
    assert!(
        !spill_files(temporary.workspace_path().join("scratch")).is_empty(),
        "forced spill exists while the in-flight selection owns it"
    );
    handle.cancel();
    let error = job.blocking_wait().unwrap_err();
    assert!(matches!(error, WorkspaceError::Cancelled));
    let retained = spill_files(temporary.workspace_path().join("scratch"));
    assert!(
        !retained.is_empty(),
        "cancelled selection retains its unpublished spill instead of deleting by pathname"
    );
    assert!(
        retained
            .iter()
            .all(|path| fs::metadata(path).unwrap().len() == 0),
        "cancelled unpublished spills release their payload through owned handles"
    );
}

#[test]
fn overlay_block_and_payload_limits_are_cumulative_across_source_batches() {
    let (_temporary, _index, workspace, _ticks, _classifications) =
        create_fixture_workspace("cumulative-overlay-limits", 257);
    let root = workspace.head();
    let target = root
        .select_point_ids(
            [0, 256].map(|ordinal| PointId::new(workspace.source(), ordinal)),
            selection_limits(1, 8 * MIB),
        )
        .blocking_wait()
        .expect("overlay target materializes");
    let operation = OperationId::from_bytes([33; 16]).unwrap();
    let outcome = workspace
        .commit(
            CommitRequest::set_classification(operation, target, 42),
            CommitLimits::default(),
        )
        .blocking_wait()
        .expect("overlay fixture commit has a certain outcome");
    assert!(matches!(outcome, CommitOutcome::Committed(_)));
    let head = workspace.head();
    let defaults = selection_limits(1, 8 * MIB);

    let one_block = PointSetLimits::new(
        defaults.candidate_limits(),
        defaults.source_read_budget(),
        defaults.max_input_point_ids(),
        defaults.max_output_points(),
        1,
        defaults.max_overlay_bytes(),
        defaults.max_working_bytes(),
        defaults.max_resident_bytes(),
        defaults.max_temporary_bytes(),
    );
    let block_error = head
        .select(PointQuery::all(), one_block)
        .blocking_wait()
        .unwrap_err();
    assert!(matches!(
        block_error,
        WorkspaceError::ResourceLimit {
            limit: "overlay blocks",
            required: 2,
            allowed: 1
        }
    ));

    let one_payload = PointSetLimits::new(
        defaults.candidate_limits(),
        defaults.source_read_budget(),
        defaults.max_input_point_ids(),
        defaults.max_output_points(),
        defaults.max_overlay_segments(),
        20,
        defaults.max_working_bytes(),
        defaults.max_resident_bytes(),
        defaults.max_temporary_bytes(),
    );
    let payload_error = head
        .select(PointQuery::all(), one_payload)
        .blocking_wait()
        .unwrap_err();
    assert!(matches!(
        payload_error,
        WorkspaceError::ResourceLimit {
            limit: "overlay payload bytes",
            required: 40,
            allowed: 20
        }
    ));
}

#[test]
fn generated_las_and_laz_sources_select_commit_and_reopen_without_source_mutation() {
    for (case, extension) in ["las", "laz"].into_iter().enumerate() {
        let temporary = TemporaryFixture::new(extension);
        let source_path = temporary.path().join(format!("fixture.{extension}"));
        let (ticks, classifications) = fixture_rows(257);
        write_las_family_fixture(&source_path, &ticks, &classifications);
        let source_bytes = fs::read(&source_path).expect("read immutable Source fixture");

        let source = source_las::open(&source_path)
            .blocking_wait()
            .expect("generated LAS-family Source opens");
        let classification_attribute = source
            .metadata()
            .attributes()
            .definitions()
            .iter()
            .find(|definition| definition.name() == "classification")
            .expect("LAS-family schema declares classification")
            .id();
        let index = prepare(source, temporary.index_path(), PrepareLimits::default())
            .blocking_wait()
            .expect("LAS-family Source index prepares");
        let workspace = create(
            temporary.workspace_path(),
            index,
            WorkspaceSchema::new(classification_attribute),
            OpenLimits::default(),
        )
        .blocking_wait()
        .expect("LAS-family Workspace creates");
        let root = workspace.head();
        let selected = root
            .select(
                PointQuery::all().classification_is(3),
                forced_spill_limits(11),
            )
            .blocking_wait()
            .expect("LAS-family exact classification Query completes");
        let expected = classifications
            .iter()
            .enumerate()
            .filter_map(|(ordinal, &classification)| {
                (classification == 3)
                    .then_some(u64::try_from(ordinal).expect("fixture ordinal fits u64"))
            })
            .collect::<Vec<_>>();
        assert_eq!(ordinals(&selected, 5), expected);

        let operation_byte = u8::try_from(case).expect("case index fits u8") + 20;
        let operation = OperationId::from_bytes([operation_byte; 16])
            .expect("deterministic Operation Identity is nonzero");
        let revision = match workspace
            .commit(
                CommitRequest::set_classification(operation, selected.clone(), 42),
                CommitLimits::default(),
            )
            .blocking_wait()
            .expect("LAS-family classification commit has a certain result")
        {
            CommitOutcome::Committed(receipt) => receipt.revision(),
            other => panic!("LAS-family classification commit did not commit: {other:?}"),
        };
        assert_eq!(
            fs::read(&source_path).expect("reread immutable Source fixture"),
            source_bytes,
            "{extension} Source bytes changed after a Workspace Edit"
        );

        drop(selected);
        drop(root);
        drop(workspace);
        let reopened_source = source_las::open(&source_path)
            .blocking_wait()
            .expect("unchanged LAS-family Source reopens");
        let reopened_index = prepare(
            reopened_source,
            temporary.index_path(),
            PrepareLimits::default(),
        )
        .blocking_wait()
        .expect("complete LAS-family index reopens");
        let reopened = open(
            temporary.workspace_path(),
            reopened_index,
            OpenLimits::default(),
        )
        .blocking_wait()
        .expect("LAS-family Workspace reopens");
        assert_eq!(reopened.head().provenance().revision(), revision);
        let edited = reopened
            .head()
            .select(
                PointQuery::all().classification_is(42),
                selection_limits(13, 8 * MIB),
            )
            .blocking_wait()
            .expect("reopened LAS-family Snapshot applies the durable overlay");
        assert_eq!(ordinals(&edited, 7), expected);
        assert_eq!(
            fs::read(&source_path).expect("read Source bytes after reopen"),
            source_bytes,
            "{extension} Source bytes changed after reopen"
        );
    }
}

fn write_las_family_fixture(path: &std::path::Path, ticks: &[[i64; 3]], classes: &[u8]) {
    let mut builder = Builder::from((1, 4));
    builder.point_format = Format::new(0).expect("PDRF 0 matches the populated fixture fields");
    let position = transform();
    let scales = position.scale();
    let offsets = position.offset();
    builder.transforms = Vector {
        x: Transform {
            scale: scales[0],
            offset: offsets[0],
        },
        y: Transform {
            scale: scales[1],
            offset: offsets[1],
        },
        z: Transform {
            scale: scales[2],
            offset: offsets[2],
        },
    };
    let mut writer = Writer::from_path(path, builder.into_header().unwrap())
        .expect("create generated LAS-family fixture");
    for (&ticks, &classification) in ticks.iter().zip(classes) {
        let world = position.world_f64(ticks);
        writer
            .write_point(Point {
                x: world[0],
                y: world[1],
                z: world[2],
                classification: Classification::new(classification)
                    .expect("fixture classification is accepted by LAS"),
                ..Point::default()
            })
            .expect("write generated LAS-family Point");
    }
    writer.close().expect("seal generated LAS-family fixture");
}
