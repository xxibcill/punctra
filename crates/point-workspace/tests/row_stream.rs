//! Exact Snapshot Point-row behavior through the public Workspace seam.

#[path = "support/evidence.rs"]
#[allow(dead_code)]
mod evidence;
mod support;

use blake3::Hasher;
use foundation_runtime::BatchStream;
use las::{
    Builder, Point, Transform, Vector, Writer,
    point::{Classification, Format},
};
use point_contracts::{ContentHash, PointId, WorldBounds};
use point_index::{CandidateLimits, PrepareLimits, prepare};
use point_source::ReadBudget;
use point_workspace::{
    CommitLimits, CommitOutcome, CommitRequest, OpenLimits, OperationId, PointQuery,
    PointRowLimits, Snapshot, SnapshotPointBatch, SnapshotPointSummary, WorkspaceError,
    WorkspaceSchema, create,
};

use evidence::{create_fixture_workspace, selection_limits};
use support::{TemporaryFixture, fixture_rows, inclusive, transform};

const MIB: u64 = 1024 * 1024;
const ROW_BYTES: u64 = 33;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Row {
    ordinal: u64,
    ticks: [i64; 3],
    classification: u8,
}

fn row_limits(source_batch_points: u64, output_batch_points: u64) -> PointRowLimits {
    let source = ReadBudget::new(source_batch_points, 8 * MIB)
        .expect("test Source batch limits are nonzero")
        .with_max_points(10_000_000)
        .with_max_adapter_working_bytes(16 * MIB);
    PointRowLimits::new(
        CandidateLimits::default(),
        source,
        1_000_000,
        128 * MIB,
        10_000_000,
        output_batch_points,
        output_batch_points.saturating_mul(ROW_BYTES),
        256 * MIB,
    )
}

fn collect(
    snapshot: &Snapshot,
    query: PointQuery,
    limits: PointRowLimits,
) -> (Vec<Row>, SnapshotPointSummary) {
    let mut batches = snapshot
        .point_rows(query, limits)
        .expect("Snapshot Point stream starts");
    assert_eq!(batches.source_metadata().position_transform(), transform());
    assert_batch_stream(&batches);
    let mut rows = Vec::new();
    while let Some(batch) = batches.next().expect("Snapshot Point batch succeeds") {
        assert!(batches.summary().is_none());
        assert_batch_contract(&batch, limits);
        rows.extend(
            batch
                .ordinals()
                .iter()
                .copied()
                .zip(batch.positions().ticks().iter().copied())
                .zip(batch.effective_classifications().iter().copied())
                .map(|((ordinal, ticks), classification)| Row {
                    ordinal,
                    ticks,
                    classification,
                }),
        );
    }
    let summary = batches
        .summary()
        .expect("terminal None publishes exact facts")
        .clone();
    assert_eq!(summary.exact_count(), u64::try_from(rows.len()).unwrap());
    assert!(batches.next().expect("success is fused").is_none());
    assert_eq!(batches.summary(), Some(&summary));
    (rows, summary)
}

fn assert_batch_stream(
    _: &impl BatchStream<
        Batch = SnapshotPointBatch,
        Summary = SnapshotPointSummary,
        Error = WorkspaceError,
    >,
) {
}

fn assert_batch_contract(batch: &SnapshotPointBatch, limits: PointRowLimits) {
    assert!(!batch.is_empty());
    assert_eq!(batch.len(), batch.positions().len());
    assert_eq!(batch.len(), batch.effective_classifications().len());
    assert!(u64::try_from(batch.len()).unwrap() <= limits.max_batch_points());
    let payload_bytes = u64::try_from(batch.len())
        .unwrap_or(u64::MAX)
        .saturating_mul(ROW_BYTES);
    assert!(payload_bytes <= limits.max_batch_payload_bytes());
    assert!(batch.ordinals().windows(2).all(|pair| pair[0] < pair[1]));
    for row in 0..batch.len() {
        assert_eq!(
            batch.point_id(row),
            Some(PointId::new(batch.source(), batch.ordinals()[row]))
        );
    }
    assert_eq!(batch.point_id(batch.len()), None);
}

fn operation(byte: u8) -> OperationId {
    OperationId::from_bytes([byte; 16]).expect("nonzero deterministic Operation Identity")
}

#[test]
fn exact_rows_and_hashes_are_partition_independent_and_match_point_set_membership() {
    let (_temporary, _index, workspace, ticks, classifications) =
        create_fixture_workspace("row-partitions", 5_003);
    let snapshot = workspace.head();
    let bounds = WorldBounds::new(
        transform().world_f64([-12, -8, -8]),
        transform().world_f64([12, 8, 8]),
    )
    .expect("fixture bounds are ordered");
    let query = PointQuery::within(bounds).classification_is(3);
    let expected = ticks
        .iter()
        .copied()
        .zip(classifications.iter().copied())
        .enumerate()
        .filter_map(|(ordinal, (ticks, classification))| {
            (inclusive(bounds, transform().world_f64(ticks)) && classification == 3).then_some(
                Row {
                    ordinal: u64::try_from(ordinal).unwrap(),
                    ticks,
                    classification,
                },
            )
        })
        .collect::<Vec<_>>();

    let (one_by_one, fine_summary) = collect(&snapshot, query, row_limits(1, 3));
    let (chunked, coarse_summary) = collect(&snapshot, query, row_limits(257, 17));

    assert_eq!(one_by_one, expected);
    assert_eq!(chunked, expected);
    assert_eq!(fine_summary, coarse_summary);
    assert_eq!(fine_summary.provenance(), snapshot.provenance());
    assert_eq!(fine_summary.query(), query);
    assert!(fine_summary.candidate_point_count() >= fine_summary.exact_count());
    assert_eq!(
        fine_summary.content_hash(),
        row_content_hash(&snapshot, &expected)
    );

    let point_set = snapshot
        .select(query, selection_limits(31, 8 * MIB))
        .blocking_wait()
        .expect("comparison Point Set materializes");
    assert_eq!(
        fine_summary.point_id_hash(),
        point_set.metadata().point_id_hash(),
        "row and Point Set membership use one canonical identity digest"
    );
}

#[test]
fn root_edited_historical_and_revert_snapshots_expose_exact_effective_rows() {
    let (_temporary, _index, workspace, ticks, _classifications) =
        create_fixture_workspace("row-revisions", 513);
    let root = workspace.head();
    let target_ordinals = [0_u64, 1, 2, 97, 255, 512];
    let points = root
        .select_point_ids(
            target_ordinals.map(|ordinal| PointId::new(workspace.source(), ordinal)),
            selection_limits(3, 8 * MIB),
        )
        .blocking_wait()
        .expect("Edit target materializes");
    let set_revision = match workspace
        .commit(
            CommitRequest::set_classification(operation(41), points, 42),
            CommitLimits::default(),
        )
        .blocking_wait()
        .expect("classification commit is certain")
    {
        CommitOutcome::Committed(receipt) => receipt.revision(),
        outcome => panic!("classification commit did not commit: {outcome:?}"),
    };
    let edited = workspace.snapshot(set_revision).unwrap();
    let expected = target_ordinals
        .iter()
        .copied()
        .map(|ordinal| Row {
            ordinal,
            ticks: ticks[usize::try_from(ordinal).unwrap()],
            classification: 42,
        })
        .collect::<Vec<_>>();
    assert!(
        collect(
            &root,
            PointQuery::all().classification_is(42),
            row_limits(7, 2)
        )
        .0
        .is_empty()
    );
    assert_eq!(
        collect(
            &edited,
            PointQuery::all().classification_is(42),
            row_limits(1, 2)
        )
        .0,
        expected
    );

    let revert_revision = match workspace
        .commit(
            CommitRequest::revert_head(operation(42), set_revision),
            CommitLimits::default(),
        )
        .blocking_wait()
        .expect("Revert is certain")
    {
        CommitOutcome::Committed(receipt) => receipt.revision(),
        outcome => panic!("Revert did not commit: {outcome:?}"),
    };
    let reverted = workspace.snapshot(revert_revision).unwrap();
    assert!(
        collect(
            &reverted,
            PointQuery::all().classification_is(42),
            row_limits(13, 3)
        )
        .0
        .is_empty()
    );
    assert_eq!(
        collect(
            &edited,
            PointQuery::all().classification_is(42),
            row_limits(29, 4)
        )
        .0,
        expected,
        "the historical edited Snapshot remains immutable after Revert"
    );
}

#[test]
fn output_failure_and_cancellation_are_fused_without_a_summary() {
    let (_temporary, _index, workspace, _ticks, _classifications) =
        create_fixture_workspace("row-fused", 101);
    let snapshot = workspace.head();
    let defaults = row_limits(17, 2);
    let output_limited = PointRowLimits::new(
        defaults.candidate_limits(),
        defaults.source_read_budget(),
        defaults.max_overlay_segments(),
        defaults.max_overlay_bytes(),
        3,
        defaults.max_batch_points(),
        defaults.max_batch_payload_bytes(),
        defaults.max_working_bytes(),
    );
    let mut limited = snapshot
        .point_rows(PointQuery::all(), output_limited)
        .expect("limited stream starts");
    assert_eq!(limited.next().unwrap().unwrap().len(), 2);
    assert_eq!(limited.next().unwrap().unwrap().len(), 1);
    assert!(matches!(
        limited.next().unwrap_err(),
        WorkspaceError::ResourceLimit {
            limit: "emitted Snapshot Points",
            required: 4,
            allowed: 3
        }
    ));
    assert!(limited.summary().is_none());
    assert!(limited.next().expect("failure is fused").is_none());

    let mut cancelled = snapshot
        .point_rows(PointQuery::all(), row_limits(11, 5))
        .expect("cancellation stream starts");
    assert!(cancelled.next().unwrap().is_some());
    cancelled.handle().cancel();
    assert!(matches!(cancelled.next(), Err(WorkspaceError::Cancelled)));
    assert!(cancelled.summary().is_none());
    assert!(cancelled.next().expect("cancellation is fused").is_none());

    let plan_blocked = PointRowLimits::new(
        CandidateLimits::new(0, 0, 0, 0),
        defaults.source_read_budget(),
        defaults.max_overlay_segments(),
        defaults.max_overlay_bytes(),
        defaults.max_output_points(),
        defaults.max_batch_points(),
        defaults.max_batch_payload_bytes(),
        defaults.max_working_bytes(),
    );
    let mut cancelled_before_planning = snapshot
        .point_rows(PointQuery::all(), plan_blocked)
        .expect("stream exposes cancellation before candidate planning");
    cancelled_before_planning.handle().cancel();
    assert!(matches!(
        cancelled_before_planning.next(),
        Err(WorkspaceError::Cancelled)
    ));
    assert!(cancelled_before_planning.summary().is_none());
}

#[test]
fn zero_output_capacity_accepts_a_complete_no_match_query_but_fails_on_a_match() {
    let (_temporary, _index, workspace, _ticks, _classifications) =
        create_fixture_workspace("row-zero-output", 257);
    let snapshot = workspace.head();
    let defaults = row_limits(7, 1);
    let zero = PointRowLimits::new(
        defaults.candidate_limits(),
        defaults.source_read_budget(),
        defaults.max_overlay_segments(),
        defaults.max_overlay_bytes(),
        0,
        0,
        0,
        defaults.max_working_bytes(),
    );

    let (rows, summary) = collect(
        &snapshot,
        PointQuery::all().classification_is(u8::MAX),
        zero,
    );
    assert!(rows.is_empty());
    assert_eq!(summary.exact_count(), 0);
    assert_eq!(summary.candidate_point_count(), 257);

    let mut matching = snapshot
        .point_rows(PointQuery::all(), zero)
        .expect("zero-capacity matching stream starts before evaluating rows");
    assert!(matches!(
        matching.next().unwrap_err(),
        WorkspaceError::ResourceLimit {
            limit: "emitted Snapshot Points",
            required: 1,
            allowed: 0
        }
    ));
    assert!(matching.summary().is_none());
    assert!(
        matching
            .next()
            .expect("zero-capacity error is fused")
            .is_none()
    );
}

#[test]
fn overlay_limits_accumulate_across_source_batches() {
    let (_temporary, _index, workspace, _ticks, _classifications) =
        create_fixture_workspace("row-overlay-limits", 257);
    let root = workspace.head();
    let points = root
        .select_point_ids(
            [0_u64, 256].map(|ordinal| PointId::new(workspace.source(), ordinal)),
            selection_limits(1, 8 * MIB),
        )
        .blocking_wait()
        .expect("overlay target materializes");
    assert!(matches!(
        workspace
            .commit(
                CommitRequest::set_classification(operation(61), points, 42),
                CommitLimits::default(),
            )
            .blocking_wait()
            .expect("overlay fixture commit is certain"),
        CommitOutcome::Committed(_)
    ));
    let defaults = row_limits(1, 1);
    let one_segment = PointRowLimits::new(
        defaults.candidate_limits(),
        defaults.source_read_budget(),
        1,
        defaults.max_overlay_bytes(),
        defaults.max_output_points(),
        defaults.max_batch_points(),
        defaults.max_batch_payload_bytes(),
        defaults.max_working_bytes(),
    );
    let mut rows = workspace
        .head()
        .point_rows(PointQuery::all(), one_segment)
        .expect("overlay-limited row stream starts");
    let error = loop {
        match rows.next() {
            Ok(Some(_)) => {}
            Err(error) => break error,
            Ok(None) => panic!("cumulative overlay ceiling unexpectedly completed"),
        }
    };
    assert!(matches!(
        error,
        WorkspaceError::ResourceLimit {
            limit: "overlay blocks",
            required: 2,
            allowed: 1
        }
    ));
    assert!(rows.summary().is_none());
    assert!(rows.next().expect("overlay failure is fused").is_none());
}

#[test]
fn generated_las_and_laz_emit_identical_exact_row_values() {
    let (ticks, classifications) = fixture_rows(257);
    let expected = ticks
        .iter()
        .copied()
        .zip(classifications.iter().copied())
        .enumerate()
        .filter_map(|(ordinal, (ticks, classification))| {
            (classification == 3).then_some(Row {
                ordinal: u64::try_from(ordinal).unwrap(),
                ticks,
                classification,
            })
        })
        .collect::<Vec<_>>();

    for extension in ["las", "laz"] {
        let temporary = TemporaryFixture::new(&format!("row-{extension}"));
        let source_path = temporary.path().join(format!("fixture.{extension}"));
        write_las_family_fixture(&source_path, &ticks, &classifications);
        let source = source_las::open(&source_path)
            .blocking_wait()
            .expect("generated LAS-family Source opens");
        let classification = source
            .metadata()
            .attributes()
            .definitions()
            .iter()
            .find(|definition| definition.name() == "classification")
            .expect("LAS-family schema declares classification")
            .id();
        let index = prepare(source, temporary.index_path(), PrepareLimits::default())
            .blocking_wait()
            .expect("LAS-family index prepares");
        let workspace = create(
            temporary.workspace_path(),
            index,
            WorkspaceSchema::new(classification),
            OpenLimits::default(),
        )
        .blocking_wait()
        .expect("LAS-family Workspace creates");
        let (actual, summary) = collect(
            &workspace.head(),
            PointQuery::all().classification_is(3),
            row_limits(11, 5),
        );
        assert_eq!(actual, expected, "{extension} changed exact row meaning");
        assert_eq!(
            summary.exact_count(),
            u64::try_from(expected.len()).unwrap()
        );
    }
}

fn write_las_family_fixture(path: &std::path::Path, ticks: &[[i64; 3]], classes: &[u8]) {
    let mut builder = Builder::from((1, 4));
    builder.point_format = Format::new(0).expect("PDRF 0 matches the fixture fields");
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

fn row_content_hash(snapshot: &Snapshot, rows: &[Row]) -> ContentHash {
    let mut hasher = Hasher::new();
    hasher.update(b"punctra-snapshot-point-rows-v1");
    hasher.update(snapshot.provenance().workspace().as_bytes());
    hasher.update(snapshot.provenance().source().as_bytes());
    hasher.update(snapshot.provenance().revision().as_bytes());
    for row in rows {
        hasher.update(&row.ordinal.to_le_bytes());
        for coordinate in row.ticks {
            hasher.update(&coordinate.to_le_bytes());
        }
        hasher.update(&[row.classification]);
    }
    ContentHash::new(*hasher.finalize().as_bytes())
}
