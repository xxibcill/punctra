use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek, SeekFrom};
use std::sync::Arc;

use foundation_runtime::{OperationReporter, ProgressPhase, ProgressSnapshot};
use las::raw;
use laz::LasZipDecompressor;
use point_contracts::{
    AttributeColumn, AttributeColumns, AttributeId, AttributeValues, PointBatch,
    QuantizedPositions, SourceId, WorldBounds,
};
use point_source::adapter::{AdapterRead, AdapterReadRequest, ReadAdapter};
use point_source::{ReadBudget, ReadLimit, SourceError, SourceSpan};

use crate::format::{
    AttributeKind, AttributePlan, Compression, FileWitness, LasLayout, LazSeekMode,
    SourceFileWitness, classify_io,
};

const FIXED_DECODER_WORKING_BYTES: u64 = 64 * 1024;
const VERIFICATION_WORKING_BYTES: u64 = 64 * 1024 * 1024;
const VERIFICATION_ROWS: u64 = 16_384;
const READ_CANCELLATION_WORKING_BYTES: u64 = 8 * 1_024 * 1_024;

#[cfg(not(test))]
type DecoderInput = File;

#[cfg(test)]
struct DecoderInput {
    file: File,
    read_bytes: u64,
}

#[cfg(not(test))]
fn decoder_input(file: &File) -> std::io::Result<DecoderInput> {
    file.try_clone()
}

#[cfg(test)]
fn decoder_input(file: &File) -> std::io::Result<DecoderInput> {
    Ok(DecoderInput {
        file: file.try_clone()?,
        read_bytes: 0,
    })
}

#[cfg(test)]
impl Read for DecoderInput {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.file.read(buffer)?;
        self.read_bytes = self
            .read_bytes
            .saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        Ok(read)
    }
}

#[cfg(test)]
impl Seek for DecoderInput {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(position)
    }
}

pub(crate) struct LasReadAdapter {
    file: Arc<File>,
    source_witness: SourceFileWitness,
    layout: Arc<LasLayout>,
}

impl LasReadAdapter {
    pub(crate) fn new(
        file: Arc<File>,
        source_witness: SourceFileWitness,
        layout: Arc<LasLayout>,
    ) -> Self {
        Self {
            file,
            source_witness,
            layout,
        }
    }
}

impl ReadAdapter for LasReadAdapter {
    fn start_read(
        &self,
        request: AdapterReadRequest,
        source: SourceId,
        reporter: OperationReporter,
    ) -> Result<Box<dyn AdapterRead>, SourceError> {
        self.source_witness.ensure_unchanged()?;
        let selected = selected_attributes(&self.layout, request.attributes().explicit())?;
        let max_rows = batch_rows(
            &self.layout,
            request.budget(),
            request.max_output_batch_points(),
        )?;
        let decoder = RecordDecoder::new(&self.file, &self.layout)?;
        let spans = request.spans().to_vec();
        let next_ordinal = spans.first().map_or(0, |span| span.first_ordinal());
        Ok(Box::new(LasRead {
            source_witness: self.source_witness.clone(),
            layout: Arc::clone(&self.layout),
            decoder,
            reporter,
            source,
            spans,
            selected,
            max_rows,
            raw: Vec::new(),
            span_index: 0,
            next_ordinal,
            terminal: false,
        }))
    }
}

fn selected_attributes(
    layout: &LasLayout,
    selected: Option<&[AttributeId]>,
) -> Result<Vec<AttributePlan>, SourceError> {
    let selected = selected.expect("point-source resolves every adapter Attribute selection");
    selected
        .iter()
        .map(|id| {
            layout
                .attributes
                .iter()
                .find(|attribute| attribute.definition.id() == *id)
                .cloned()
                .ok_or_else(|| {
                    SourceError::unsupported_schema(format!(
                        "Source does not contain requested Attribute {id:?}"
                    ))
                })
        })
        .collect()
}

fn batch_rows(
    layout: &LasLayout,
    budget: ReadBudget,
    max_output_batch_points: u64,
) -> Result<u64, SourceError> {
    let fixed = FIXED_DECODER_WORKING_BYTES
        .checked_add(layout.compression.decoder_bytes())
        .ok_or(SourceError::ResourceLimit {
            limit: ReadLimit::AdapterWorkingBytes,
            required: u64::MAX,
            allowed: budget.max_adapter_working_bytes(),
        })?;
    let record_len = u64::try_from(layout.record_len)
        .map_err(|_| SourceError::adapter("LAS record width does not fit u64"))?;
    let required = fixed
        .checked_add(record_len)
        .ok_or(SourceError::ResourceLimit {
            limit: ReadLimit::AdapterWorkingBytes,
            required: u64::MAX,
            allowed: budget.max_adapter_working_bytes(),
        })?;
    if required > budget.max_adapter_working_bytes() {
        return Err(SourceError::ResourceLimit {
            limit: ReadLimit::AdapterWorkingBytes,
            required,
            allowed: budget.max_adapter_working_bytes(),
        });
    }
    let interruptible_working_bytes = budget
        .max_adapter_working_bytes()
        .min(READ_CANCELLATION_WORKING_BYTES)
        .max(required);
    let rows_by_working = (interruptible_working_bytes - fixed) / record_len;
    Ok(max_output_batch_points.min(rows_by_working))
}

struct LasRead {
    source_witness: SourceFileWitness,
    layout: Arc<LasLayout>,
    decoder: RecordDecoder,
    reporter: OperationReporter,
    source: SourceId,
    spans: Vec<SourceSpan>,
    selected: Vec<AttributePlan>,
    max_rows: u64,
    raw: Vec<u8>,
    span_index: usize,
    next_ordinal: u64,
    terminal: bool,
}

impl AdapterRead for LasRead {
    fn next(&mut self) -> Result<Option<PointBatch>, SourceError> {
        if self.terminal {
            return Ok(None);
        }
        if let Err(error) = self.reporter.check_cancelled().map_err(SourceError::from) {
            return self.fail(error);
        }
        if let Err(error) = self.source_witness.ensure_unchanged() {
            return self.fail(error);
        }
        let Some(span) = self.spans.get(self.span_index).copied() else {
            self.terminal = true;
            return Ok(None);
        };

        if let Err(error) = self.decoder.move_to(
            self.next_ordinal,
            self.max_rows,
            &mut self.raw,
            &self.reporter,
        ) {
            let error = match error {
                DecoderMoveError::Cancelled => SourceError::Cancelled,
                DecoderMoveError::Decode(message) => self.decode_failure(&message),
            };
            return self.fail(error);
        }
        let point_count = (span.end_ordinal() - self.next_ordinal).min(self.max_rows);
        if let Err(error) = self.decoder.read_records(point_count, &mut self.raw) {
            let error = self.decode_failure(&error);
            return self.fail(error);
        }
        let batch = match canonical_batch(
            &self.layout,
            self.source,
            self.next_ordinal,
            &self.raw,
            &self.selected,
        ) {
            Ok(batch) => batch,
            Err(error) => return self.fail(error),
        };
        if let Err(error) = self.reporter.check_cancelled().map_err(SourceError::from) {
            return self.fail(error);
        }
        if let Err(error) = self.source_witness.ensure_unchanged() {
            return self.fail(error);
        }
        self.advance(point_count, span.end_ordinal());
        Ok(Some(batch))
    }
}

impl LasRead {
    fn advance(&mut self, point_count: u64, span_end: u64) {
        self.next_ordinal += point_count;
        if self.next_ordinal == span_end {
            self.span_index += 1;
            if let Some(next) = self.spans.get(self.span_index) {
                self.next_ordinal = next.first_ordinal();
            }
        }
    }

    fn decode_failure(&self, message: &str) -> SourceError {
        self.source_witness
            .ensure_unchanged()
            .err()
            .unwrap_or_else(|| {
                SourceError::corrupt(format!(
                    "could not decode LAS Point at or after ordinal {}: {message}",
                    self.next_ordinal
                ))
            })
    }

    fn fail<T>(&mut self, error: SourceError) -> Result<T, SourceError> {
        self.terminal = true;
        Err(error)
    }
}

type LazDecoder = LasZipDecompressor<'static, BufReader<DecoderInput>>;

enum RecordDecoder {
    Las {
        reader: BufReader<DecoderInput>,
        point_offset: u64,
        record_len: u64,
        current: u64,
    },
    Laz {
        decoder: LazDecoder,
        record_len: u64,
        current: u64,
        seek_mode: LazSeekMode,
    },
}

#[derive(Debug)]
enum DecoderMoveError {
    Cancelled,
    Decode(String),
}

impl RecordDecoder {
    fn new(file: &File, layout: &LasLayout) -> Result<Self, SourceError> {
        let mut reader = BufReader::new(decoder_input(file).map_err(classify_io)?);
        reader
            .seek(SeekFrom::Start(layout.point_offset))
            .map_err(classify_io)?;
        let record_len = u64::try_from(layout.record_len)
            .map_err(|_| SourceError::adapter("LAS record width does not fit u64"))?;
        match &layout.compression {
            Compression::Las => Ok(Self::Las {
                reader,
                point_offset: layout.point_offset,
                record_len,
                current: 0,
            }),
            Compression::Laz { vlr, seek_mode, .. } => {
                let decoder = LasZipDecompressor::new(reader, vlr.clone())
                    .map_err(|error| SourceError::corrupt(error.to_string()))?;
                Ok(Self::Laz {
                    decoder,
                    record_len,
                    current: 0,
                    seek_mode: *seek_mode,
                })
            }
        }
    }

    fn move_to(
        &mut self,
        target: u64,
        quantum: u64,
        buffer: &mut Vec<u8>,
        reporter: &OperationReporter,
    ) -> Result<(), DecoderMoveError> {
        check_move_cancelled(reporter)?;
        match self {
            Self::Las {
                reader,
                point_offset,
                record_len,
                current,
            } => move_las(reader, *point_offset, *record_len, current, target)
                .map_err(DecoderMoveError::Decode)?,
            Self::Laz {
                decoder,
                record_len,
                current,
                seek_mode,
            } => move_laz(
                decoder,
                *record_len,
                current,
                *seek_mode,
                target,
                quantum,
                buffer,
                reporter,
            )?,
        }
        check_move_cancelled(reporter)
    }

    fn read_records(&mut self, point_count: u64, buffer: &mut Vec<u8>) -> Result<(), String> {
        match self {
            Self::Las {
                reader,
                record_len,
                current,
                ..
            } => {
                resize_record_buffer(buffer, point_count, *record_len)?;
                reader
                    .read_exact(buffer)
                    .map_err(|error| error.to_string())?;
                *current += point_count;
                Ok(())
            }
            Self::Laz {
                decoder,
                record_len,
                current,
                ..
            } => read_laz_records(decoder, *record_len, current, point_count, buffer),
        }
    }

    #[cfg(test)]
    fn read_bytes(&self) -> u64 {
        match self {
            Self::Las { reader, .. } => reader.get_ref().read_bytes,
            Self::Laz { decoder, .. } => decoder.get().get_ref().read_bytes,
        }
    }

    #[cfg(test)]
    fn force_sequential_laz_seek(&mut self) {
        let Self::Laz { seek_mode, .. } = self else {
            panic!("test expected a LAZ decoder");
        };
        *seek_mode = LazSeekMode::Sequential;
    }
}

fn move_las(
    reader: &mut BufReader<DecoderInput>,
    point_offset: u64,
    record_len: u64,
    current: &mut u64,
    target: u64,
) -> Result<(), String> {
    let offset = target
        .checked_mul(record_len)
        .and_then(|bytes| point_offset.checked_add(bytes))
        .ok_or_else(|| "LAS point seek offset overflow".to_owned())?;
    reader
        .seek(SeekFrom::Start(offset))
        .map_err(|error| error.to_string())?;
    *current = target;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn move_laz(
    decoder: &mut LazDecoder,
    record_len: u64,
    current: &mut u64,
    seek_mode: LazSeekMode,
    target: u64,
    quantum: u64,
    buffer: &mut Vec<u8>,
    reporter: &OperationReporter,
) -> Result<(), DecoderMoveError> {
    if target < *current {
        return Err(DecoderMoveError::Decode(
            "LAZ decoder cannot move backward within one sorted read".to_owned(),
        ));
    }
    if should_use_chunk_seek(seek_mode, *current, target) {
        decoder.seek(target).map_err(|error| {
            DecoderMoveError::Decode(format!(
                "could not seek the LAZ decoder to ordinal {target}: {error}"
            ))
        })?;
        *current = target;
        return Ok(());
    }
    replay_laz_gap(
        decoder, record_len, current, target, quantum, buffer, reporter,
    )
}

fn should_use_chunk_seek(mode: LazSeekMode, current: u64, target: u64) -> bool {
    match mode {
        LazSeekMode::FixedChunks { points_per_chunk } => {
            points_per_chunk != 0 && current / points_per_chunk != target / points_per_chunk
        }
        LazSeekMode::Sequential => false,
    }
}

fn replay_laz_gap(
    decoder: &mut LazDecoder,
    record_len: u64,
    current: &mut u64,
    target: u64,
    quantum: u64,
    buffer: &mut Vec<u8>,
    reporter: &OperationReporter,
) -> Result<(), DecoderMoveError> {
    while *current < target {
        check_move_cancelled(reporter)?;
        let count = (target - *current).min(quantum);
        read_laz_records(decoder, record_len, current, count, buffer)
            .map_err(DecoderMoveError::Decode)?;
    }
    Ok(())
}

fn read_laz_records(
    decoder: &mut LazDecoder,
    record_len: u64,
    current: &mut u64,
    point_count: u64,
    buffer: &mut Vec<u8>,
) -> Result<(), String> {
    resize_record_buffer(buffer, point_count, record_len)?;
    decoder
        .decompress_many(buffer)
        .map_err(|error| error.to_string())?;
    *current += point_count;
    Ok(())
}

fn resize_record_buffer(
    buffer: &mut Vec<u8>,
    point_count: u64,
    record_len: u64,
) -> Result<(), String> {
    let bytes = point_count
        .checked_mul(record_len)
        .ok_or_else(|| "LAS decode buffer length overflow".to_owned())?;
    let bytes = usize::try_from(bytes)
        .map_err(|_| "LAS decode buffer does not fit the host address space".to_owned())?;
    buffer.resize(bytes, 0);
    Ok(())
}

fn check_move_cancelled(reporter: &OperationReporter) -> Result<(), DecoderMoveError> {
    reporter
        .check_cancelled()
        .map_err(|_| DecoderMoveError::Cancelled)
}

pub(crate) fn scan_bounds(
    file: &File,
    witness: &FileWitness,
    layout: &LasLayout,
    reporter: &OperationReporter,
) -> Result<Option<WorldBounds>, SourceError> {
    let fixed = FIXED_DECODER_WORKING_BYTES
        .checked_add(layout.compression.decoder_bytes())
        .ok_or(SourceError::ResourceLimit {
            limit: ReadLimit::VerificationWorkingBytes,
            required: u64::MAX,
            allowed: VERIFICATION_WORKING_BYTES,
        })?;
    let record_len = u64::try_from(layout.record_len)
        .map_err(|_| SourceError::corrupt("LAS record width does not fit u64"))?;
    let required = fixed
        .checked_add(record_len)
        .ok_or(SourceError::ResourceLimit {
            limit: ReadLimit::VerificationWorkingBytes,
            required: u64::MAX,
            allowed: VERIFICATION_WORKING_BYTES,
        })?;
    if required > VERIFICATION_WORKING_BYTES {
        return Err(SourceError::ResourceLimit {
            limit: ReadLimit::VerificationWorkingBytes,
            required,
            allowed: VERIFICATION_WORKING_BYTES,
        });
    }
    let rows = ((VERIFICATION_WORKING_BYTES - fixed) / record_len).clamp(1, VERIFICATION_ROWS);
    let mut decoder = RecordDecoder::new(file, layout)?;
    let mut raw_bytes = Vec::new();
    let mut completed = 0_u64;
    let mut tick_min = [i64::MAX; 3];
    let mut tick_max = [i64::MIN; 3];

    while completed < layout.point_count {
        reporter.check_cancelled()?;
        witness.ensure_file(file)?;
        let count = (layout.point_count - completed).min(rows);
        decoder
            .read_records(count, &mut raw_bytes)
            .map_err(|error| {
                SourceError::corrupt(format!(
                    "could not decode LAS Point at or after ordinal {completed}: {error}"
                ))
            })?;
        validate_and_grow_bounds(layout, &raw_bytes, &mut tick_min, &mut tick_max)?;
        completed += count;
        reporter.report_progress(ProgressSnapshot::new(
            ProgressPhase::RUNNING,
            completed,
            Some(layout.point_count),
        )?)?;
    }
    witness.ensure_file(file)?;
    let bounds = if layout.point_count == 0 {
        None
    } else {
        Some(
            WorldBounds::new(
                layout.transform.world_f64(tick_min),
                layout.transform.world_f64(tick_max),
            )
            .map_err(|error| SourceError::corrupt(error.to_string()))?,
        )
    };
    if !declared_bounds_contain(layout.declared_bounds, bounds) {
        return Err(SourceError::corrupt(
            "decoded Point bounds extend beyond the LAS public header",
        ));
    }
    Ok(bounds)
}

fn declared_bounds_contain(declared: Option<WorldBounds>, decoded: Option<WorldBounds>) -> bool {
    match (declared, decoded) {
        (None, None) => true,
        (Some(declared), Some(decoded)) => {
            let declared_min = declared.min();
            let declared_max = declared.max();
            let decoded_min = decoded.min();
            let decoded_max = decoded.max();
            (0..3).all(|axis| {
                declared_min[axis] <= decoded_min[axis] && decoded_max[axis] <= declared_max[axis]
            })
        }
        _ => false,
    }
}

fn validate_and_grow_bounds(
    layout: &LasLayout,
    raw_bytes: &[u8],
    tick_min: &mut [i64; 3],
    tick_max: &mut [i64; 3],
) -> Result<(), SourceError> {
    let mut cursor = Cursor::new(raw_bytes);
    for record in raw_bytes.chunks_exact(layout.record_len) {
        raw::Point::read_from(&mut cursor, &layout.format)
            .map_err(|error| SourceError::corrupt(error.to_string()))?;
        let ticks = position_ticks(record)?;
        for axis in 0..3 {
            tick_min[axis] = tick_min[axis].min(ticks[axis]);
            tick_max[axis] = tick_max[axis].max(ticks[axis]);
        }
    }
    Ok(())
}

fn canonical_batch(
    layout: &LasLayout,
    source: SourceId,
    first_ordinal: u64,
    raw_bytes: &[u8],
    selected: &[AttributePlan],
) -> Result<PointBatch, SourceError> {
    if !raw_bytes.len().is_multiple_of(layout.record_len) {
        return Err(SourceError::corrupt(
            "decoded LAS record block is not row-aligned",
        ));
    }
    let rows = raw_bytes.len() / layout.record_len;
    let ticks = raw_bytes
        .chunks_exact(layout.record_len)
        .map(position_ticks)
        .collect::<Result<Vec<_>, SourceError>>()?;
    let positions = QuantizedPositions::new(layout.transform, ticks)
        .map_err(|error| SourceError::adapter(error.to_string()))?;
    let offsets = OptionalOffsets::for_layout(layout)?;
    let columns = selected
        .iter()
        .map(|plan| attribute_column(plan, layout, &offsets, raw_bytes))
        .collect::<Result<Vec<_>, SourceError>>()?;
    let attributes = AttributeColumns::new(columns, rows)
        .map_err(|error| SourceError::adapter(error.to_string()))?;
    PointBatch::new(source, first_ordinal, positions, attributes)
        .map_err(|error| SourceError::adapter(error.to_string()))
}

fn position_ticks(record: &[u8]) -> Result<[i64; 3], SourceError> {
    Ok([
        i64::from(read_i32(record, 0)?),
        i64::from(read_i32(record, 4)?),
        i64::from(read_i32(record, 8)?),
    ])
}

struct OptionalOffsets {
    gps: Option<usize>,
    color: Option<usize>,
    nir: Option<usize>,
    waveform: Option<usize>,
    extra: usize,
}

impl OptionalOffsets {
    fn for_layout(layout: &LasLayout) -> Result<Self, SourceError> {
        let format = layout.format;
        let mut cursor: usize = if format.is_extended { 30 } else { 20 };
        let gps = if format.is_extended {
            Some(22)
        } else if format.has_gps_time {
            let offset = cursor;
            cursor += 8;
            Some(offset)
        } else {
            None
        };
        let color = if format.has_color {
            let offset = cursor;
            cursor += 6;
            Some(offset)
        } else {
            None
        };
        let nir = if format.has_nir {
            let offset = cursor;
            cursor += 2;
            Some(offset)
        } else {
            None
        };
        let waveform = if format.has_waveform {
            let offset = cursor;
            cursor += 29;
            Some(offset)
        } else {
            None
        };
        let extra = cursor;
        cursor = cursor
            .checked_add(usize::from(format.extra_bytes))
            .ok_or_else(|| SourceError::corrupt("LAS record layout overflows"))?;
        if cursor != layout.record_len {
            return Err(SourceError::corrupt(
                "LAS record layout does not match the declared record length",
            ));
        }
        Ok(Self {
            gps,
            color,
            nir,
            waveform,
            extra,
        })
    }
}

// The exhaustive match is intentionally kept together so the raw LAS-to-canonical
// field mapping can be audited against the point-record layouts in one place.
#[allow(clippy::too_many_lines)]
fn attribute_column(
    plan: &AttributePlan,
    layout: &LasLayout,
    offsets: &OptionalOffsets,
    raw_bytes: &[u8],
) -> Result<AttributeColumn, SourceError> {
    let records = || raw_bytes.chunks_exact(layout.record_len);
    let values = match plan.kind {
        AttributeKind::Intensity => AttributeValues::u16(
            records()
                .map(|row| read_u16(row, 12))
                .collect::<Result<_, _>>()?,
        ),
        AttributeKind::ReturnNumber => AttributeValues::u8(
            records()
                .map(|row| {
                    Ok(if layout.format.is_extended {
                        row[14] & 15
                    } else {
                        row[14] & 7
                    })
                })
                .collect::<Result<_, SourceError>>()?,
        ),
        AttributeKind::NumberOfReturns => AttributeValues::u8(
            records()
                .map(|row| {
                    Ok(if layout.format.is_extended {
                        row[14] >> 4
                    } else {
                        (row[14] >> 3) & 7
                    })
                })
                .collect::<Result<_, SourceError>>()?,
        ),
        AttributeKind::ScanDirection => AttributeValues::u8(
            records()
                .map(|row| {
                    let flags = if layout.format.is_extended {
                        row[15]
                    } else {
                        row[14]
                    };
                    Ok((flags >> 6) & 1)
                })
                .collect::<Result<_, SourceError>>()?,
        ),
        AttributeKind::EdgeOfFlightLine => AttributeValues::u8(
            records()
                .map(|row| {
                    let flags = if layout.format.is_extended {
                        row[15]
                    } else {
                        row[14]
                    };
                    Ok((flags >> 7) & 1)
                })
                .collect::<Result<_, SourceError>>()?,
        ),
        AttributeKind::Classification => AttributeValues::u8(
            records()
                .map(|row| {
                    Ok(if layout.format.is_extended {
                        row[16]
                    } else {
                        row[15] & 31
                    })
                })
                .collect::<Result<_, SourceError>>()?,
        ),
        AttributeKind::Synthetic => AttributeValues::u8(
            records()
                .map(|row| {
                    Ok(if layout.format.is_extended {
                        row[15] & 1
                    } else {
                        (row[15] >> 5) & 1
                    })
                })
                .collect::<Result<_, SourceError>>()?,
        ),
        AttributeKind::KeyPoint => AttributeValues::u8(
            records()
                .map(|row| {
                    Ok(if layout.format.is_extended {
                        (row[15] >> 1) & 1
                    } else {
                        (row[15] >> 6) & 1
                    })
                })
                .collect::<Result<_, SourceError>>()?,
        ),
        AttributeKind::Withheld => AttributeValues::u8(
            records()
                .map(|row| {
                    Ok(if layout.format.is_extended {
                        (row[15] >> 2) & 1
                    } else {
                        (row[15] >> 7) & 1
                    })
                })
                .collect::<Result<_, SourceError>>()?,
        ),
        AttributeKind::Overlap => AttributeValues::u8(
            records()
                .map(|row| {
                    Ok(if layout.format.is_extended {
                        (row[15] >> 3) & 1
                    } else {
                        u8::from(row[15] & 31 == 12)
                    })
                })
                .collect::<Result<_, SourceError>>()?,
        ),
        AttributeKind::ScannerChannel => AttributeValues::u8(
            records()
                .map(|row| Ok((row[15] >> 4) & 3))
                .collect::<Result<_, SourceError>>()?,
        ),
        AttributeKind::ScanAngleI8 => AttributeValues::i8(
            records()
                .map(|row| Ok(i8::from_le_bytes([row[16]])))
                .collect::<Result<_, SourceError>>()?,
        ),
        AttributeKind::ScanAngleI16 => AttributeValues::i16(
            records()
                .map(|row| read_i16(row, 18))
                .collect::<Result<_, _>>()?,
        ),
        AttributeKind::UserData => AttributeValues::u8(
            records()
                .map(|row| Ok(row[17]))
                .collect::<Result<_, SourceError>>()?,
        ),
        AttributeKind::PointSourceId => AttributeValues::u16(
            records()
                .map(|row| read_u16(row, if layout.format.is_extended { 20 } else { 18 }))
                .collect::<Result<_, _>>()?,
        ),
        AttributeKind::GpsTime => AttributeValues::f64(
            records()
                .map(|row| read_f64(row, required_offset(offsets.gps, "GPS time")?))
                .collect::<Result<_, _>>()?,
        ),
        AttributeKind::Red => AttributeValues::u16(
            records()
                .map(|row| read_u16(row, required_offset(offsets.color, "color")?))
                .collect::<Result<_, _>>()?,
        ),
        AttributeKind::Green => AttributeValues::u16(
            records()
                .map(|row| read_u16(row, required_offset(offsets.color, "color")? + 2))
                .collect::<Result<_, _>>()?,
        ),
        AttributeKind::Blue => AttributeValues::u16(
            records()
                .map(|row| read_u16(row, required_offset(offsets.color, "color")? + 4))
                .collect::<Result<_, _>>()?,
        ),
        AttributeKind::WaveformDescriptor => AttributeValues::u8(
            records()
                .map(|row| Ok(row[required_offset(offsets.waveform, "waveform")?]))
                .collect::<Result<_, SourceError>>()?,
        ),
        AttributeKind::WaveformOffset => AttributeValues::u64(
            records()
                .map(|row| read_u64(row, required_offset(offsets.waveform, "waveform")? + 1))
                .collect::<Result<_, _>>()?,
        ),
        AttributeKind::WaveformSize => AttributeValues::u32(
            records()
                .map(|row| read_u32(row, required_offset(offsets.waveform, "waveform")? + 9))
                .collect::<Result<_, _>>()?,
        ),
        AttributeKind::WaveformLocation => AttributeValues::f32(
            records()
                .map(|row| read_f32(row, required_offset(offsets.waveform, "waveform")? + 13))
                .collect::<Result<_, _>>()?,
        ),
        AttributeKind::WaveformDx => AttributeValues::f32(
            records()
                .map(|row| read_f32(row, required_offset(offsets.waveform, "waveform")? + 17))
                .collect::<Result<_, _>>()?,
        ),
        AttributeKind::WaveformDy => AttributeValues::f32(
            records()
                .map(|row| read_f32(row, required_offset(offsets.waveform, "waveform")? + 21))
                .collect::<Result<_, _>>()?,
        ),
        AttributeKind::WaveformDz => AttributeValues::f32(
            records()
                .map(|row| read_f32(row, required_offset(offsets.waveform, "waveform")? + 25))
                .collect::<Result<_, _>>()?,
        ),
        AttributeKind::Nir => AttributeValues::u16(
            records()
                .map(|row| read_u16(row, required_offset(offsets.nir, "NIR")?))
                .collect::<Result<_, _>>()?,
        ),
        AttributeKind::ExtraBytes(width) => {
            let width_usize = usize::try_from(width)
                .map_err(|_| SourceError::adapter("Extra Bytes width does not fit usize"))?;
            let mut payload =
                Vec::with_capacity(records().len().checked_mul(width_usize).ok_or_else(|| {
                    SourceError::adapter("Extra Bytes batch allocation overflows")
                })?);
            for row in records() {
                payload.extend_from_slice(slice(row, offsets.extra, width_usize)?);
            }
            AttributeValues::fixed_bytes(width, payload)
                .map_err(|error| SourceError::adapter(error.to_string()))?
        }
    };
    AttributeColumn::new(plan.definition.clone(), values)
        .map_err(|error| SourceError::adapter(error.to_string()))
}

fn required_offset(offset: Option<usize>, field: &str) -> Result<usize, SourceError> {
    offset.ok_or_else(|| SourceError::corrupt(format!("LAS layout omits requested {field}")))
}

fn slice(record: &[u8], offset: usize, len: usize) -> Result<&[u8], SourceError> {
    record
        .get(offset..offset.saturating_add(len))
        .ok_or_else(|| SourceError::corrupt("truncated LAS point-record field"))
}

fn read_i16(record: &[u8], offset: usize) -> Result<i16, SourceError> {
    Ok(i16::from_le_bytes(
        slice(record, offset, 2)?.try_into().map_err(slice_error)?,
    ))
}

fn read_u16(record: &[u8], offset: usize) -> Result<u16, SourceError> {
    Ok(u16::from_le_bytes(
        slice(record, offset, 2)?.try_into().map_err(slice_error)?,
    ))
}

fn read_i32(record: &[u8], offset: usize) -> Result<i32, SourceError> {
    Ok(i32::from_le_bytes(
        slice(record, offset, 4)?.try_into().map_err(slice_error)?,
    ))
}

fn read_u32(record: &[u8], offset: usize) -> Result<u32, SourceError> {
    Ok(u32::from_le_bytes(
        slice(record, offset, 4)?.try_into().map_err(slice_error)?,
    ))
}

fn read_u64(record: &[u8], offset: usize) -> Result<u64, SourceError> {
    Ok(u64::from_le_bytes(
        slice(record, offset, 8)?.try_into().map_err(slice_error)?,
    ))
}

fn read_f32(record: &[u8], offset: usize) -> Result<f32, SourceError> {
    read_u32(record, offset).map(f32::from_bits)
}

fn read_f64(record: &[u8], offset: usize) -> Result<f64, SourceError> {
    read_u64(record, offset).map(f64::from_bits)
}

fn slice_error(_: std::array::TryFromSliceError) -> SourceError {
    SourceError::corrupt("truncated LAS fixed-width point field")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use foundation_runtime::OperationControl;
    use las::point::{Classification, Format, ScanDirection};
    use las::raw::point::Waveform;
    use las::{Builder, Color, Point, Transform, Vector, Writer};
    use point_source::adapter::FullVerification;

    use super::{DecoderMoveError, RecordDecoder, should_use_chunk_seek};
    use crate::format::{Compression, LazSeekMode, verify_file};

    const FIXTURE_POINTS: u64 = 50_003;
    const LATER_CHUNK_ORDINAL: u64 = 50_001;
    const MOVE_QUANTUM: u64 = 257;
    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn chunk_seek_is_used_only_across_validated_fixed_chunk_boundaries() {
        let fixed = LazSeekMode::FixedChunks {
            points_per_chunk: 50_000,
        };
        assert!(!should_use_chunk_seek(fixed, 0, 49_999));
        assert!(!should_use_chunk_seek(fixed, 50_000, 50_001));
        assert!(should_use_chunk_seek(fixed, 49_999, 50_001));
        assert!(!should_use_chunk_seek(LazSeekMode::Sequential, 0, 50_001));
    }

    #[test]
    fn fixed_chunk_seek_avoids_prior_chunk_io_and_preserves_exact_records() {
        let directory = FixtureDirectory::new();
        for point_format in [5_u8, 8] {
            let path = directory
                .path()
                .join(format!("seek-format-{point_format}.laz"));
            write_seek_fixture(&path, point_format);
            assert_efficient_exact_seek(&path, point_format);
        }
    }

    fn assert_efficient_exact_seek(path: &Path, point_format: u8) {
        let verification = OperationControl::new();
        let verified =
            verify_file(path, FullVerification::Identify, &verification.reporter()).unwrap();
        assert!(matches!(
            &verified.layout.compression,
            Compression::Laz {
                seek_mode: LazSeekMode::FixedChunks {
                    points_per_chunk: 50_000
                },
                ..
            }
        ));

        let movement = OperationControl::new();
        let reporter = movement.reporter();
        let mut sequential = RecordDecoder::new(&verified.file, &verified.layout).unwrap();
        sequential.force_sequential_laz_seek();
        let sequential_start_bytes = sequential.read_bytes();
        let mut sequential_buffer = Vec::new();
        sequential
            .move_to(
                LATER_CHUNK_ORDINAL,
                MOVE_QUANTUM,
                &mut sequential_buffer,
                &reporter,
            )
            .unwrap();
        let sequential_move_bytes = sequential.read_bytes() - sequential_start_bytes;
        sequential.read_records(2, &mut sequential_buffer).unwrap();
        drop(sequential);

        let mut chunk_seek = RecordDecoder::new(&verified.file, &verified.layout).unwrap();
        let chunk_start_bytes = chunk_seek.read_bytes();
        let mut chunk_buffer = Vec::new();
        chunk_seek
            .move_to(
                LATER_CHUNK_ORDINAL,
                MOVE_QUANTUM,
                &mut chunk_buffer,
                &reporter,
            )
            .unwrap();
        let chunk_move_bytes = chunk_seek.read_bytes() - chunk_start_bytes;
        assert!(
            chunk_move_bytes.saturating_mul(2) < sequential_move_bytes,
            "point format {point_format}: chunk seek read {chunk_move_bytes} bytes, sequential replay read {sequential_move_bytes}"
        );

        chunk_seek.read_records(2, &mut chunk_buffer).unwrap();
        assert_eq!(
            chunk_buffer, sequential_buffer,
            "point format {point_format}"
        );
        assert_eq!(
            raw_ticks(&chunk_buffer, verified.layout.record_len),
            [
                seek_ticks(LATER_CHUNK_ORDINAL),
                seek_ticks(LATER_CHUNK_ORDINAL + 1)
            ],
            "point format {point_format}"
        );

        let cancelled = OperationControl::new();
        cancelled.cancel();
        let mut decoder = RecordDecoder::new(&verified.file, &verified.layout).unwrap();
        assert!(matches!(
            decoder.move_to(
                LATER_CHUNK_ORDINAL,
                MOVE_QUANTUM,
                &mut Vec::new(),
                &cancelled.reporter(),
            ),
            Err(DecoderMoveError::Cancelled)
        ));
    }

    fn raw_ticks(records: &[u8], record_len: usize) -> [[i64; 3]; 2] {
        let mut ticks = [[0_i64; 3]; 2];
        for (row, record) in records.chunks_exact(record_len).enumerate() {
            for axis in 0..3 {
                let start = axis * 4;
                ticks[row][axis] = i64::from(i32::from_le_bytes(
                    record[start..start + 4].try_into().unwrap(),
                ));
            }
        }
        ticks
    }

    fn write_seek_fixture(path: &Path, point_format: u8) {
        let format = Format::new(point_format).unwrap();
        let mut builder = Builder::from((1, 4));
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
        let mut writer = Writer::from_path(path, builder.into_header().unwrap()).unwrap();
        for ordinal in 0..FIXTURE_POINTS {
            writer.write_point(seek_point(format, ordinal)).unwrap();
        }
        writer.close().unwrap();
    }

    fn seek_point(format: Format, ordinal: u64) -> Point {
        let ticks = seek_ticks(ordinal);
        let small = u8::try_from(ordinal % 251).unwrap();
        Point {
            x: f64::from(i32::try_from(ticks[0]).unwrap()),
            y: f64::from(i32::try_from(ticks[1]).unwrap()),
            z: f64::from(i32::try_from(ticks[2]).unwrap()),
            intensity: u16::from(small) * 257,
            return_number: 1,
            number_of_returns: 1,
            scan_direction: ScanDirection::LeftToRight,
            classification: Classification::Ground,
            gps_time: format
                .has_gps_time
                .then_some(f64::from(u32::try_from(ordinal).unwrap()) + 0.25),
            color: format.has_color.then_some(Color::new(
                u16::from(small),
                u16::from(small) * 2,
                u16::from(small) * 3,
            )),
            waveform: format.has_waveform.then(|| Waveform {
                wave_packet_descriptor_index: small % 31 + 1,
                byte_offset_to_waveform_data: ordinal * 13,
                waveform_packet_size_in_bytes: u32::from(small) + 1,
                return_point_waveform_location: f32::from(small) / 251.0,
                x_t: f32::from(small) * 0.5,
                y_t: -f32::from(small) * 0.25,
                z_t: f32::from(small) * 0.125,
            }),
            nir: format.has_nir.then_some(u16::from(small) * 5),
            ..Point::default()
        }
    }

    fn seek_ticks(ordinal: u64) -> [i64; 3] {
        let ordinal = i64::try_from(ordinal).unwrap();
        let mixed = ordinal * 1_103_515_245 + 12_345;
        [
            ordinal * 7 - 200_000,
            mixed % 1_000_003,
            ordinal % 997 - 498,
        ]
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
                "punctra-source-las-seek-{}-{timestamp}-{counter}",
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
}
