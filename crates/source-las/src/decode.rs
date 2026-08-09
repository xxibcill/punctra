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
use point_source::{ReadBudget, SourceError, SourceSpan};

use crate::format::{
    AttributeKind, AttributePlan, Compression, FileWitness, LasLayout, classify_io,
};

const FIXED_DECODER_WORKING_BYTES: u64 = 64 * 1024;
const VERIFICATION_WORKING_BYTES: u64 = 64 * 1024 * 1024;
const VERIFICATION_ROWS: u64 = 16_384;

pub(crate) struct LasReadAdapter {
    file: Arc<File>,
    witness: FileWitness,
    layout: Arc<LasLayout>,
}

impl LasReadAdapter {
    pub(crate) fn new(file: Arc<File>, witness: FileWitness, layout: Arc<LasLayout>) -> Self {
        Self {
            file,
            witness,
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
        self.witness.ensure_file(&self.file)?;
        let selected = selected_attributes(&self.layout, request.attributes().explicit())?;
        let max_rows = batch_rows(&self.layout, &selected, request.budget())?;
        let decoder = RecordDecoder::new(&self.file, &self.layout)?;
        let spans = request.spans().to_vec();
        let next_ordinal = spans.first().map_or(0, |span| span.first_ordinal());
        Ok(Box::new(LasRead {
            file: Arc::clone(&self.file),
            witness: self.witness.clone(),
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
                .ok_or(SourceError::UnknownAttribute { attribute: *id })
        })
        .collect()
}

fn batch_rows(
    layout: &LasLayout,
    selected: &[AttributePlan],
    budget: ReadBudget,
) -> Result<u64, SourceError> {
    let canonical_row_bytes = selected.iter().try_fold(24_u64, |bytes, attribute| {
        bytes
            .checked_add(u64::from(attribute.definition.data_type().element_bytes()))
            .ok_or(SourceError::ResourceLimit {
                limit: "canonical Point payload bytes",
                required: u64::MAX,
                allowed: budget.max_batch_payload_bytes(),
            })
    })?;
    let rows_by_payload = budget.max_batch_payload_bytes() / canonical_row_bytes;
    if rows_by_payload == 0 {
        return Err(SourceError::ResourceLimit {
            limit: "batch payload bytes",
            required: canonical_row_bytes,
            allowed: budget.max_batch_payload_bytes(),
        });
    }

    let fixed = FIXED_DECODER_WORKING_BYTES
        .checked_add(layout.compression.decoder_bytes())
        .ok_or(SourceError::ResourceLimit {
            limit: "adapter working bytes",
            required: u64::MAX,
            allowed: budget.max_adapter_working_bytes(),
        })?;
    let record_len = u64::try_from(layout.record_len)
        .map_err(|_| SourceError::adapter("LAS record width does not fit u64"))?;
    let required = fixed
        .checked_add(record_len)
        .ok_or(SourceError::ResourceLimit {
            limit: "adapter working bytes",
            required: u64::MAX,
            allowed: budget.max_adapter_working_bytes(),
        })?;
    if required > budget.max_adapter_working_bytes() {
        return Err(SourceError::ResourceLimit {
            limit: "adapter working bytes",
            required,
            allowed: budget.max_adapter_working_bytes(),
        });
    }
    let rows_by_working = (budget.max_adapter_working_bytes() - fixed) / record_len;
    Ok(budget
        .max_batch_points()
        .min(rows_by_payload)
        .min(rows_by_working))
}

struct LasRead {
    file: Arc<File>,
    witness: FileWitness,
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
        if let Err(error) = self.witness.ensure_file(&self.file) {
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
            let error = self.decode_failure(&error);
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
        if let Err(error) = self.witness.ensure_file(&self.file) {
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
        self.witness
            .ensure_file(&self.file)
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

enum RecordDecoder {
    Las {
        reader: BufReader<File>,
        point_offset: u64,
        record_len: u64,
        current: u64,
    },
    Laz {
        decoder: LasZipDecompressor<'static, BufReader<File>>,
        record_len: u64,
        current: u64,
    },
}

impl RecordDecoder {
    fn new(file: &File, layout: &LasLayout) -> Result<Self, SourceError> {
        let mut reader = BufReader::new(file.try_clone().map_err(classify_io)?);
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
            Compression::Laz { vlr, .. } => {
                let decoder = LasZipDecompressor::new(reader, vlr.clone())
                    .map_err(|error| SourceError::corrupt(error.to_string()))?;
                Ok(Self::Laz {
                    decoder,
                    record_len,
                    current: 0,
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
    ) -> Result<(), String> {
        if let Self::Las {
            reader,
            point_offset,
            record_len,
            current,
        } = self
        {
            let offset = target
                .checked_mul(*record_len)
                .and_then(|bytes| point_offset.checked_add(bytes))
                .ok_or_else(|| "LAS point seek offset overflow".to_owned())?;
            reader
                .seek(SeekFrom::Start(offset))
                .map_err(|error| error.to_string())?;
            *current = target;
            return Ok(());
        }

        let current = match self {
            Self::Laz { current, .. } => *current,
            Self::Las { .. } => unreachable!("LAS reads returned above"),
        };
        if target < current {
            return Err("LAZ decoder cannot move backward within one sorted read".to_owned());
        }
        while match self {
            Self::Laz { current, .. } => *current < target,
            Self::Las { .. } => false,
        } {
            reporter
                .check_cancelled()
                .map_err(|error| error.to_string())?;
            let current = match self {
                Self::Laz { current, .. } => *current,
                Self::Las { .. } => unreachable!("LAS reads returned above"),
            };
            let count = (target - current).min(quantum);
            self.read_records(count, buffer)?;
        }
        Ok(())
    }

    fn read_records(&mut self, point_count: u64, buffer: &mut Vec<u8>) -> Result<(), String> {
        let record_len = match self {
            Self::Las { record_len, .. } | Self::Laz { record_len, .. } => *record_len,
        };
        let bytes = point_count
            .checked_mul(record_len)
            .ok_or_else(|| "LAS decode buffer length overflow".to_owned())?;
        let bytes = usize::try_from(bytes)
            .map_err(|_| "LAS decode buffer does not fit the host address space".to_owned())?;
        buffer.resize(bytes, 0);
        match self {
            Self::Las {
                reader, current, ..
            } => {
                reader
                    .read_exact(buffer)
                    .map_err(|error| error.to_string())?;
                *current += point_count;
            }
            Self::Laz {
                decoder, current, ..
            } => {
                decoder
                    .decompress_many(buffer)
                    .map_err(|error| error.to_string())?;
                *current += point_count;
            }
        }
        Ok(())
    }
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
            limit: "verification working bytes",
            required: u64::MAX,
            allowed: VERIFICATION_WORKING_BYTES,
        })?;
    let record_len = u64::try_from(layout.record_len)
        .map_err(|_| SourceError::corrupt("LAS record width does not fit u64"))?;
    let required = fixed
        .checked_add(record_len)
        .ok_or(SourceError::ResourceLimit {
            limit: "verification working bytes",
            required: u64::MAX,
            allowed: VERIFICATION_WORKING_BYTES,
        })?;
    if required > VERIFICATION_WORKING_BYTES {
        return Err(SourceError::ResourceLimit {
            limit: "verification working bytes",
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
