use std::collections::BTreeSet;
use std::fs::{File, Metadata};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use foundation_runtime::OperationReporter;
use las::point::Format;
use laz::laszip::ChunkTable;
use point_contracts::{
    AttributeDataType, AttributeDefinition, AttributeId, AttributeSchema, ContentHash,
    CoordinateReference, LinearUnit, MAX_COORDINATE_REFERENCE_WKT_BYTES,
    MAX_METADATA_RECORD_PAYLOAD_BYTES, MAX_METADATA_RECORDS, MAX_SOURCE_METADATA_PAYLOAD_BYTES,
    MetadataRecord, PositionTransform, SourceMetadata, SpatialAxes, SpatialReferenceProfile,
    SpatialReferenceProvenance, WorldBounds,
};
use point_source::adapter::FullVerification;
use point_source::{SourceDiagnostic, SourceError};
use tempfile::tempfile;

use crate::decode::scan_bounds;

const HASH_BUFFER_BYTES: usize = 1024 * 1024;
const VLR_HEADER_BYTES: usize = 54;
const EVLR_HEADER_BYTES: usize = 60;
const LAZ_CHUNK_ENTRY_BYTES: u64 = 16;
const LAZ_BASE_DECODER_BYTES: u64 = 768 * 1024;
const LAZ_EXTRA_BYTE_DECODER_BYTES: u64 = 16 * 1024;
const MAX_LAZ_CHUNKS: u64 = 1_000_000;

pub(crate) struct VerifiedFile {
    pub(crate) file: Arc<File>,
    pub(crate) source_witness: SourceFileWitness,
    pub(crate) layout: Arc<LasLayout>,
    pub(crate) metadata: Arc<SourceMetadata>,
    pub(crate) content_hash: ContentHash,
}

#[derive(Clone)]
pub(crate) struct SourceFileWitness {
    file: Arc<File>,
    path: Arc<PathBuf>,
    metadata: FileWitness,
}

impl SourceFileWitness {
    fn new(file: File, path: &Path, metadata: FileWitness) -> Self {
        Self {
            file: Arc::new(file),
            path: Arc::new(path.to_path_buf()),
            metadata,
        }
    }

    pub(crate) fn ensure_unchanged(&self) -> Result<(), SourceError> {
        self.metadata.ensure_file(&self.file)?;
        let path_metadata = FileWitness::for_path(&self.path).map_err(|_| {
            SourceError::changed("the verified Source path is no longer accessible")
        })?;
        if path_metadata == self.metadata {
            Ok(())
        } else {
            Err(SourceError::changed(
                "the file at the verified Source path changed",
            ))
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileWitness {
    len: u64,
    modified_nanos: Option<u128>,
}

impl FileWitness {
    fn from_metadata(metadata: &Metadata) -> Self {
        let modified_nanos = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos());
        Self {
            len: metadata.len(),
            modified_nanos,
        }
    }

    pub(crate) fn for_file(file: &File) -> Result<Self, SourceError> {
        file.metadata()
            .map(|metadata| Self::from_metadata(&metadata))
            .map_err(classify_io)
    }

    fn for_path(path: &Path) -> Result<Self, SourceError> {
        path.metadata()
            .map(|metadata| Self::from_metadata(&metadata))
            .map_err(classify_io)
    }

    pub(crate) fn ensure_file(&self, file: &File) -> Result<(), SourceError> {
        if &Self::for_file(file)? == self {
            Ok(())
        } else {
            Err(SourceError::changed("the verified file changed"))
        }
    }
}

#[derive(Clone)]
pub(crate) struct LasLayout {
    pub(crate) point_offset: u64,
    pub(crate) point_count: u64,
    pub(crate) record_len: usize,
    pub(crate) format: Format,
    pub(crate) transform: PositionTransform,
    pub(crate) declared_bounds: Option<WorldBounds>,
    pub(crate) attributes: Vec<AttributePlan>,
    pub(crate) compression: Compression,
}

#[derive(Clone)]
pub(crate) enum Compression {
    Las,
    Laz {
        vlr: laz::LazVlr,
        decoder_bytes: u64,
        seek_mode: LazSeekMode,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LazSeekMode {
    FixedChunks { points_per_chunk: u64 },
    Sequential,
}

impl Compression {
    pub(crate) const fn decoder_bytes(&self) -> u64 {
        match self {
            Self::Las => 0,
            Self::Laz { decoder_bytes, .. } => *decoder_bytes,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum AttributeKind {
    Intensity,
    ReturnNumber,
    NumberOfReturns,
    ScanDirection,
    EdgeOfFlightLine,
    Classification,
    Synthetic,
    KeyPoint,
    Withheld,
    Overlap,
    ScannerChannel,
    ScanAngleI8,
    ScanAngleI16,
    UserData,
    PointSourceId,
    GpsTime,
    Red,
    Green,
    Blue,
    WaveformDescriptor,
    WaveformOffset,
    WaveformSize,
    WaveformLocation,
    WaveformDx,
    WaveformDy,
    WaveformDz,
    Nir,
    ExtraBytes(u32),
}

#[derive(Clone)]
pub(crate) struct AttributePlan {
    pub(crate) definition: AttributeDefinition,
    pub(crate) kind: AttributeKind,
}

pub(crate) fn verify_file(
    path: &Path,
    verification: FullVerification,
    reporter: &OperationReporter,
) -> Result<VerifiedFile, SourceError> {
    reporter.check_cancelled()?;
    let mut source_file = File::open(path).map_err(classify_io)?;
    let source_witness = FileWitness::for_file(&source_file)?;
    let mut file = tempfile().map_err(classify_io)?;
    let content_hash =
        snapshot_and_hash(&mut source_file, &mut file, source_witness.len, reporter)?;
    if verification
        .expected_content_hash()
        .is_some_and(|expected| expected != content_hash)
    {
        return Err(SourceError::changed(
            "the exact file-byte hash differs from the record",
        ));
    }

    source_witness.ensure_file(&source_file)?;
    if FileWitness::for_path(path)? != source_witness {
        return Err(SourceError::changed(
            "the file at the verified path changed",
        ));
    }
    let snapshot_witness = FileWitness::for_file(&file)?;
    let parsed = parse_layout(&mut file, snapshot_witness.len)?;
    let layout = Arc::new(parsed.layout);
    let bounds = scan_bounds(&file, &snapshot_witness, &layout, reporter)?;
    snapshot_witness.ensure_file(&file)?;

    let schema = AttributeSchema::new(
        layout
            .attributes
            .iter()
            .map(|attribute| attribute.definition.clone())
            .collect(),
    )
    .map_err(contract_error)?;
    let metadata = SourceMetadata::new(
        layout.point_count,
        layout.transform,
        parsed.coordinate_reference,
        schema,
        bounds,
        parsed.format_name,
        parsed.metadata_records,
    )
    .map_err(contract_error)?;

    Ok(VerifiedFile {
        file: Arc::new(file),
        source_witness: SourceFileWitness::new(source_file, path, source_witness),
        layout,
        metadata: Arc::new(metadata),
        content_hash,
    })
}

struct ParsedLayout {
    layout: LasLayout,
    coordinate_reference: CoordinateReference,
    metadata_records: Vec<MetadataRecord>,
    format_name: &'static str,
}

#[derive(Clone, Copy)]
struct FileLayoutFacts {
    file_len: u64,
    point_offset: u64,
    evlr_start: Option<u64>,
    record_len: u64,
    point_count: u64,
    format: Format,
}

fn parse_layout(file: &mut File, file_len: u64) -> Result<ParsedLayout, SourceError> {
    file.seek(SeekFrom::Start(0)).map_err(classify_io)?;
    let header = las::raw::Header::read_from(&mut *file)
        .map_err(|error| SourceError::corrupt(error.to_string()))?;
    let point_offset = u64::from(header.offset_to_point_data);
    if u64::from(header.header_size) > point_offset || point_offset > file_len {
        return Err(SourceError::corrupt(
            "invalid LAS header or point-data offset",
        ));
    }

    let mut format = Format::new(header.point_data_record_format)
        .map_err(|error| SourceError::corrupt(error.to_string()))?;
    reject_unsupported_compressed_format(format)?;
    let base_len = u64::from(format.len());
    let record_len = u64::from(header.point_data_record_length);
    if record_len < base_len || record_len == 0 {
        return Err(SourceError::corrupt(
            "point-record length is smaller than its LAS format",
        ));
    }
    format.extra_bytes = u16::try_from(record_len - base_len)
        .map_err(|_| SourceError::corrupt("point-record extra-byte width does not fit u16"))?;

    let point_count = point_count(&header)?;
    let transform = PositionTransform::new(
        [header.x_offset, header.y_offset, header.z_offset],
        [
            header.x_scale_factor,
            header.y_scale_factor,
            header.z_scale_factor,
        ],
    )
    .map_err(contract_error)?;
    let declared_bounds = if point_count == 0 {
        None
    } else {
        Some(
            WorldBounds::new(
                [header.min_x, header.min_y, header.min_z],
                [header.max_x, header.max_y, header.max_z],
            )
            .map_err(contract_error)?,
        )
    };

    let regular_count = u64::from(header.number_of_variable_length_records);
    let extended_count = header
        .evlr
        .map_or(0, |evlr| u64::from(evlr.number_of_evlrs));
    let mut metadata_budget = MetadataReadBudget::new();
    metadata_budget.reserve_records(regular_count)?;
    metadata_budget.reserve_records(extended_count)?;

    let regular = read_vlrs(
        file,
        u64::from(header.header_size),
        regular_count,
        false,
        point_offset,
        &mut metadata_budget,
    )?;
    let evlr_start = header.evlr.map(|evlr| evlr.start_of_first_evlr);
    let extended = match header.evlr {
        Some(evlr) => read_vlrs(
            file,
            evlr.start_of_first_evlr,
            u64::from(evlr.number_of_evlrs),
            true,
            file_len,
            &mut metadata_budget,
        )?,
        None => Vec::new(),
    };

    let facts = FileLayoutFacts {
        file_len,
        point_offset,
        evlr_start,
        record_len,
        point_count,
        format,
    };
    let compression = if format.is_compressed {
        compression(file, facts, &regular)?
    } else {
        validate_uncompressed_points(facts)?
    };

    let coordinate_reference = coordinate_reference(&regular, &extended)?;
    let metadata_records = metadata_records(regular, extended)?;
    Ok(ParsedLayout {
        layout: LasLayout {
            point_offset,
            point_count,
            record_len: usize::try_from(record_len)
                .map_err(|_| SourceError::corrupt("point-record length does not fit usize"))?,
            format,
            transform,
            declared_bounds,
            attributes: attribute_plans(format)?,
            compression,
        },
        coordinate_reference,
        metadata_records,
        format_name: if format.is_compressed { "LAZ" } else { "LAS" },
    })
}

fn reject_unsupported_compressed_format(format: Format) -> Result<(), SourceError> {
    if format.is_compressed && format.is_extended && format.has_waveform {
        let point_format = format
            .to_u8()
            .map_err(|error| SourceError::corrupt(error.to_string()))?;
        return Err(SourceError::UnsupportedFormat {
            format: SourceDiagnostic::new(format!(
                "LAZ point format {point_format} (layered WavePacket14)"
            )),
        });
    }
    Ok(())
}

fn validate_uncompressed_points(facts: FileLayoutFacts) -> Result<Compression, SourceError> {
    let point_bytes = facts
        .point_count
        .checked_mul(facts.record_len)
        .ok_or_else(|| SourceError::corrupt("point-data byte length overflows"))?;
    let end = facts
        .point_offset
        .checked_add(point_bytes)
        .ok_or_else(|| SourceError::corrupt("point-data end offset overflows"))?;
    if end > facts.evlr_start.unwrap_or(facts.file_len) {
        return Err(SourceError::corrupt(
            "uncompressed point records extend beyond the file",
        ));
    }
    Ok(Compression::Las)
}

fn point_count(header: &las::raw::Header) -> Result<u64, SourceError> {
    let legacy = u64::from(header.number_of_point_records);
    let Some(large_file) = header.large_file else {
        return Ok(legacy);
    };
    let extended = large_file.number_of_point_records;
    if legacy != 0 && legacy != extended {
        return Err(SourceError::corrupt(format!(
            "LAS 1.4 point counts disagree: legacy count {legacy}, extended count {extended}"
        )));
    }
    Ok(if legacy == 0 { extended } else { legacy })
}

struct RawVlr {
    user_id: String,
    record_id: u16,
    description: String,
    data: Vec<u8>,
}

struct MetadataReadBudget {
    record_count: u64,
    payload_bytes: u64,
}

impl MetadataReadBudget {
    const fn new() -> Self {
        Self {
            record_count: 0,
            payload_bytes: 0,
        }
    }

    fn reserve_records(&mut self, count: u64) -> Result<(), SourceError> {
        let record_count = self
            .record_count
            .checked_add(count)
            .ok_or_else(|| SourceError::corrupt("LAS metadata record count overflows"))?;
        if record_count > MAX_METADATA_RECORDS as u64 {
            return Err(SourceError::corrupt(
                "LAS metadata record count exceeds the canonical cap",
            ));
        }
        self.record_count = record_count;
        Ok(())
    }

    fn reserve_payload(&mut self, payload_bytes: u64) -> Result<(), SourceError> {
        let total = self
            .payload_bytes
            .checked_add(payload_bytes)
            .ok_or_else(|| SourceError::corrupt("LAS metadata payload length overflows"))?;
        if total > MAX_SOURCE_METADATA_PAYLOAD_BYTES as u64 {
            return Err(SourceError::corrupt(
                "LAS metadata exceeds the canonical total cap",
            ));
        }
        self.payload_bytes = total;
        Ok(())
    }
}

fn read_vlrs(
    file: &mut File,
    start: u64,
    count: u64,
    extended: bool,
    limit: u64,
    budget: &mut MetadataReadBudget,
) -> Result<Vec<RawVlr>, SourceError> {
    file.seek(SeekFrom::Start(start)).map_err(classify_io)?;
    let mut reader = BufReader::new(file);
    let mut records = Vec::with_capacity(
        usize::try_from(count).map_err(|_| SourceError::corrupt("metadata count overflow"))?,
    );
    for _ in 0..count {
        records.push(read_vlr(&mut reader, extended, limit, budget)?);
    }
    Ok(records)
}

struct RawVlrHeader {
    start: u64,
    len: u64,
    payload_len: u64,
    description_start: usize,
    fixed: [u8; EVLR_HEADER_BYTES],
}

fn read_vlr_header(
    reader: &mut BufReader<&mut File>,
    extended: bool,
    limit: u64,
) -> Result<RawVlrHeader, SourceError> {
    let header_len = if extended {
        EVLR_HEADER_BYTES
    } else {
        VLR_HEADER_BYTES
    };
    let start = reader.stream_position().map_err(classify_io)?;
    let len = u64::try_from(header_len)
        .map_err(|_| metadata_corrupt(extended, "header", start, "length does not fit u64"))?;
    if start.checked_add(len).is_none_or(|end| end > limit) {
        return Err(metadata_corrupt(
            extended,
            "header",
            start,
            "extends beyond its LAS section",
        ));
    }
    let mut fixed = [0_u8; EVLR_HEADER_BYTES];
    reader
        .read_exact(&mut fixed[..header_len])
        .map_err(classify_io)
        .map_err(|error| contextualize_metadata_error(error, extended, "header", start))?;
    let (payload_len, description_start) = if extended {
        (
            u64::from_le_bytes(fixed[20..28].try_into().map_err(|_| {
                metadata_corrupt(extended, "header", start, "truncated payload length")
            })?),
            28,
        )
    } else {
        (
            u64::from(u16::from_le_bytes(fixed[20..22].try_into().map_err(
                |_| metadata_corrupt(extended, "header", start, "truncated payload length"),
            )?)),
            22,
        )
    };
    if payload_len > MAX_METADATA_RECORD_PAYLOAD_BYTES as u64 {
        return Err(metadata_corrupt(
            extended,
            "payload",
            start,
            "exceeds the canonical per-record cap",
        ));
    }
    Ok(RawVlrHeader {
        start,
        len,
        payload_len,
        description_start,
        fixed,
    })
}

fn read_vlr(
    reader: &mut BufReader<&mut File>,
    extended: bool,
    limit: u64,
    budget: &mut MetadataReadBudget,
) -> Result<RawVlr, SourceError> {
    let header = read_vlr_header(reader, extended, limit)?;
    let user_id = las_text(&header.fixed[2..18], "user ID")
        .map_err(|error| contextualize_metadata_error(error, extended, "header", header.start))?;
    let description = las_text(
        &header.fixed[header.description_start..header.description_start + 32],
        "description",
    )
    .map_err(|error| contextualize_metadata_error(error, extended, "header", header.start))?;
    budget
        .reserve_payload(header.payload_len)
        .map_err(|error| contextualize_metadata_error(error, extended, "payload", header.start))?;
    let payload_end = header
        .start
        .checked_add(header.len)
        .and_then(|offset| offset.checked_add(header.payload_len))
        .ok_or_else(|| {
            metadata_corrupt(extended, "payload", header.start, "end offset overflows")
        })?;
    if payload_end > limit {
        return Err(metadata_corrupt(
            extended,
            "payload",
            header.start,
            "extends beyond its LAS section",
        ));
    }
    let mut data = vec![
        0_u8;
        usize::try_from(header.payload_len).map_err(|_| metadata_corrupt(
            extended,
            "payload",
            header.start,
            "length does not fit usize"
        ))?
    ];
    reader
        .read_exact(&mut data)
        .map_err(classify_io)
        .map_err(|error| contextualize_metadata_error(error, extended, "payload", header.start))?;
    let record_id = u16::from_le_bytes(header.fixed[18..20].try_into().map_err(|_| {
        metadata_corrupt(
            extended,
            "header",
            header.start,
            "truncated record identity",
        )
    })?);
    Ok(RawVlr {
        user_id,
        record_id,
        description,
        data,
    })
}

fn contextualize_metadata_error(
    error: SourceError,
    extended: bool,
    phase: &str,
    byte_offset: u64,
) -> SourceError {
    match error {
        SourceError::CorruptSource { reason } => {
            metadata_corrupt(extended, phase, byte_offset, reason)
        }
        error => error,
    }
}

fn metadata_corrupt(
    extended: bool,
    phase: &str,
    byte_offset: u64,
    reason: impl std::fmt::Display,
) -> SourceError {
    let section = if extended { "EVLR" } else { "VLR" };
    SourceError::corrupt(format!("{section} {phase} at byte {byte_offset}: {reason}"))
}

fn compression(
    file: &mut File,
    facts: FileLayoutFacts,
    records: &[RawVlr],
) -> Result<Compression, SourceError> {
    let mut matching_records = records.iter().filter(|record| {
        record.user_id == laz::LazVlr::USER_ID && record.record_id == laz::LazVlr::RECORD_ID
    });
    let record = matching_records
        .next()
        .ok_or_else(|| SourceError::corrupt("compressed LAS has no LASzip VLR"))?;
    if matching_records.next().is_some() {
        return Err(SourceError::corrupt(
            "compressed LAS has more than one LASzip VLR",
        ));
    }
    let vlr = laz::LazVlr::from_buffer(&record.data)
        .map_err(|error| SourceError::corrupt(error.to_string()))?;
    validate_laz_items(&vlr, facts.format, facts.record_len)?;
    let compressor = laz_compressor(&record.data)?;
    let decoder_bytes = preflight_chunk_table(file, facts, compressor, &vlr)?;
    let seek_mode = laz_seek_mode(compressor, &vlr);
    Ok(Compression::Laz {
        vlr,
        decoder_bytes,
        seek_mode,
    })
}

fn laz_compressor(vlr_bytes: &[u8]) -> Result<u16, SourceError> {
    let bytes = vlr_bytes
        .get(0..2)
        .ok_or_else(|| SourceError::corrupt("truncated LASzip VLR"))?;
    bytes
        .try_into()
        .map(u16::from_le_bytes)
        .map_err(slice_error)
}

fn laz_seek_mode(compressor: u16, vlr: &laz::LazVlr) -> LazSeekMode {
    if matches!(compressor, 2 | 3) && !vlr.uses_variable_size_chunks() {
        LazSeekMode::FixedChunks {
            points_per_chunk: u64::from(vlr.chunk_size()),
        }
    } else {
        LazSeekMode::Sequential
    }
}

fn validate_laz_items(
    vlr: &laz::LazVlr,
    format: Format,
    record_len: u64,
) -> Result<(), SourceError> {
    let point_format = format
        .to_u8()
        .map_err(|error| SourceError::corrupt(error.to_string()))?;
    let expected =
        laz::LazItemRecordBuilder::default_for_point_format_id(point_format, format.extra_bytes)
            .map_err(|error| SourceError::corrupt(error.to_string()))?;
    if vlr.items().len() != expected.len()
        || vlr.items().iter().zip(&expected).any(|(actual, expected)| {
            actual.item_type() != expected.item_type() || actual.size() != expected.size()
        })
    {
        return Err(SourceError::corrupt(
            "LASzip items do not match the declared LAS point format",
        ));
    }
    if vlr.items().iter().any(|item| {
        if format.is_extended {
            item.version() != 3
        } else {
            !matches!(item.version(), 1 | 2)
        }
    }) {
        return Err(SourceError::corrupt(
            "LASzip item version is unsupported for the declared LAS point format",
        ));
    }
    let item_bytes = vlr.items().iter().try_fold(0_u64, |bytes, item| {
        bytes.checked_add(u64::from(item.size()))
    });
    if item_bytes != Some(record_len) {
        return Err(SourceError::corrupt(
            "LASzip item width differs from the LAS point-record length",
        ));
    }
    Ok(())
}

fn preflight_chunk_table(
    file: &mut File,
    facts: FileLayoutFacts,
    compressor: u16,
    vlr: &laz::LazVlr,
) -> Result<u64, SourceError> {
    let model_bytes = laz_decoder_model_bytes(vlr)?;
    if compressor == 1 {
        return Ok(model_bytes);
    }
    if !matches!(compressor, 2 | 3) {
        return Err(SourceError::corrupt(
            "unsupported LASzip compressor organization",
        ));
    }
    let section_end = facts.evlr_start.unwrap_or(facts.file_len);
    if facts
        .point_offset
        .checked_add(8)
        .is_none_or(|end| end > section_end)
    {
        return Err(SourceError::corrupt("truncated LASzip chunk-table offset"));
    }
    file.seek(SeekFrom::Start(facts.point_offset))
        .map_err(classify_io)?;
    let mut offset_bytes = [0_u8; 8];
    file.read_exact(&mut offset_bytes).map_err(classify_io)?;
    let mut table_offset = i64::from_le_bytes(offset_bytes);
    if table_offset <= i64::try_from(facts.point_offset).unwrap_or(i64::MAX) {
        if facts.file_len < 8 {
            return Err(SourceError::corrupt("missing LASzip chunk table"));
        }
        file.seek(SeekFrom::End(-8)).map_err(classify_io)?;
        file.read_exact(&mut offset_bytes).map_err(classify_io)?;
        table_offset = i64::from_le_bytes(offset_bytes);
    }
    let table_offset = u64::try_from(table_offset)
        .map_err(|_| SourceError::corrupt("invalid LASzip chunk-table offset"))?;
    if table_offset <= facts.point_offset
        || table_offset
            .checked_add(8)
            .is_none_or(|end| end > section_end)
    {
        return Err(SourceError::corrupt(
            "LASzip chunk table is outside point data",
        ));
    }
    file.seek(SeekFrom::Start(table_offset + 4))
        .map_err(classify_io)?;
    let mut count = [0_u8; 4];
    file.read_exact(&mut count).map_err(classify_io)?;
    let count = u64::from(u32::from_le_bytes(count));
    if count > MAX_LAZ_CHUNKS {
        return Err(SourceError::corrupt(
            "LASzip chunk table exceeds the adapter cap",
        ));
    }
    let table_bytes = count
        .checked_mul(LAZ_CHUNK_ENTRY_BYTES)
        .ok_or_else(|| SourceError::corrupt("LASzip chunk-table allocation overflows"))?;

    file.seek(SeekFrom::Start(facts.point_offset))
        .map_err(classify_io)?;
    let table = ChunkTable::read_from(&mut *file, vlr)
        .map_err(|error| SourceError::corrupt(error.to_string()))?;
    if u64::try_from(table.len()).ok() != Some(count) {
        return Err(SourceError::corrupt(
            "LASzip chunk-table count changed while decoding",
        ));
    }
    let point_data_start = facts
        .point_offset
        .checked_add(8)
        .ok_or_else(|| SourceError::corrupt("LASzip point-data offset overflows"))?;
    let max_chunk_bytes = preflight_chunks(
        file,
        &table,
        point_data_start,
        table_offset,
        compressor == 3,
        facts.point_count,
        facts.record_len,
        vlr,
    )?;
    model_bytes
        .checked_add(table_bytes)
        .and_then(|bytes| bytes.checked_add(max_chunk_bytes))
        .ok_or_else(|| SourceError::corrupt("LASzip decoder memory estimate overflows"))
}

fn laz_decoder_model_bytes(vlr: &laz::LazVlr) -> Result<u64, SourceError> {
    let extra_bytes = vlr.items().iter().try_fold(0_u64, |bytes, item| {
        let item_bytes = match item.item_type() {
            laz::LazItemType::Byte(width) | laz::LazItemType::Byte14(width) => u64::from(width),
            _ => 0,
        };
        bytes.checked_add(item_bytes)
    });
    LAZ_BASE_DECODER_BYTES
        .checked_add(
            extra_bytes
                .ok_or_else(|| SourceError::corrupt("LASzip Extra Bytes width overflows"))?
                .checked_mul(LAZ_EXTRA_BYTE_DECODER_BYTES)
                .ok_or_else(|| SourceError::corrupt("LASzip decoder memory estimate overflows"))?,
        )
        .ok_or_else(|| SourceError::corrupt("LASzip decoder memory estimate overflows"))
}

#[allow(clippy::too_many_arguments)]
fn preflight_chunks(
    file: &mut File,
    table: &ChunkTable,
    point_data_start: u64,
    table_offset: u64,
    layered: bool,
    point_count: u64,
    record_len: u64,
    vlr: &laz::LazVlr,
) -> Result<u64, SourceError> {
    let mut chunk_start = point_data_start;
    let mut max_chunk_bytes = 0_u64;
    let mut encoded_points = 0_u64;
    let fixed_layered_chunk_size = if layered && !vlr.uses_variable_size_chunks() {
        let chunk_size = u64::from(vlr.chunk_size());
        validate_fixed_chunk_table(table.len(), point_count, chunk_size)?;
        Some(chunk_size)
    } else {
        None
    };
    for (chunk_index, entry) in table.as_ref().iter().enumerate() {
        let chunk_end = chunk_start
            .checked_add(entry.byte_count)
            .ok_or_else(|| SourceError::corrupt("LASzip chunk offset overflows"))?;
        if chunk_end > table_offset {
            return Err(SourceError::corrupt(
                "LASzip chunk extends into its chunk table",
            ));
        }
        if layered {
            let chunk_points =
                preflight_layered_chunk(file, chunk_start, entry.byte_count, record_len, vlr)?;
            if let Some(chunk_size) = fixed_layered_chunk_size {
                validate_fixed_layered_chunk_count(
                    chunk_index,
                    table.len(),
                    chunk_points,
                    point_count,
                    chunk_size,
                )?;
            }
            encoded_points = encoded_points
                .checked_add(chunk_points)
                .ok_or_else(|| SourceError::corrupt("LASzip Point count overflows"))?;
        }
        max_chunk_bytes = max_chunk_bytes.max(entry.byte_count);
        chunk_start = chunk_end;
    }
    if chunk_start != table_offset {
        return Err(SourceError::corrupt(
            "LASzip chunks do not end at the declared chunk table",
        ));
    }
    if layered && encoded_points != point_count {
        return Err(SourceError::corrupt(
            "LASzip layered chunk Point counts differ from the LAS header",
        ));
    }
    if !layered && vlr.uses_variable_size_chunks() {
        let encoded_points = table.as_ref().iter().try_fold(0_u64, |points, entry| {
            (entry.point_count != 0)
                .then(|| points.checked_add(entry.point_count))
                .flatten()
        });
        if encoded_points != Some(point_count) {
            return Err(SourceError::corrupt(
                "LASzip variable chunk Point counts differ from the LAS header",
            ));
        }
    } else if !layered {
        let chunk_size = u64::from(vlr.chunk_size());
        validate_fixed_chunk_table(table.len(), point_count, chunk_size)?;
    }
    Ok(max_chunk_bytes)
}

fn validate_fixed_chunk_table(
    table_len: usize,
    point_count: u64,
    chunk_size: u64,
) -> Result<(), SourceError> {
    if chunk_size == 0 {
        return Err(SourceError::corrupt("LASzip chunk size is zero"));
    }
    let expected_chunks = point_count.div_ceil(chunk_size);
    if u64::try_from(table_len).ok() != Some(expected_chunks) {
        return Err(SourceError::corrupt(
            "LASzip chunk count differs from the LAS header Point count",
        ));
    }
    Ok(())
}

fn validate_fixed_layered_chunk_count(
    chunk_index: usize,
    chunk_count: usize,
    actual_points: u64,
    point_count: u64,
    chunk_size: u64,
) -> Result<(), SourceError> {
    let is_final = chunk_index
        .checked_add(1)
        .is_some_and(|index| index == chunk_count);
    let expected_points = if is_final {
        let preceding_chunks = u64::try_from(chunk_index)
            .ok()
            .and_then(|index| index.checked_mul(chunk_size))
            .ok_or_else(|| SourceError::corrupt("LASzip chunk Point count overflows"))?;
        point_count
            .checked_sub(preceding_chunks)
            .filter(|remaining| *remaining != 0)
            .ok_or_else(|| SourceError::corrupt("LASzip final chunk Point count is invalid"))?
    } else {
        chunk_size
    };
    if actual_points != expected_points {
        return Err(SourceError::corrupt(
            "LASzip layered chunk Point count differs from fixed chunk boundaries",
        ));
    }
    Ok(())
}

fn preflight_layered_chunk(
    file: &mut File,
    chunk_start: u64,
    chunk_bytes: u64,
    record_len: u64,
    vlr: &laz::LazVlr,
) -> Result<u64, SourceError> {
    let layer_count = layered_layer_count(vlr)?;
    let size_table_bytes = layer_count
        .checked_mul(4)
        .ok_or_else(|| SourceError::corrupt("LASzip layer-size table overflows"))?;
    let header_bytes = record_len
        .checked_add(4)
        .and_then(|bytes| bytes.checked_add(size_table_bytes))
        .ok_or_else(|| SourceError::corrupt("LASzip layered chunk header overflows"))?;
    if header_bytes > chunk_bytes {
        return Err(SourceError::corrupt(
            "LASzip layered chunk header exceeds its chunk",
        ));
    }
    let count_offset = chunk_start
        .checked_add(record_len)
        .ok_or_else(|| SourceError::corrupt("LASzip layered chunk offset overflows"))?;
    file.seek(SeekFrom::Start(count_offset))
        .map_err(classify_io)?;
    let mut fixed = [0_u8; 4];
    file.read_exact(&mut fixed).map_err(classify_io)?;
    let point_count = u64::from(u32::from_le_bytes(fixed));
    if point_count == 0 {
        return Err(SourceError::corrupt(
            "LASzip layered chunk declares zero Points",
        ));
    }
    let mut layer_bytes = 0_u64;
    for _ in 0..layer_count {
        file.read_exact(&mut fixed).map_err(classify_io)?;
        layer_bytes = layer_bytes
            .checked_add(u64::from(u32::from_le_bytes(fixed)))
            .ok_or_else(|| SourceError::corrupt("LASzip layer payload length overflows"))?;
    }
    let encoded_bytes = header_bytes
        .checked_add(layer_bytes)
        .ok_or_else(|| SourceError::corrupt("LASzip layered chunk length overflows"))?;
    if encoded_bytes != chunk_bytes {
        return Err(SourceError::corrupt(
            "LASzip layer sizes do not cover exactly one chunk",
        ));
    }
    Ok(point_count)
}

fn layered_layer_count(vlr: &laz::LazVlr) -> Result<u64, SourceError> {
    vlr.items().iter().try_fold(0_u64, |layers, item| {
        let item_layers = match item.item_type() {
            laz::LazItemType::Point14 => 9,
            laz::LazItemType::RGB14 | laz::LazItemType::WavePacket14 => 1,
            laz::LazItemType::RGBNIR14 => 2,
            laz::LazItemType::Byte14(width) => u64::from(width),
            _ => {
                return Err(SourceError::corrupt(
                    "non-layered LASzip item appears in a layered chunk",
                ));
            }
        };
        layers
            .checked_add(item_layers)
            .ok_or_else(|| SourceError::corrupt("LASzip layer count overflows"))
    })
}

fn coordinate_reference(
    regular: &[RawVlr],
    extended: &[RawVlr],
) -> Result<CoordinateReference, SourceError> {
    let mut wkt_candidates = regular
        .iter()
        .chain(extended)
        .filter(|record| record.user_id == "LASF_Projection" && record.record_id == 2112);
    if let Some(record) = wkt_candidates.next() {
        if wkt_candidates.next().is_some() {
            return Ok(CoordinateReference::Unknown);
        }
        let Ok(wkt) = std::str::from_utf8(&record.data) else {
            return Ok(CoordinateReference::Unknown);
        };
        let wkt = wkt.trim_end_matches('\0').trim();
        if wkt.is_empty() || wkt.len() > MAX_COORDINATE_REFERENCE_WKT_BYTES {
            return Ok(CoordinateReference::Unknown);
        }
        return CoordinateReference::wkt(wkt.to_owned()).map_err(contract_error);
    }

    let mut key_directories = regular
        .iter()
        .chain(extended)
        .filter(|record| record.user_id == "LASF_Projection" && record.record_id == 34735);
    let Some(directory) = key_directories.next() else {
        return Ok(CoordinateReference::Unknown);
    };
    if key_directories.next().is_some() {
        return Ok(CoordinateReference::Unknown);
    }
    Ok(parse_geotiff_profile(&directory.data)
        .map_or(CoordinateReference::Unknown, CoordinateReference::profile))
}

const GEOTIFF_DIRECTORY_VERSION: u16 = 1;
const GEOTIFF_KEY_REVISION: u16 = 1;
const GEOTIFF_MINOR_REVISION: u16 = 0;
const GEOTIFF_MODEL_TYPE_PROJECTED: u16 = 1;
const GT_MODEL_TYPE_KEY: u16 = 1024;
const PROJECTED_CS_TYPE_KEY: u16 = 3072;
const PROJECTED_LINEAR_UNITS_KEY: u16 = 3076;
const VERTICAL_CS_TYPE_KEY: u16 = 4096;
const VERTICAL_UNITS_KEY: u16 = 4099;

fn parse_geotiff_profile(bytes: &[u8]) -> Option<SpatialReferenceProfile> {
    let words = bytes
        .chunks_exact(2)
        .map(|word| u16::from_le_bytes([word[0], word[1]]))
        .collect::<Vec<_>>();
    if !bytes.chunks_exact(2).remainder().is_empty() || words.len() < 4 {
        return None;
    }
    let [version, revision, minor, key_count] = words[..4] else {
        return None;
    };
    if version != GEOTIFF_DIRECTORY_VERSION
        || revision != GEOTIFF_KEY_REVISION
        || minor != GEOTIFF_MINOR_REVISION
    {
        return None;
    }
    let key_count = usize::from(key_count);
    if words.len() != 4_usize.checked_add(key_count.checked_mul(4)?)? {
        return None;
    }

    let mut model_type = None;
    let mut horizontal_epsg = None;
    let mut horizontal_unit = None;
    let mut vertical_epsg = None;
    let mut vertical_unit = None;
    let mut seen_keys = BTreeSet::new();
    for entry in words[4..].chunks_exact(4) {
        let [key, location, count, value] = entry else {
            return None;
        };
        if !seen_keys.insert(*key) || *location != 0 || *count != 1 {
            return None;
        }
        if !matches!(
            *key,
            GT_MODEL_TYPE_KEY
                | PROJECTED_CS_TYPE_KEY
                | PROJECTED_LINEAR_UNITS_KEY
                | VERTICAL_CS_TYPE_KEY
                | VERTICAL_UNITS_KEY
        ) {
            continue;
        }
        let target = match *key {
            GT_MODEL_TYPE_KEY => &mut model_type,
            PROJECTED_CS_TYPE_KEY => &mut horizontal_epsg,
            PROJECTED_LINEAR_UNITS_KEY => &mut horizontal_unit,
            VERTICAL_CS_TYPE_KEY => &mut vertical_epsg,
            VERTICAL_UNITS_KEY => &mut vertical_unit,
            _ => unreachable!(),
        };
        *target = Some(*value);
    }
    if model_type != Some(GEOTIFF_MODEL_TYPE_PROJECTED) {
        return None;
    }
    SpatialReferenceProfile::new(
        u32::from(horizontal_epsg?),
        u32::from(vertical_epsg?),
        SpatialAxes::EastingNorthingElevation,
        linear_unit(horizontal_unit?)?,
        linear_unit(vertical_unit?)?,
        SpatialReferenceProvenance::SourceMetadata,
    )
    .ok()
}

fn linear_unit(value: u16) -> Option<LinearUnit> {
    let epsg_code = u32::from(value);
    [
        LinearUnit::Metre,
        LinearUnit::InternationalFoot,
        LinearUnit::UsSurveyFoot,
    ]
    .into_iter()
    .find(|unit| unit.epsg_code() == epsg_code)
}

fn metadata_records(
    regular: Vec<RawVlr>,
    extended: Vec<RawVlr>,
) -> Result<Vec<MetadataRecord>, SourceError> {
    regular
        .into_iter()
        .map(|record| metadata_record("las.vlr", record))
        .chain(
            extended
                .into_iter()
                .map(|record| metadata_record("las.evlr", record)),
        )
        .collect()
}

fn metadata_record(namespace: &str, record: RawVlr) -> Result<MetadataRecord, SourceError> {
    let RawVlr {
        user_id,
        record_id,
        description,
        data,
    } = record;
    let name = format!("{user_id}:{record_id}:{description}");
    MetadataRecord::new(namespace, name, data).map_err(contract_error)
}

// Keeping the schema declaration together makes its stable IDs and raw-field
// coverage directly reviewable alongside the decoder's exhaustive mapping.
#[allow(clippy::too_many_lines)]
fn attribute_plans(format: Format) -> Result<Vec<AttributePlan>, SourceError> {
    let mut attributes = vec![
        plan(
            1,
            "intensity",
            AttributeDataType::U16,
            AttributeKind::Intensity,
        )?,
        plan(
            2,
            "return_number",
            AttributeDataType::U8,
            AttributeKind::ReturnNumber,
        )?,
        plan(
            3,
            "number_of_returns",
            AttributeDataType::U8,
            AttributeKind::NumberOfReturns,
        )?,
        plan(
            4,
            "scan_direction",
            AttributeDataType::U8,
            AttributeKind::ScanDirection,
        )?,
        plan(
            5,
            "edge_of_flight_line",
            AttributeDataType::U8,
            AttributeKind::EdgeOfFlightLine,
        )?,
        plan(
            6,
            "classification",
            AttributeDataType::U8,
            AttributeKind::Classification,
        )?,
        plan(
            7,
            "synthetic",
            AttributeDataType::U8,
            AttributeKind::Synthetic,
        )?,
        plan(
            8,
            "key_point",
            AttributeDataType::U8,
            AttributeKind::KeyPoint,
        )?,
        plan(
            9,
            "withheld",
            AttributeDataType::U8,
            AttributeKind::Withheld,
        )?,
        plan(10, "overlap", AttributeDataType::U8, AttributeKind::Overlap)?,
        plan(
            13,
            "user_data",
            AttributeDataType::U8,
            AttributeKind::UserData,
        )?,
        plan(
            14,
            "point_source_id",
            AttributeDataType::U16,
            AttributeKind::PointSourceId,
        )?,
    ];
    if format.is_extended {
        attributes.extend([
            plan(
                11,
                "scanner_channel",
                AttributeDataType::U8,
                AttributeKind::ScannerChannel,
            )?,
            plan(
                12,
                "scan_angle",
                AttributeDataType::I16,
                AttributeKind::ScanAngleI16,
            )?,
        ]);
    } else {
        attributes.push(plan(
            12,
            "scan_angle",
            AttributeDataType::I8,
            AttributeKind::ScanAngleI8,
        )?);
    }
    if format.has_gps_time {
        attributes.push(plan(
            15,
            "gps_time",
            AttributeDataType::F64,
            AttributeKind::GpsTime,
        )?);
    }
    if format.has_color {
        attributes.extend([
            plan(16, "red", AttributeDataType::U16, AttributeKind::Red)?,
            plan(17, "green", AttributeDataType::U16, AttributeKind::Green)?,
            plan(18, "blue", AttributeDataType::U16, AttributeKind::Blue)?,
        ]);
    }
    if format.has_waveform {
        attributes.extend([
            plan(
                19,
                "waveform_descriptor",
                AttributeDataType::U8,
                AttributeKind::WaveformDescriptor,
            )?,
            plan(
                20,
                "waveform_offset",
                AttributeDataType::U64,
                AttributeKind::WaveformOffset,
            )?,
            plan(
                21,
                "waveform_size",
                AttributeDataType::U32,
                AttributeKind::WaveformSize,
            )?,
            plan(
                22,
                "waveform_location",
                AttributeDataType::F32,
                AttributeKind::WaveformLocation,
            )?,
            plan(
                23,
                "waveform_dx",
                AttributeDataType::F32,
                AttributeKind::WaveformDx,
            )?,
            plan(
                24,
                "waveform_dy",
                AttributeDataType::F32,
                AttributeKind::WaveformDy,
            )?,
            plan(
                25,
                "waveform_dz",
                AttributeDataType::F32,
                AttributeKind::WaveformDz,
            )?,
        ]);
    }
    if format.has_nir {
        attributes.push(plan(26, "nir", AttributeDataType::U16, AttributeKind::Nir)?);
    }
    if format.extra_bytes != 0 {
        let width = u32::from(format.extra_bytes);
        attributes.push(plan(
            4096,
            "extra_bytes",
            AttributeDataType::fixed_bytes(width).map_err(contract_error)?,
            AttributeKind::ExtraBytes(width),
        )?);
    }
    Ok(attributes)
}

fn plan(
    id: u32,
    name: &str,
    data_type: AttributeDataType,
    kind: AttributeKind,
) -> Result<AttributePlan, SourceError> {
    let id = AttributeId::new(id).map_err(contract_error)?;
    let definition = AttributeDefinition::new(id, name, data_type).map_err(contract_error)?;
    Ok(AttributePlan { definition, kind })
}

fn snapshot_and_hash(
    source: &mut File,
    snapshot: &mut File,
    file_len: u64,
    reporter: &OperationReporter,
) -> Result<ContentHash, SourceError> {
    source.seek(SeekFrom::Start(0)).map_err(classify_io)?;
    snapshot.set_len(0).map_err(classify_io)?;
    snapshot.seek(SeekFrom::Start(0)).map_err(classify_io)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; HASH_BUFFER_BYTES];
    let mut completed = 0_u64;
    loop {
        reporter.check_cancelled()?;
        let read = source.read(&mut buffer).map_err(classify_io)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        snapshot.write_all(&buffer[..read]).map_err(classify_io)?;
        completed = completed
            .checked_add(
                u64::try_from(read)
                    .map_err(|_| SourceError::adapter("hash byte count overflow"))?,
            )
            .ok_or_else(|| SourceError::adapter("hash byte count overflow"))?;
    }
    if completed != file_len {
        return Err(SourceError::changed(
            "the file length changed while hashing",
        ));
    }
    snapshot.flush().map_err(classify_io)?;
    snapshot.seek(SeekFrom::Start(0)).map_err(classify_io)?;
    Ok(ContentHash::new(*hasher.finalize().as_bytes()))
}

pub(crate) fn classify_io(error: std::io::Error) -> SourceError {
    let kind = error.kind();
    let reason = error.to_string();
    drop(error);
    if kind == std::io::ErrorKind::NotFound {
        SourceError::SourceMissing {
            reason: reason.into(),
        }
    } else if kind == std::io::ErrorKind::UnexpectedEof {
        SourceError::corrupt(reason)
    } else {
        SourceError::adapter(reason)
    }
}

fn contract_error(error: impl std::fmt::Display) -> SourceError {
    SourceError::corrupt(error.to_string())
}

fn slice_error(_: std::array::TryFromSliceError) -> SourceError {
    SourceError::corrupt("truncated LAS fixed-width field")
}

fn las_text(bytes: &[u8], field: &str) -> Result<String, SourceError> {
    let text = std::str::from_utf8(bytes).map_err(|error| {
        SourceError::corrupt(format!("{field} contains invalid UTF-8: {error}"))
    })?;
    Ok(text.trim_end_matches(['\0', ' ']).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_budget_is_shared_across_file_sections() {
        let mut budget = MetadataReadBudget::new();
        budget
            .reserve_records(MAX_METADATA_RECORDS as u64 - 1)
            .unwrap();
        budget.reserve_records(1).unwrap();
        assert!(matches!(
            budget.reserve_records(1),
            Err(SourceError::CorruptSource { .. })
        ));

        let mut budget = MetadataReadBudget::new();
        budget
            .reserve_payload(MAX_SOURCE_METADATA_PAYLOAD_BYTES as u64 - 1)
            .unwrap();
        budget.reserve_payload(1).unwrap();
        assert!(matches!(
            budget.reserve_payload(1),
            Err(SourceError::CorruptSource { .. })
        ));
    }

    #[test]
    fn codec_seek_mode_requires_a_fixed_chunked_organization() {
        let fixed = laz::LazVlrBuilder::default()
            .with_point_format(0, 0)
            .unwrap()
            .with_fixed_chunk_size(123)
            .build();
        assert_eq!(
            laz_seek_mode(2, &fixed),
            LazSeekMode::FixedChunks {
                points_per_chunk: 123
            }
        );
        assert_eq!(laz_seek_mode(1, &fixed), LazSeekMode::Sequential);

        let variable = laz::LazVlrBuilder::default()
            .with_point_format(0, 0)
            .unwrap()
            .with_variable_chunk_size()
            .build();
        assert_eq!(laz_seek_mode(2, &variable), LazSeekMode::Sequential);
        assert_eq!(laz_seek_mode(3, &variable), LazSeekMode::Sequential);
    }

    #[test]
    fn fixed_layered_chunk_counts_must_match_vlr_boundaries() {
        validate_fixed_chunk_table(2, 4, 2).unwrap();
        validate_fixed_layered_chunk_count(0, 2, 2, 4, 2).unwrap();
        validate_fixed_layered_chunk_count(1, 2, 2, 4, 2).unwrap();

        assert!(validate_fixed_layered_chunk_count(0, 2, 1, 4, 2).is_err());
        assert!(validate_fixed_layered_chunk_count(1, 2, 3, 4, 2).is_err());
        assert!(validate_fixed_chunk_table(3, 4, 2).is_err());
    }
}
