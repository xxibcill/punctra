//! Exact field coverage for every supported LAS and LAZ point-record format.

use std::fs::{self, FileTimes, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use las::point::{Classification, Format, ScanDirection};
use las::raw::point::Waveform;
use las::{Builder, Color, Point, Transform, Vector, Writer};
use point_contracts::{AttributeDataType, AttributeId, AttributeValues, PointBatch};
use point_source::SourceError;

const EXTRA_BYTES_WIDTH: u16 = 2;
const POINT_COUNT: usize = 2;
const CANCELLATION_FIXTURE_POINTS: usize = 5_000;
static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

#[test]
fn every_supported_point_format_decodes_without_field_misalignment() {
    let directory = FixtureDirectory::new();
    for point_format in 0..=10_u8 {
        assert_supported_format(&directory, point_format, "las");
    }
    for point_format in (0..=8_u8).filter(|point_format| !matches!(point_format, 4 | 5)) {
        assert_supported_format(&directory, point_format, "laz");
    }
}

#[test]
fn legacy_waveform_laz_formats_preserve_changing_packets() {
    let directory = FixtureDirectory::new();
    assert_ne!(
        waveform(0).byte_offset_to_waveform_data,
        waveform(1).byte_offset_to_waveform_data
    );
    for point_format in [4_u8, 5] {
        assert_supported_format(&directory, point_format, "laz");
    }
}

#[test]
fn layered_waveform_laz_formats_are_explicitly_unsupported() {
    let directory = FixtureDirectory::new();
    for point_format in [9_u8, 10] {
        let path = directory
            .path()
            .join(format!("unsupported-format-{point_format}.laz"));
        write_fixture(&path, point_format);
        let Err(error) = source_las::open(&path).blocking_wait() else {
            panic!("LAZ point format {point_format} unexpectedly opened");
        };
        match error {
            SourceError::UnsupportedFormat { format } => assert_eq!(
                format.as_str(),
                format!("LAZ point format {point_format} (layered WavePacket14)")
            ),
            other => panic!("LAZ point format {point_format} returned {other}"),
        }
    }
}

#[test]
fn conflicting_las_1_4_point_counts_are_corrupt() {
    let directory = FixtureDirectory::new();
    let path = directory.path().join("conflicting-point-counts.las");
    write_fixture(&path, 8);

    let mut bytes = fs::read(&path).unwrap();
    assert_eq!(
        u32::from_le_bytes(bytes[107..111].try_into().unwrap()),
        u32::try_from(POINT_COUNT).unwrap()
    );
    bytes[107..111].copy_from_slice(&1_u32.to_le_bytes());
    fs::write(&path, bytes).unwrap();

    assert!(matches!(
        source_las::open(&path).blocking_wait(),
        Err(SourceError::CorruptSource { .. })
    ));
}

#[test]
fn verified_source_reads_from_immutable_bytes() {
    let directory = FixtureDirectory::new();
    let path = directory.path().join("immutable-source.las");
    write_fixture(&path, 8);
    let source = source_las::open(&path).blocking_wait().unwrap();
    let original_modified = fs::metadata(&path).unwrap().modified().unwrap();

    let mut bytes = fs::read(&path).unwrap();
    let point_offset =
        usize::try_from(u32::from_le_bytes(bytes[96..100].try_into().unwrap())).unwrap();
    bytes[point_offset..point_offset + 4].copy_from_slice(&(-5_i32).to_le_bytes());
    fs::write(&path, bytes).unwrap();
    OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(original_modified))
        .unwrap();

    let mut batches = source.points().unwrap();
    let batch = batches.next().unwrap().unwrap();
    assert_eq!(batch.positions().ticks()[0], ticks(0));
}

#[test]
fn permissive_budget_still_allows_cancellation_between_decode_quanta() {
    let directory = FixtureDirectory::new();
    let path = directory.path().join("cancellation-quanta.las");
    write_cancellation_fixture(&path);
    let source = source_las::open(&path).blocking_wait().unwrap();
    let permissive = point_source::ReadBudget::new(u64::MAX, u64::MAX)
        .unwrap()
        .with_max_adapter_working_bytes(u64::MAX);
    let mut batches = source
        .read(point_source::ReadRequest::all().budget(permissive))
        .unwrap();

    let first = batches.next().unwrap().unwrap();
    assert!(first.len() < CANCELLATION_FIXTURE_POINTS);
    batches.handle().cancel();
    assert!(matches!(batches.next(), Err(SourceError::Cancelled)));
    assert!(batches.next().unwrap().is_none());
    assert!(batches.summary().is_none());
}

fn assert_supported_format(directory: &FixtureDirectory, point_format: u8, extension: &str) {
    let path = directory
        .path()
        .join(format!("format-{point_format}.{extension}"));
    write_fixture(&path, point_format);
    let source = source_las::open(&path)
        .blocking_wait()
        .unwrap_or_else(|error| panic!("open point format {point_format} {extension}: {error}"));
    assert_source(&source, point_format, extension);
}

fn assert_source(source: &point_source::Source, point_format: u8, extension: &str) {
    let format = format_with_extra_bytes(point_format);
    assert_eq!(source.metadata().point_count(), POINT_COUNT as u64);
    assert_eq!(
        source.metadata().attributes().len(),
        expected_ids(format).len()
    );
    assert_eq!(
        source
            .metadata()
            .attributes()
            .definitions()
            .iter()
            .map(|definition| definition.id().get())
            .collect::<Vec<_>>(),
        expected_ids(format),
        "schema for point format {point_format} {extension}"
    );
    let scan_angle = source
        .metadata()
        .attributes()
        .get(attribute_id(12))
        .unwrap();
    assert_eq!(
        scan_angle.data_type(),
        if format.is_extended {
            AttributeDataType::I16
        } else {
            AttributeDataType::I8
        }
    );

    let mut batches = source.points().unwrap();
    let batch = batches.next().unwrap().unwrap();
    assert_eq!(batch.len(), POINT_COUNT);
    for row in 0..POINT_COUNT {
        assert_row(&batch, format, row);
    }
    assert!(batches.next().unwrap().is_none());
    assert_eq!(batches.summary().unwrap().exact_count(), POINT_COUNT as u64);
}

fn assert_row(batch: &PointBatch, format: Format, row: usize) {
    let small = u8::try_from(row).unwrap();
    assert_eq!(batch.positions().ticks()[row], ticks(row));
    assert_eq!(u16_value(batch, 1, row), 1_000 + u16::from(small));
    assert_eq!(u8_value(batch, 2, row), 1);
    assert_eq!(u8_value(batch, 3, row), 1);
    assert_eq!(u8_value(batch, 4, row), 1);
    assert_eq!(u8_value(batch, 5, row), 1);
    assert_eq!(u8_value(batch, 6, row), 2);
    assert_eq!(u8_value(batch, 7, row), 1);
    assert_eq!(u8_value(batch, 8, row), 1);
    assert_eq!(u8_value(batch, 9, row), 1);
    assert_eq!(u8_value(batch, 10, row), 0);
    if format.is_extended {
        assert_eq!(u8_value(batch, 11, row), small % 4);
        assert_eq!(values(batch, 12).as_i16().unwrap()[row], 1_000);
    } else {
        assert_eq!(values(batch, 12).as_i8().unwrap()[row], 6);
    }
    assert_eq!(u8_value(batch, 13, row), 20 + small);
    assert_eq!(u16_value(batch, 14, row), 300 + u16::from(small));
    assert_optional_values(batch, format, row, small);
    let (width, payload) = values(batch, 4096).as_fixed_bytes().unwrap();
    assert_eq!(width, u32::from(EXTRA_BYTES_WIDTH));
    assert_eq!(&payload[row * 2..row * 2 + 2], &[small, 0xa5]);
}

fn assert_optional_values(batch: &PointBatch, format: Format, row: usize, small: u8) {
    if format.has_gps_time {
        assert_eq!(
            values(batch, 15).as_f64().unwrap()[row].to_bits(),
            (1_000.25 + f64::from(small)).to_bits()
        );
    }
    if format.has_color {
        assert_eq!(u16_value(batch, 16, row), 10_000 + u16::from(small));
        assert_eq!(u16_value(batch, 17, row), 20_000 + u16::from(small));
        assert_eq!(u16_value(batch, 18, row), 30_000 + u16::from(small));
    }
    if format.has_nir {
        assert_eq!(u16_value(batch, 26, row), 40_000 + u16::from(small));
    }
    if format.has_waveform {
        let waveform = waveform(row);
        assert_eq!(
            u8_value(batch, 19, row),
            waveform.wave_packet_descriptor_index
        );
        assert_eq!(
            values(batch, 20).as_u64().unwrap()[row],
            waveform.byte_offset_to_waveform_data
        );
        assert_eq!(
            values(batch, 21).as_u32().unwrap()[row],
            waveform.waveform_packet_size_in_bytes
        );
        assert_eq!(
            values(batch, 22).as_f32().unwrap()[row].to_bits(),
            waveform.return_point_waveform_location.to_bits()
        );
        assert_eq!(
            values(batch, 23).as_f32().unwrap()[row].to_bits(),
            waveform.x_t.to_bits()
        );
        assert_eq!(
            values(batch, 24).as_f32().unwrap()[row].to_bits(),
            waveform.y_t.to_bits()
        );
        assert_eq!(
            values(batch, 25).as_f32().unwrap()[row].to_bits(),
            waveform.z_t.to_bits()
        );
    }
}

fn expected_ids(format: Format) -> Vec<u32> {
    let mut ids = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 13, 14, 4096];
    if format.is_extended {
        ids.push(11);
    }
    if format.has_gps_time {
        ids.push(15);
    }
    if format.has_color {
        ids.extend([16, 17, 18]);
    }
    if format.has_waveform {
        ids.extend(19..=25);
    }
    if format.has_nir {
        ids.push(26);
    }
    ids.sort_unstable();
    ids
}

fn write_fixture(path: &Path, point_format: u8) {
    let format = format_with_extra_bytes(point_format);
    let mut builder = Builder::from((1, 4));
    builder.point_format = format;
    builder.transforms = transforms();
    let mut writer = Writer::from_path(path, builder.into_header().unwrap()).unwrap();
    for row in 0..POINT_COUNT {
        writer.write_point(point(format, row)).unwrap();
    }
    writer.close().unwrap();
}

fn write_cancellation_fixture(path: &Path) {
    let mut builder = Builder::from((1, 4));
    builder.point_format = Format::new(0).unwrap();
    let mut writer = Writer::from_path(path, builder.into_header().unwrap()).unwrap();
    for row in 0..CANCELLATION_FIXTURE_POINTS {
        writer
            .write_point(Point {
                x: f64::from(u32::try_from(row).unwrap()),
                return_number: 1,
                number_of_returns: 1,
                ..Point::default()
            })
            .unwrap();
    }
    writer.close().unwrap();
}

fn point(format: Format, row: usize) -> Point {
    let small = u8::try_from(row).unwrap();
    let world = world(ticks(row));
    Point {
        x: world[0],
        y: world[1],
        z: world[2],
        intensity: 1_000 + u16::from(small),
        return_number: 1,
        number_of_returns: 1,
        scan_direction: ScanDirection::LeftToRight,
        is_edge_of_flight_line: true,
        classification: Classification::Ground,
        is_synthetic: true,
        is_key_point: true,
        is_withheld: true,
        scanner_channel: if format.is_extended { small % 4 } else { 0 },
        scan_angle: 6.0,
        user_data: 20 + small,
        point_source_id: 300 + u16::from(small),
        gps_time: format.has_gps_time.then_some(1_000.25 + f64::from(small)),
        color: format.has_color.then_some(Color::new(
            10_000 + u16::from(small),
            20_000 + u16::from(small),
            30_000 + u16::from(small),
        )),
        waveform: format.has_waveform.then(|| waveform(row)),
        nir: format.has_nir.then_some(40_000 + u16::from(small)),
        extra_bytes: vec![small, 0xa5],
        ..Point::default()
    }
}

fn waveform(row: usize) -> Waveform {
    let small = u8::try_from(row).unwrap();
    Waveform {
        wave_packet_descriptor_index: 7 + small,
        byte_offset_to_waveform_data: 10_000 + u64::from(small) * 123,
        waveform_packet_size_in_bytes: 512 + u32::from(small) * 17,
        return_point_waveform_location: 0.25 + f32::from(small) * 0.125,
        x_t: -1.0 - f32::from(small) * 0.5,
        y_t: 2.0 + f32::from(small) * 0.75,
        z_t: 0.5 + f32::from(small) * 0.25,
    }
}

fn format_with_extra_bytes(point_format: u8) -> Format {
    let mut format = Format::new(point_format).unwrap();
    format.extra_bytes = EXTRA_BYTES_WIDTH;
    format
}

fn ticks(row: usize) -> [i64; 3] {
    let ordinal = i64::try_from(row).unwrap();
    [ordinal * 3 - 7, 5 - ordinal * 2, ordinal * ordinal - 2]
}

fn world(ticks: [i64; 3]) -> [f64; 3] {
    let ticks = ticks.map(|tick| f64::from(i32::try_from(tick).unwrap()));
    [
        ticks[0] * 0.25 + 100.0,
        ticks[1] * 0.5 - 50.0,
        ticks[2] * 2.0 + 2.0,
    ]
}

fn transforms() -> Vector<Transform> {
    Vector {
        x: Transform {
            scale: 0.25,
            offset: 100.0,
        },
        y: Transform {
            scale: 0.5,
            offset: -50.0,
        },
        z: Transform {
            scale: 2.0,
            offset: 2.0,
        },
    }
}

fn values(batch: &PointBatch, id: u32) -> &AttributeValues {
    batch.attributes().get(attribute_id(id)).unwrap().values()
}

fn u8_value(batch: &PointBatch, id: u32, row: usize) -> u8 {
    values(batch, id).as_u8().unwrap()[row]
}

fn u16_value(batch: &PointBatch, id: u32, row: usize) -> u16 {
    values(batch, id).as_u16().unwrap()[row]
}

fn attribute_id(value: u32) -> AttributeId {
    AttributeId::new(value).unwrap()
}

struct FixtureDirectory(PathBuf);

impl FixtureDirectory {
    fn new() -> Self {
        let counter = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "punctra-source-las-formats-{}-{timestamp}-{counter}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
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
