//! Public sparse-read coverage for LAZ organizations without random chunk seeking.

use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{
        Arc, Barrier,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

use las::point::Format;
use las::{Builder, Header, Point, Transform, Vector, Vlr};
use laz::{LasZipCompressor, LazVlr, LazVlrBuilder};
use point_source::{ReadBudget, ReadRequest, Source, SourceError, SourceSpan};

const POINT_COUNT: usize = 50_000;
const LAS_VLR_HEADER_BYTES: usize = 54;
const LASZIP_CHUNK_OFFSET_BYTES: usize = 8;
const GENEROUS_PAYLOAD_BYTES: u64 = 64 * 1_024;
const GENEROUS_ADAPTER_BYTES: u64 = 16 * 1_024 * 1_024;

#[derive(Clone, Copy, Debug)]
enum SequentialOrganization {
    PointWise,
    VariableChunks,
}

#[test]
fn pointwise_and_variable_chunk_laz_support_sparse_public_reads_and_cancellation() {
    let directory = FixtureDirectory::new();
    for organization in [
        SequentialOrganization::PointWise,
        SequentialOrganization::VariableChunks,
    ] {
        let path = directory.path().join(match organization {
            SequentialOrganization::PointWise => "pointwise.laz",
            SequentialOrganization::VariableChunks => "variable-chunks.laz",
        });
        write_fixture(&path, organization);
        assert_fixture_organization(&path, organization);

        let source = source_las::open(&path)
            .blocking_wait()
            .unwrap_or_else(|error| panic!("open {organization:?} fixture: {error}"));
        assert_sparse_read(&source, organization);
        assert_cancellable_replay(&source, organization);
    }
}

fn assert_sparse_read(source: &Source, organization: SequentialOrganization) {
    let requested = [span(7, 2), span(POINT_COUNT - 3, 3)];
    let budget = ReadBudget::new(2, GENEROUS_PAYLOAD_BYTES)
        .unwrap()
        .with_max_spans(2)
        .with_max_points(5)
        .with_max_adapter_working_bytes(GENEROUS_ADAPTER_BYTES);
    let mut batches = source
        .read(ReadRequest::all().spans(requested).budget(budget))
        .unwrap_or_else(|error| panic!("start sparse {organization:?} read: {error}"));
    let mut actual = Vec::new();
    while let Some(batch) = batches
        .next()
        .unwrap_or_else(|error| panic!("read sparse {organization:?} batch: {error}"))
    {
        actual.extend(
            batch
                .positions()
                .ticks()
                .iter()
                .copied()
                .enumerate()
                .map(|(row, ticks)| (batch.first_ordinal() + u64::try_from(row).unwrap(), ticks)),
        );
    }

    let expected_ordinals = [
        7_u64,
        8,
        u64::try_from(POINT_COUNT - 3).unwrap(),
        u64::try_from(POINT_COUNT - 2).unwrap(),
        u64::try_from(POINT_COUNT - 1).unwrap(),
    ];
    assert_eq!(
        actual,
        expected_ordinals
            .into_iter()
            .map(|ordinal| (ordinal, ticks(usize::try_from(ordinal).unwrap())))
            .collect::<Vec<_>>(),
        "{organization:?}"
    );
    let summary = batches
        .summary()
        .expect("successful sparse read has summary");
    assert_eq!(summary.spans(), &requested);
    assert_eq!(summary.exact_count(), 5);
    assert_eq!(summary.budget(), budget);
}

fn assert_cancellable_replay(source: &Source, organization: SequentialOrganization) {
    let budget = ReadBudget::new(1, GENEROUS_PAYLOAD_BYTES)
        .unwrap()
        .with_max_spans(1)
        .with_max_points(1)
        .with_max_adapter_working_bytes(GENEROUS_ADAPTER_BYTES);
    let request = ReadRequest::all()
        .spans([span(POINT_COUNT - 1, 1)])
        .budget(budget);
    let mut batches = source
        .read(request)
        .unwrap_or_else(|error| panic!("start cancellable {organization:?} read: {error}"));
    let handle = batches.handle();
    let start = Arc::new(Barrier::new(2));
    let worker_start = Arc::clone(&start);
    let worker = thread::spawn(move || {
        worker_start.wait();
        let first = batches.next();
        assert!(
            matches!(first, Err(SourceError::Cancelled)),
            "{organization:?} replay returned {first:?}"
        );
        assert!(batches.next().unwrap().is_none());
        assert!(batches.summary().is_none());
    });

    start.wait();
    thread::sleep(Duration::from_millis(2));
    handle.cancel();
    worker.join().expect("cancellable replay thread succeeds");
}

fn write_fixture(path: &Path, organization: SequentialOrganization) {
    let vlr = LazVlrBuilder::default().with_point_format(0, 0).unwrap();
    let vlr = match organization {
        SequentialOrganization::PointWise => vlr
            .with_fixed_chunk_size(u32::try_from(POINT_COUNT + 1).unwrap())
            .build(),
        SequentialOrganization::VariableChunks => vlr.with_variable_chunk_size().build(),
    };
    write_chunked(path, &vlr, organization);
    if matches!(organization, SequentialOrganization::PointWise) {
        convert_single_chunk_to_pointwise(path);
    }
}

fn write_chunked(path: &Path, vlr: &LazVlr, organization: SequentialOrganization) {
    let header = fixture_header(vlr);
    let mut file = File::create(path).expect("create sequential LAZ fixture");
    header
        .write_to(&mut file)
        .expect("write sequential LAZ header");
    let format = *header.point_format();
    let transforms = *header.transforms();
    let mut compressor = LasZipCompressor::new(file, vlr.clone()).expect("create LAZ compressor");
    let variable_chunks = [3_usize, 17, 257, 1_021];
    let mut chunk_index = 0;
    let mut remaining_in_chunk = variable_chunks[chunk_index];

    for ordinal in 0..POINT_COUNT {
        let raw = fixture_point(ordinal)
            .into_raw(&transforms)
            .expect("fixture point fits LAS transforms");
        let mut bytes = Vec::with_capacity(usize::from(format.len()));
        raw.write_to(&mut bytes, &format)
            .expect("encode raw LAS point");
        compressor
            .compress_one(&bytes)
            .expect("compress sequential LAZ point");

        if matches!(organization, SequentialOrganization::VariableChunks) {
            remaining_in_chunk -= 1;
            if remaining_in_chunk == 0 && ordinal + 1 != POINT_COUNT {
                compressor
                    .finish_current_chunk()
                    .expect("finish variable LAZ chunk");
                chunk_index = (chunk_index + 1) % variable_chunks.len();
                remaining_in_chunk = variable_chunks[chunk_index];
            }
        }
    }
    compressor.done().expect("finish sequential LAZ fixture");
    let file = compressor.into_inner();
    file.sync_all().expect("flush sequential LAZ fixture");
}

fn fixture_header(vlr: &LazVlr) -> Header {
    let mut builder = Builder::from((1, 4));
    let mut format = Format::new(0).unwrap();
    format.is_compressed = true;
    builder.point_format = format;
    builder.transforms = Vector {
        x: Transform {
            scale: 1.0,
            offset: 0.0,
        },
        y: Transform {
            scale: 1.0,
            offset: 0.0,
        },
        z: Transform {
            scale: 1.0,
            offset: 0.0,
        },
    };
    let mut vlr_data = Vec::new();
    vlr.write_to(&mut vlr_data).expect("encode LASzip VLR");
    builder.vlrs.push(Vlr {
        user_id: LazVlr::USER_ID.to_owned(),
        record_id: LazVlr::RECORD_ID,
        description: LazVlr::DESCRIPTION.to_owned(),
        data: vlr_data,
    });
    let mut header = builder.into_header().expect("build sequential LAZ header");
    for ordinal in 0..POINT_COUNT {
        header.add_point(&fixture_point(ordinal));
    }
    header
}

fn fixture_point(ordinal: usize) -> Point {
    let ticks = ticks(ordinal);
    let world = ticks.map(|tick| f64::from(i32::try_from(tick).unwrap()));
    Point {
        x: world[0],
        y: world[1],
        z: world[2],
        return_number: 1,
        number_of_returns: 1,
        ..Point::default()
    }
}

fn ticks(ordinal: usize) -> [i64; 3] {
    let ordinal = i64::try_from(ordinal).unwrap();
    [ordinal, ordinal * 2 - 100, ordinal % 97 - 48]
}

fn convert_single_chunk_to_pointwise(path: &Path) {
    // Compressor organizations 1 and 2 use the same pointwise record coding.
    // Organization 1 stores one uninterrupted stream without organization 2's
    // leading chunk-table offset or trailing table.
    let bytes = fs::read(path).expect("read chunked LAZ fixture");
    let point_offset = usize::try_from(u32::from_le_bytes(bytes[96..100].try_into().unwrap()))
        .expect("point offset fits usize");
    let chunk_table_offset = i64::from_le_bytes(
        bytes[point_offset..point_offset + LASZIP_CHUNK_OFFSET_BYTES]
            .try_into()
            .unwrap(),
    );
    let chunk_table_offset = usize::try_from(chunk_table_offset)
        .expect("chunk table offset is a positive addressable value");
    assert!(chunk_table_offset <= bytes.len());

    let mut pointwise = bytes[..point_offset].to_vec();
    let vlr_data_offset = laszip_vlr_data_offset(&pointwise);
    pointwise[vlr_data_offset..vlr_data_offset + 2].copy_from_slice(&1_u16.to_le_bytes());
    pointwise
        .extend_from_slice(&bytes[point_offset + LASZIP_CHUNK_OFFSET_BYTES..chunk_table_offset]);
    fs::write(path, pointwise).expect("write pointwise LAZ fixture");
}

fn assert_fixture_organization(path: &Path, organization: SequentialOrganization) {
    let bytes = fs::read(path).expect("read sequential LAZ organization");
    let vlr_data_offset = laszip_vlr_data_offset(&bytes);
    let compressor = u16::from_le_bytes(
        bytes[vlr_data_offset..vlr_data_offset + 2]
            .try_into()
            .unwrap(),
    );
    match organization {
        SequentialOrganization::PointWise => assert_eq!(compressor, 1),
        SequentialOrganization::VariableChunks => {
            assert_eq!(compressor, 2);
            let chunk_size = u32::from_le_bytes(
                bytes[vlr_data_offset + 12..vlr_data_offset + 16]
                    .try_into()
                    .unwrap(),
            );
            assert_eq!(chunk_size, u32::MAX);
        }
    }
}

fn laszip_vlr_data_offset(bytes: &[u8]) -> usize {
    let header_size = usize::from(u16::from_le_bytes(bytes[94..96].try_into().unwrap()));
    let vlr_start = header_size;
    assert_eq!(
        &bytes[vlr_start + 2..vlr_start + 2 + LazVlr::USER_ID.len()],
        LazVlr::USER_ID.as_bytes()
    );
    assert_eq!(
        u16::from_le_bytes(bytes[vlr_start + 18..vlr_start + 20].try_into().unwrap()),
        LazVlr::RECORD_ID
    );
    vlr_start + LAS_VLR_HEADER_BYTES
}

fn span(first: usize, count: u64) -> SourceSpan {
    SourceSpan::new(u64::try_from(first).unwrap(), count).unwrap()
}

struct FixtureDirectory(PathBuf);

impl FixtureDirectory {
    fn new() -> Self {
        static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "punctra-source-las-sequential-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create sequential LAZ fixture directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
