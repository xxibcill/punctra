use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    mem,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use blake3::Hasher;
use point_contracts::{ContentHash, PointId, SourceId};

use crate::{
    PointIdReadLimits, PointSetLimits, PointSetMetadata, SnapshotProvenance, WorkspaceError,
    workspace::Session,
};

const POINT_ID_HASH_DOMAIN: &[u8] = b"punctra-point-set-ids-v1";
const CONTENT_HASH_DOMAIN: &[u8] = b"punctra-point-set-content-v1";
const FILE_HASH_DOMAIN: &[u8] = b"punctra-point-set-file-v1";
const FRAME_HASH_DOMAIN: &[u8] = b"punctra-point-set-frame-v1";
const FILE_MAGIC: &[u8; 8] = b"PSET0001";
const FILE_VERSION: u32 = 1;
const FRAME_TAG: u8 = 1;
const FOOTER_TAG: u8 = u8::MAX;
const DEFAULT_FRAME_RECORDS: usize = 4_096;
const MIN_RESIDENT_GROWTH_RECORDS: usize = 4_096;
const RANDOM_NAME_ATTEMPTS: usize = 32;

/// One bounded batch from repeatable Point Set identity iteration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PointIdBatch {
    ids: Vec<PointId>,
}

impl PointIdBatch {
    /// Returns ordered unique Point Identities.
    #[must_use]
    pub fn ids(&self) -> &[PointId] {
        &self.ids
    }

    /// Returns the exact number of Point Identities in this nonempty batch.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Reports whether this batch is empty.
    ///
    /// A successfully constructed batch always returns `false`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

/// Repeatable, bounded Point Identity batches from one immutable Point Set.
pub struct PointIdBatches {
    source: SourceId,
    cursor: PointSetRecordCursor,
    max_batch_records: usize,
    remaining: u64,
    limits: PointIdReadLimits,
    terminal: bool,
}

impl PointIdBatches {
    /// Returns the next nonempty identity batch, or terminal `None`.
    ///
    /// A corruption or resource error is returned once; later calls are fused
    /// to `None`.
    ///
    /// # Errors
    ///
    /// Returns an error when spill storage is missing, changed, corrupt, or
    /// cannot be read within the declared limits.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<PointIdBatch>, WorkspaceError> {
        if self.terminal {
            return Ok(None);
        }
        if let Err(error) = self.cursor.verify_storage() {
            return self.fail(error);
        }
        let target = match batch_target(self.max_batch_records, self.remaining) {
            Ok(target) => target,
            Err(error) => return self.fail(error),
        };
        let mut ids =
            match allocate_batch::<PointId>(target, self.limits, self.cursor.read_buffer_bytes()) {
                Ok(ids) => ids,
                Err(error) => return self.fail(error),
            };
        while ids.len() < target {
            let output_bytes = allocation_bytes::<PointId>(ids.capacity());
            let record = match self.cursor.next_record(output_bytes, self.limits) {
                Ok(Some(record)) => record,
                Ok(None) => {
                    return self.fail(WorkspaceError::invalid_point_set(
                        "Point Set ended before its sealed count",
                    ));
                }
                Err(error) => return self.fail(error),
            };
            ids.push(PointId::new(self.source, record.ordinal));
        }
        let Ok(emitted) = u64::try_from(ids.len()) else {
            return self.fail(WorkspaceError::invalid_point_set(
                "Point Set batch length does not fit u64",
            ));
        };
        self.remaining -= emitted;
        if !ids.is_empty() {
            if let Err(error) = self.cursor.verify_storage() {
                return self.fail(error);
            }
            return Ok(Some(PointIdBatch { ids }));
        }
        match self.cursor.next_record(0, self.limits) {
            Ok(None) => {
                self.terminal = true;
                Ok(None)
            }
            Ok(Some(_)) => self.fail(WorkspaceError::invalid_point_set(
                "Point Set contains records beyond its sealed count",
            )),
            Err(error) => self.fail(error),
        }
    }

    fn fail<T>(&mut self, error: WorkspaceError) -> Result<T, WorkspaceError> {
        self.terminal = true;
        Err(error)
    }
}

/// Immutable process-scoped Point Identities captured at one Snapshot.
#[derive(Clone)]
pub struct PointSet {
    inner: Arc<PointSetInner>,
}

impl PointSet {
    /// Returns immutable provenance, count, and canonical hashes.
    #[must_use]
    pub fn metadata(&self) -> &PointSetMetadata {
        &self.inner.metadata
    }

    /// Starts a repeatable bounded identity read.
    ///
    /// # Errors
    ///
    /// Returns an error when one identity cannot fit in a requested batch, or
    /// when checked spill storage is missing, changed, or corrupt.
    pub fn ids(&self, limits: PointIdReadLimits) -> Result<PointIdBatches, WorkspaceError> {
        let exact_count = self.inner.metadata.exact_count();
        require_read_count(exact_count, limits)?;
        let cursor = PointSetRecordCursor::new(self.clone(), limits)?;
        let max_batch_records =
            bounded_batch_records(limits, exact_count, cursor.read_buffer_bytes())?;
        Ok(PointIdBatches {
            source: self.inner.metadata.provenance().source(),
            cursor,
            max_batch_records,
            remaining: exact_count,
            limits,
            terminal: false,
        })
    }

    pub(crate) fn records(
        &self,
        limits: PointIdReadLimits,
    ) -> Result<PointSetRecordBatches, WorkspaceError> {
        PointSetRecordBatches::new(self.clone(), limits)
    }

    pub(crate) fn commit_metadata(&self) -> &PointSetMetadata {
        self.metadata()
    }
}

impl std::fmt::Debug for PointSet {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PointSet")
            .field("metadata", &self.inner.metadata)
            .finish_non_exhaustive()
    }
}

struct PointSetInner {
    metadata: PointSetMetadata,
    storage: PointSetStorage,
    _session: Arc<Session>,
}

impl Drop for PointSetInner {
    fn drop(&mut self) {
        if let PointSetStorage::Spill(spill) = &self.storage {
            let _ = fs::remove_file(&spill.path);
        }
    }
}

enum PointSetStorage {
    Memory(Vec<PointSetRecord>),
    Spill(SpillDescriptor),
}

struct SpillDescriptor {
    path: PathBuf,
    file_bytes: u64,
    modified: SystemTime,
    max_frame_records: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PointSetRecord {
    pub(crate) ordinal: u64,
    pub(crate) effective_classification: u8,
}

pub(crate) struct PointSetRecordBatches {
    cursor: PointSetRecordCursor,
    max_batch_records: usize,
    remaining: u64,
    limits: PointIdReadLimits,
    terminal: bool,
}

impl PointSetRecordBatches {
    fn new(owner: PointSet, limits: PointIdReadLimits) -> Result<Self, WorkspaceError> {
        let exact_count = owner.metadata().exact_count();
        require_read_count(exact_count, limits)?;
        let cursor = PointSetRecordCursor::new(owner, limits)?;
        let max_read_buffer_bytes = cursor.read_buffer_bytes();
        Ok(Self {
            cursor,
            max_batch_records: bounded_batch_records(limits, exact_count, max_read_buffer_bytes)?,
            remaining: exact_count,
            limits,
            terminal: false,
        })
    }

    #[allow(clippy::should_implement_trait)]
    pub(crate) fn next(&mut self) -> Result<Option<Vec<PointSetRecord>>, WorkspaceError> {
        if self.terminal {
            return Ok(None);
        }
        if let Err(error) = self.cursor.verify_storage() {
            return self.fail(error);
        }
        let target = match batch_target(self.max_batch_records, self.remaining) {
            Ok(target) => target,
            Err(error) => return self.fail(error),
        };
        let mut records = match allocate_batch::<PointSetRecord>(
            target,
            self.limits,
            self.cursor.read_buffer_bytes(),
        ) {
            Ok(records) => records,
            Err(error) => return self.fail(error),
        };
        while records.len() < target {
            let output_bytes = allocation_bytes::<PointSetRecord>(records.capacity());
            let record = match self.cursor.next_record(output_bytes, self.limits) {
                Ok(Some(record)) => record,
                Ok(None) => {
                    return self.fail(WorkspaceError::invalid_point_set(
                        "Point Set ended before its sealed count",
                    ));
                }
                Err(error) => return self.fail(error),
            };
            records.push(record);
        }
        let Ok(emitted) = u64::try_from(records.len()) else {
            return self.fail(WorkspaceError::invalid_point_set(
                "Point Set batch length does not fit u64",
            ));
        };
        self.remaining -= emitted;
        if !records.is_empty() {
            if let Err(error) = self.cursor.verify_storage() {
                return self.fail(error);
            }
            return Ok(Some(records));
        }
        match self.cursor.next_record(0, self.limits) {
            Ok(None) => {
                self.terminal = true;
                Ok(None)
            }
            Ok(Some(_)) => self.fail(WorkspaceError::invalid_point_set(
                "Point Set contains records beyond its sealed count",
            )),
            Err(error) => self.fail(error),
        }
    }

    fn fail<T>(&mut self, error: WorkspaceError) -> Result<T, WorkspaceError> {
        self.terminal = true;
        Err(error)
    }
}

struct PointSetRecordCursor {
    owner: PointSet,
    reader: RecordReader,
}

impl PointSetRecordCursor {
    fn new(owner: PointSet, limits: PointIdReadLimits) -> Result<Self, WorkspaceError> {
        let reader = match &owner.inner.storage {
            PointSetStorage::Memory(records) => RecordReader::Memory {
                next: 0,
                len: records.len(),
            },
            PointSetStorage::Spill(spill) => RecordReader::Spill(Box::new(SpillReader::open(
                spill,
                &owner.inner.metadata,
                limits,
            )?)),
        };
        Ok(Self { owner, reader })
    }

    fn next_record(
        &mut self,
        output_bytes: u64,
        limits: PointIdReadLimits,
    ) -> Result<Option<PointSetRecord>, WorkspaceError> {
        match &mut self.reader {
            RecordReader::Memory { next, len } => {
                if *next == *len {
                    Ok(None)
                } else {
                    require_working(output_bytes, 0, limits)?;
                    let record = match &self.owner.inner.storage {
                        PointSetStorage::Memory(records) => records[*next],
                        PointSetStorage::Spill(_) => unreachable!("reader and storage agree"),
                    };
                    *next += 1;
                    Ok(Some(record))
                }
            }
            RecordReader::Spill(reader) => reader.next_record(output_bytes, limits),
        }
    }

    fn read_buffer_bytes(&self) -> u64 {
        match &self.reader {
            RecordReader::Memory { .. } => 0,
            RecordReader::Spill(reader) => reader.buffer_bytes(),
        }
    }

    fn verify_storage(&self) -> Result<(), WorkspaceError> {
        match &self.reader {
            RecordReader::Memory { .. } => Ok(()),
            RecordReader::Spill(reader) => reader.verify_path_state(),
        }
    }
}

enum RecordReader {
    Memory { next: usize, len: usize },
    Spill(Box<SpillReader>),
}

pub(crate) struct PointSetBuilder {
    session: Arc<Session>,
    provenance: SnapshotProvenance,
    limits: PointSetLimits,
    storage: BuilderStorage,
    exact_count: u64,
    previous_ordinal: Option<u64>,
    point_id_hasher: Hasher,
    content_hasher: Hasher,
}

impl PointSetBuilder {
    pub(crate) fn new(
        session: Arc<Session>,
        provenance: SnapshotProvenance,
        limits: PointSetLimits,
    ) -> Self {
        let point_id_hasher = point_id_hasher(&provenance);
        let content_hasher = content_hasher(&provenance);
        Self {
            session,
            provenance,
            limits,
            storage: BuilderStorage::Memory(Vec::new()),
            exact_count: 0,
            previous_ordinal: None,
            point_id_hasher,
            content_hasher,
        }
    }

    pub(crate) fn push(
        &mut self,
        record: PointSetRecord,
        other_working_bytes: u64,
    ) -> Result<(), WorkspaceError> {
        self.validate_record(record)?;
        let next_count = checked_add_with_limit(
            self.exact_count,
            1,
            "selected Points",
            self.limits.max_output_points(),
        )?;
        self.store(record, other_working_bytes)?;
        self.exact_count = next_count;
        update_hashes(&mut self.point_id_hasher, &mut self.content_hasher, record);
        self.previous_ordinal = Some(record.ordinal);
        Ok(())
    }

    pub(crate) fn resident_bytes(&self) -> u64 {
        match &self.storage {
            BuilderStorage::Memory(records) => resident_record_bytes(records.capacity()),
            BuilderStorage::Spill(writer) => writer.buffer_bytes(),
        }
    }

    pub(crate) fn finish(self) -> Result<PointSet, WorkspaceError> {
        let point_id_hash = ContentHash::new(*self.point_id_hasher.finalize().as_bytes());
        let content_hash = ContentHash::new(*self.content_hasher.finalize().as_bytes());
        let metadata = PointSetMetadata::new(
            self.provenance,
            self.exact_count,
            point_id_hash,
            content_hash,
        );
        let storage = match self.storage {
            BuilderStorage::Memory(records) => PointSetStorage::Memory(records),
            BuilderStorage::Spill(writer) => PointSetStorage::Spill(writer.finish(&metadata)?),
        };
        Ok(PointSet {
            inner: Arc::new(PointSetInner {
                metadata,
                storage,
                _session: self.session,
            }),
        })
    }

    fn validate_record(&self, record: PointSetRecord) -> Result<(), WorkspaceError> {
        if self
            .previous_ordinal
            .is_some_and(|previous| record.ordinal <= previous)
        {
            return Err(WorkspaceError::invalid_point_set(
                "selection records are not strictly Source-ordinal ordered",
            ));
        }
        Ok(())
    }

    fn store(
        &mut self,
        record: PointSetRecord,
        other_working_bytes: u64,
    ) -> Result<(), WorkspaceError> {
        let must_spill = match &mut self.storage {
            BuilderStorage::Memory(records) => {
                let next_len =
                    records
                        .len()
                        .checked_add(1)
                        .ok_or(WorkspaceError::ResourceLimit {
                            limit: "resident Point Set bytes",
                            required: u64::MAX,
                            allowed: self.limits.max_resident_bytes(),
                        })?;
                let available_working_bytes = self
                    .limits
                    .max_working_bytes()
                    .saturating_sub(other_working_bytes);
                let available_resident_bytes = self
                    .limits
                    .max_resident_bytes()
                    .min(available_working_bytes);
                let next_exceeds_limit = resident_record_bytes(next_len) > available_resident_bytes;
                if next_exceeds_limit {
                    true
                } else {
                    let growth_blocked = records.len() == records.capacity()
                        && !grow_resident_records(records, next_len, available_resident_bytes)?;
                    growth_blocked
                        || resident_record_bytes(records.capacity()) > available_resident_bytes
                }
            }
            BuilderStorage::Spill(writer) => {
                require_selection_working(
                    writer.buffer_bytes(),
                    other_working_bytes,
                    self.limits.max_working_bytes(),
                )?;
                writer.push(record)?;
                return Ok(());
            }
        };
        if must_spill {
            self.begin_spill(other_working_bytes)?;
        }
        match &mut self.storage {
            BuilderStorage::Memory(records) => records.push(record),
            BuilderStorage::Spill(writer) => writer.push(record)?,
        }
        Ok(())
    }

    fn begin_spill(&mut self, other_working_bytes: u64) -> Result<(), WorkspaceError> {
        let memory = match mem::replace(&mut self.storage, BuilderStorage::Memory(Vec::new())) {
            BuilderStorage::Memory(records) => records,
            BuilderStorage::Spill(writer) => {
                self.storage = BuilderStorage::Spill(writer);
                return Ok(());
            }
        };
        let mut writer = SpillWriter::create(
            self.session.scratch_path(),
            self.limits.max_temporary_bytes(),
            self.limits
                .max_working_bytes()
                .saturating_sub(other_working_bytes),
        )?;
        if let Err(error) = writer.write_existing(&memory) {
            self.storage = BuilderStorage::Memory(memory);
            return Err(error);
        }
        drop(memory);
        writer.allocate_buffer()?;
        require_selection_working(
            writer.buffer_bytes(),
            other_working_bytes,
            self.limits.max_working_bytes(),
        )?;
        self.storage = BuilderStorage::Spill(Box::new(writer));
        Ok(())
    }
}

fn grow_resident_records(
    records: &mut Vec<PointSetRecord>,
    next_len: usize,
    available_bytes: u64,
) -> Result<bool, WorkspaceError> {
    let record_bytes = allocation_bytes::<PointSetRecord>(1).max(1);
    let old_bytes = resident_record_bytes(records.capacity());
    let maximum_new_bytes = available_bytes.saturating_sub(old_bytes);
    let maximum_records = usize::try_from(maximum_new_bytes / record_bytes).unwrap_or(usize::MAX);
    if next_len > maximum_records {
        return Ok(false);
    }
    let target = records
        .capacity()
        .saturating_mul(2)
        .max(MIN_RESIDENT_GROWTH_RECORDS)
        .min(maximum_records)
        .max(next_len);
    if target <= records.capacity() {
        return Ok(false);
    }
    records
        .try_reserve_exact(target.saturating_sub(records.len()))
        .map_err(|_| WorkspaceError::ResourceLimit {
            limit: "resident Point Set allocation",
            required: resident_record_bytes(target),
            allowed: available_bytes,
        })?;
    let actual = resident_record_bytes(records.capacity());
    let overlap = old_bytes.saturating_add(actual);
    if overlap > available_bytes {
        return Err(WorkspaceError::ResourceLimit {
            limit: "resident Point Set reallocation overlap",
            required: overlap,
            allowed: available_bytes,
        });
    }
    Ok(true)
}

enum BuilderStorage {
    Memory(Vec<PointSetRecord>),
    Spill(Box<SpillWriter>),
}

struct SpillWriter {
    path: PathBuf,
    file: File,
    file_hasher: Hasher,
    bytes_written: u64,
    max_temporary_bytes: u64,
    frame_capacity: usize,
    max_frame_records_written: usize,
    buffer: Vec<PointSetRecord>,
    keep: bool,
}

impl SpillWriter {
    fn create(
        scratch: &Path,
        max_temporary_bytes: u64,
        max_working_bytes: u64,
    ) -> Result<Self, WorkspaceError> {
        let frame_capacity = frame_capacity(max_working_bytes)?;
        let (path, file) = create_spill_file(scratch)?;
        let mut writer = Self {
            path,
            file,
            file_hasher: domain_hasher(FILE_HASH_DOMAIN),
            bytes_written: 0,
            max_temporary_bytes,
            frame_capacity,
            max_frame_records_written: 0,
            buffer: Vec::new(),
            keep: false,
        };
        writer.write_hashed(FILE_MAGIC)?;
        writer.write_hashed(&FILE_VERSION.to_le_bytes())?;
        Ok(writer)
    }

    fn write_existing(&mut self, records: &[PointSetRecord]) -> Result<(), WorkspaceError> {
        for frame in records.chunks(self.frame_capacity) {
            self.write_frame(frame)?;
        }
        Ok(())
    }

    fn allocate_buffer(&mut self) -> Result<(), WorkspaceError> {
        self.buffer
            .try_reserve_exact(self.frame_capacity)
            .map_err(|_| WorkspaceError::ResourceLimit {
                limit: "Point Set spill write buffer",
                required: batch_record_bytes(self.frame_capacity),
                allowed: self.max_working_buffer_bytes(),
            })?;
        let actual = self.buffer_bytes();
        if actual > self.max_working_buffer_bytes() {
            return Err(WorkspaceError::ResourceLimit {
                limit: "Point Set spill write buffer",
                required: actual,
                allowed: self.max_working_buffer_bytes(),
            });
        }
        Ok(())
    }

    fn push(&mut self, record: PointSetRecord) -> Result<(), WorkspaceError> {
        self.buffer.push(record);
        if self.buffer.len() == self.frame_capacity {
            self.flush_buffer()?;
        }
        Ok(())
    }

    fn buffer_bytes(&self) -> u64 {
        resident_record_bytes(self.buffer.capacity())
    }

    fn max_working_buffer_bytes(&self) -> u64 {
        batch_record_bytes(self.frame_capacity)
    }

    fn finish(mut self, metadata: &PointSetMetadata) -> Result<SpillDescriptor, WorkspaceError> {
        self.flush_buffer()?;
        self.write_hashed(&[FOOTER_TAG])?;
        self.write_hashed(&metadata.exact_count().to_le_bytes())?;
        self.write_hashed(metadata.point_id_hash().as_bytes())?;
        self.write_hashed(metadata.content_hash().as_bytes())?;
        let file_checksum = *self.file_hasher.finalize().as_bytes();
        self.write_raw(&file_checksum)?;
        self.file.flush().map_err(|error| {
            WorkspaceError::io("flush Point Set spill", self.path.display(), error)
        })?;
        let file_metadata = self.file.metadata().map_err(|error| {
            WorkspaceError::io("inspect Point Set spill", self.path.display(), error)
        })?;
        let file_bytes = file_metadata.len();
        if file_bytes != self.bytes_written {
            return Err(WorkspaceError::invalid_point_set(
                "sealed spill length differs from bytes written",
            ));
        }
        let modified = file_metadata.modified().map_err(|error| {
            WorkspaceError::io(
                "inspect Point Set spill timestamp",
                self.path.display(),
                error,
            )
        })?;
        self.keep = true;
        Ok(SpillDescriptor {
            path: self.path.clone(),
            file_bytes,
            modified,
            max_frame_records: self.max_frame_records_written,
        })
    }

    fn flush_buffer(&mut self) -> Result<(), WorkspaceError> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let records = mem::take(&mut self.buffer);
        let result = self.write_frame(&records);
        self.buffer = records;
        self.buffer.clear();
        result
    }

    fn write_frame(&mut self, records: &[PointSetRecord]) -> Result<(), WorkspaceError> {
        let count = u32::try_from(records.len()).map_err(|_| WorkspaceError::ResourceLimit {
            limit: "Point Set spill frame records",
            required: u64::try_from(records.len()).unwrap_or(u64::MAX),
            allowed: u64::from(u32::MAX),
        })?;
        if count == 0 {
            return Ok(());
        }
        self.max_frame_records_written = self.max_frame_records_written.max(records.len());
        let count_bytes = count.to_le_bytes();
        self.write_hashed(&[FRAME_TAG])?;
        self.write_hashed(&count_bytes)?;
        let mut frame_hasher = domain_hasher(FRAME_HASH_DOMAIN);
        frame_hasher.update(&count_bytes);
        for record in records {
            let encoded = encode_record(*record);
            frame_hasher.update(&encoded);
            self.write_hashed(&encoded)?;
        }
        self.write_hashed(frame_hasher.finalize().as_bytes())
    }

    fn write_hashed(&mut self, bytes: &[u8]) -> Result<(), WorkspaceError> {
        self.file_hasher.update(bytes);
        self.write_raw(bytes)
    }

    fn write_raw(&mut self, bytes: &[u8]) -> Result<(), WorkspaceError> {
        let added = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let required = self.bytes_written.saturating_add(added);
        if required > self.max_temporary_bytes {
            return Err(WorkspaceError::ResourceLimit {
                limit: "Point Set temporary bytes",
                required,
                allowed: self.max_temporary_bytes,
            });
        }
        self.file.write_all(bytes).map_err(|error| {
            WorkspaceError::io("write Point Set spill", self.path.display(), error)
        })?;
        self.bytes_written = required;
        Ok(())
    }
}

impl Drop for SpillWriter {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_file(&self.path);
        }
    }
}

struct SpillReader {
    path: PathBuf,
    file: File,
    sealed_file_bytes: u64,
    sealed_modified: SystemTime,
    file_hasher: Hasher,
    frame: Vec<PointSetRecord>,
    sealed_max_frame_records: usize,
    next_frame_record: usize,
    expected_count: u64,
    expected_point_id_hash: ContentHash,
    expected_content_hash: ContentHash,
    emitted_count: u64,
    previous_ordinal: Option<u64>,
    point_id_hasher: Hasher,
    content_hasher: Hasher,
    terminal: bool,
}

impl SpillReader {
    fn open(
        descriptor: &SpillDescriptor,
        metadata: &PointSetMetadata,
        limits: PointIdReadLimits,
    ) -> Result<Self, WorkspaceError> {
        let file = File::open(&descriptor.path).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                WorkspaceError::invalid_point_set("Point Set spill is missing")
            } else {
                WorkspaceError::io("open Point Set spill", descriptor.path.display(), error)
            }
        })?;
        let actual_bytes = file
            .metadata()
            .map_err(|error| {
                WorkspaceError::io("inspect Point Set spill", descriptor.path.display(), error)
            })?
            .len();
        if actual_bytes != descriptor.file_bytes {
            return Err(WorkspaceError::invalid_point_set(
                "Point Set spill length changed after sealing",
            ));
        }
        let requested_buffer_bytes = batch_record_bytes(descriptor.max_frame_records);
        if requested_buffer_bytes > limits.max_read_buffer_bytes() {
            return Err(WorkspaceError::ResourceLimit {
                limit: "Point Set spill read buffer",
                required: requested_buffer_bytes,
                allowed: limits.max_read_buffer_bytes(),
            });
        }
        let mut frame = Vec::new();
        frame
            .try_reserve_exact(descriptor.max_frame_records)
            .map_err(|_| WorkspaceError::ResourceLimit {
                limit: "Point Set spill read buffer",
                required: requested_buffer_bytes,
                allowed: limits.max_read_buffer_bytes(),
            })?;
        let mut reader = Self {
            path: descriptor.path.clone(),
            file,
            sealed_file_bytes: descriptor.file_bytes,
            sealed_modified: descriptor.modified,
            file_hasher: domain_hasher(FILE_HASH_DOMAIN),
            frame,
            sealed_max_frame_records: descriptor.max_frame_records,
            next_frame_record: 0,
            expected_count: metadata.exact_count(),
            expected_point_id_hash: metadata.point_id_hash(),
            expected_content_hash: metadata.content_hash(),
            emitted_count: 0,
            previous_ordinal: None,
            point_id_hasher: point_id_hasher(&metadata.provenance()),
            content_hasher: content_hasher(&metadata.provenance()),
            terminal: false,
        };
        require_working(0, reader.buffer_bytes(), limits)?;
        reader.verify_path_state()?;
        let magic = reader.read_hashed_array::<8>()?;
        let version = u32::from_le_bytes(reader.read_hashed_array::<4>()?);
        if &magic != FILE_MAGIC || version != FILE_VERSION {
            return Err(WorkspaceError::invalid_point_set(
                "Point Set spill header is incompatible or corrupt",
            ));
        }
        Ok(reader)
    }

    fn next_record(
        &mut self,
        output_bytes: u64,
        limits: PointIdReadLimits,
    ) -> Result<Option<PointSetRecord>, WorkspaceError> {
        if self.terminal {
            return Ok(None);
        }
        loop {
            if self.next_frame_record < self.frame.len() {
                require_working(output_bytes, self.buffer_bytes(), limits)?;
                let record = self.frame[self.next_frame_record];
                self.next_frame_record += 1;
                return Ok(Some(record));
            }
            self.frame.clear();
            self.next_frame_record = 0;
            self.verify_path_state()?;
            let tag = self.read_hashed_array::<1>()?[0];
            match tag {
                FRAME_TAG => self.read_verified_frame(output_bytes, limits)?,
                FOOTER_TAG => return self.read_footer(),
                _ => {
                    return Err(WorkspaceError::invalid_point_set(
                        "Point Set spill contains an unknown frame tag",
                    ));
                }
            }
        }
    }

    fn read_verified_frame(
        &mut self,
        output_bytes: u64,
        limits: PointIdReadLimits,
    ) -> Result<(), WorkspaceError> {
        let count_bytes = self.read_hashed_array::<4>()?;
        let encoded_count = u32::from_le_bytes(count_bytes);
        if encoded_count == 0 {
            return Err(WorkspaceError::invalid_point_set(
                "Point Set spill contains an empty frame",
            ));
        }
        let count = usize::try_from(encoded_count).map_err(|_| {
            WorkspaceError::invalid_point_set("Point Set spill frame count does not fit usize")
        })?;
        if count > DEFAULT_FRAME_RECORDS {
            return Err(WorkspaceError::invalid_point_set(
                "Point Set spill frame exceeds the format maximum",
            ));
        }
        let candidate_count = self
            .emitted_count
            .checked_add(u64::from(encoded_count))
            .ok_or_else(|| {
                WorkspaceError::invalid_point_set("Point Set spill record count overflowed")
            })?;
        if candidate_count > self.expected_count {
            return Err(WorkspaceError::invalid_point_set(
                "Point Set spill contains more records than its metadata",
            ));
        }

        if count > self.sealed_max_frame_records {
            return Err(WorkspaceError::invalid_point_set(
                "Point Set spill frame exceeds its sealed maximum",
            ));
        }
        let buffer_bytes = self.buffer_bytes();
        if buffer_bytes > limits.max_read_buffer_bytes() {
            return Err(WorkspaceError::ResourceLimit {
                limit: "Point Set spill read buffer",
                required: buffer_bytes,
                allowed: limits.max_read_buffer_bytes(),
            });
        }
        require_working(output_bytes, buffer_bytes, limits)?;

        let mut frame_hasher = domain_hasher(FRAME_HASH_DOMAIN);
        frame_hasher.update(&count_bytes);
        let mut previous = self.previous_ordinal;
        for _ in 0..count {
            let encoded = self.read_hashed_array::<9>()?;
            frame_hasher.update(&encoded);
            let record = decode_record(encoded);
            if previous.is_some_and(|ordinal| record.ordinal <= ordinal) {
                return Err(WorkspaceError::invalid_point_set(
                    "Point Set spill records are not strictly ordered",
                ));
            }
            previous = Some(record.ordinal);
            self.frame.push(record);
        }
        let actual_checksum = self.read_hashed_array::<32>()?;
        if actual_checksum != *frame_hasher.finalize().as_bytes() {
            self.frame.clear();
            return Err(WorkspaceError::invalid_point_set(
                "Point Set spill frame checksum changed",
            ));
        }

        for &record in &self.frame {
            update_hashes(&mut self.point_id_hasher, &mut self.content_hasher, record);
        }
        self.emitted_count = candidate_count;
        self.previous_ordinal = previous;
        Ok(())
    }

    fn read_footer(&mut self) -> Result<Option<PointSetRecord>, WorkspaceError> {
        let exact_count = u64::from_le_bytes(self.read_hashed_array::<8>()?);
        let point_id_hash = ContentHash::new(self.read_hashed_array::<32>()?);
        let content_hash = ContentHash::new(self.read_hashed_array::<32>()?);
        let expected_file_hash = *self.file_hasher.finalize().as_bytes();
        let actual_file_hash = self.read_raw_array::<32>()?;
        let calculated_point_id_hash =
            ContentHash::new(*self.point_id_hasher.finalize().as_bytes());
        let calculated_content_hash = ContentHash::new(*self.content_hasher.finalize().as_bytes());
        if exact_count != self.expected_count
            || exact_count != self.emitted_count
            || point_id_hash != self.expected_point_id_hash
            || content_hash != self.expected_content_hash
            || calculated_point_id_hash != self.expected_point_id_hash
            || calculated_content_hash != self.expected_content_hash
            || actual_file_hash != expected_file_hash
        {
            return Err(WorkspaceError::invalid_point_set(
                "Point Set spill footer differs from sealed metadata",
            ));
        }
        let mut trailing = [0_u8; 1];
        match self.file.read(&mut trailing) {
            Ok(0) => {}
            Ok(_) => {
                return Err(WorkspaceError::invalid_point_set(
                    "Point Set spill contains trailing bytes",
                ));
            }
            Err(error) => {
                return Err(WorkspaceError::io(
                    "verify Point Set spill end",
                    self.path.display(),
                    error,
                ));
            }
        }
        self.terminal = true;
        Ok(None)
    }

    fn buffer_bytes(&self) -> u64 {
        resident_record_bytes(self.frame.capacity())
    }

    fn verify_path_state(&self) -> Result<(), WorkspaceError> {
        let actual = fs::metadata(&self.path).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                WorkspaceError::invalid_point_set("Point Set spill is missing")
            } else {
                WorkspaceError::io("inspect Point Set spill", self.path.display(), error)
            }
        })?;
        if actual.len() != self.sealed_file_bytes {
            return Err(WorkspaceError::invalid_point_set(
                "Point Set spill length changed after sealing",
            ));
        }
        let modified = actual.modified().map_err(|error| {
            WorkspaceError::io(
                "inspect Point Set spill timestamp",
                self.path.display(),
                error,
            )
        })?;
        if modified != self.sealed_modified {
            return Err(WorkspaceError::invalid_point_set(
                "Point Set spill timestamp changed after sealing",
            ));
        }
        Ok(())
    }

    fn read_hashed_array<const N: usize>(&mut self) -> Result<[u8; N], WorkspaceError> {
        let bytes = self.read_raw_array::<N>()?;
        self.file_hasher.update(&bytes);
        Ok(bytes)
    }

    fn read_raw_array<const N: usize>(&mut self) -> Result<[u8; N], WorkspaceError> {
        let mut bytes = [0_u8; N];
        self.file.read_exact(&mut bytes).map_err(|error| {
            if error.kind() == io::ErrorKind::UnexpectedEof {
                WorkspaceError::invalid_point_set("Point Set spill is truncated")
            } else {
                WorkspaceError::io("read Point Set spill", self.path.display(), error)
            }
        })?;
        Ok(bytes)
    }
}

fn require_read_count(exact_count: u64, limits: PointIdReadLimits) -> Result<(), WorkspaceError> {
    if exact_count > limits.max_points() {
        return Err(WorkspaceError::ResourceLimit {
            limit: "Point Set identity read Points",
            required: exact_count,
            allowed: limits.max_points(),
        });
    }
    Ok(())
}

fn bounded_batch_records(
    limits: PointIdReadLimits,
    exact_count: u64,
    required_read_buffer_bytes: u64,
) -> Result<usize, WorkspaceError> {
    if exact_count == 0 {
        return Ok(0);
    }
    if required_read_buffer_bytes > limits.max_read_buffer_bytes() {
        return Err(WorkspaceError::ResourceLimit {
            limit: "Point Set spill read buffer",
            required: required_read_buffer_bytes,
            allowed: limits.max_read_buffer_bytes(),
        });
    }
    if required_read_buffer_bytes >= limits.max_working_bytes() {
        return Err(WorkspaceError::ResourceLimit {
            limit: "Point Set identity read working bytes",
            required: required_read_buffer_bytes.saturating_add(allocation_bytes::<PointId>(1)),
            allowed: limits.max_working_bytes(),
        });
    }
    let point_bytes = allocation_bytes::<PointId>(1);
    let by_payload = limits.max_batch_bytes() / point_bytes;
    let by_working = limits
        .max_working_bytes()
        .saturating_sub(required_read_buffer_bytes)
        / point_bytes;
    let records = exact_count
        .min(limits.max_batch_points())
        .min(by_payload)
        .min(by_working);
    if records == 0 {
        return Err(WorkspaceError::ResourceLimit {
            limit: "Point Set identity batch capacity",
            required: point_bytes,
            allowed: limits.max_batch_bytes().min(limits.max_working_bytes()),
        });
    }
    usize::try_from(records).map_err(|_| WorkspaceError::ResourceLimit {
        limit: "Point Set identity read batch records",
        required: records,
        allowed: u64::try_from(usize::MAX).unwrap_or(u64::MAX),
    })
}

fn batch_target(max_batch_records: usize, remaining: u64) -> Result<usize, WorkspaceError> {
    let batch_limit = u64::try_from(max_batch_records).unwrap_or(u64::MAX);
    usize::try_from(remaining.min(batch_limit)).map_err(|_| WorkspaceError::ResourceLimit {
        limit: "Point Set identity read batch records",
        required: remaining,
        allowed: u64::try_from(usize::MAX).unwrap_or(u64::MAX),
    })
}

fn allocate_batch<T>(
    target: usize,
    limits: PointIdReadLimits,
    read_buffer_bytes: u64,
) -> Result<Vec<T>, WorkspaceError> {
    let mut values = Vec::new();
    if target == 0 {
        return Ok(values);
    }
    values
        .try_reserve_exact(target)
        .map_err(|_| WorkspaceError::ResourceLimit {
            limit: "Point Set identity batch allocation",
            required: allocation_bytes::<T>(target),
            allowed: limits.max_batch_bytes(),
        })?;
    let actual = allocation_bytes::<T>(values.capacity());
    if actual > limits.max_batch_bytes() {
        return Err(WorkspaceError::ResourceLimit {
            limit: "Point Set identity batch bytes",
            required: actual,
            allowed: limits.max_batch_bytes(),
        });
    }
    require_working(actual, read_buffer_bytes, limits)?;
    Ok(values)
}

fn require_working(
    output_bytes: u64,
    read_buffer_bytes: u64,
    limits: PointIdReadLimits,
) -> Result<(), WorkspaceError> {
    if read_buffer_bytes > limits.max_read_buffer_bytes() {
        return Err(WorkspaceError::ResourceLimit {
            limit: "Point Set spill read buffer",
            required: read_buffer_bytes,
            allowed: limits.max_read_buffer_bytes(),
        });
    }
    let required = output_bytes.saturating_add(read_buffer_bytes);
    if required > limits.max_working_bytes() {
        return Err(WorkspaceError::ResourceLimit {
            limit: "Point Set identity read working bytes",
            required,
            allowed: limits.max_working_bytes(),
        });
    }
    Ok(())
}

fn require_selection_working(
    builder_bytes: u64,
    other_working_bytes: u64,
    allowed: u64,
) -> Result<(), WorkspaceError> {
    let required = builder_bytes.saturating_add(other_working_bytes);
    if required > allowed {
        return Err(WorkspaceError::ResourceLimit {
            limit: "Point Set selection working bytes",
            required,
            allowed,
        });
    }
    Ok(())
}

fn frame_capacity(max_working_bytes: u64) -> Result<usize, WorkspaceError> {
    let record_bytes = u64::try_from(mem::size_of::<PointSetRecord>()).unwrap_or(u64::MAX);
    let affordable = max_working_bytes / record_bytes.max(1);
    if affordable == 0 {
        return Err(WorkspaceError::ResourceLimit {
            limit: "Point Set spill write buffer",
            required: record_bytes,
            allowed: max_working_bytes,
        });
    }
    let default_records = u64::try_from(DEFAULT_FRAME_RECORDS).unwrap_or(u64::MAX);
    usize::try_from(affordable.min(default_records)).map_err(|_| WorkspaceError::ResourceLimit {
        limit: "Point Set spill write buffer",
        required: affordable,
        allowed: u64::try_from(usize::MAX).unwrap_or(u64::MAX),
    })
}

fn create_spill_file(scratch: &Path) -> Result<(PathBuf, File), WorkspaceError> {
    for _ in 0..RANDOM_NAME_ATTEMPTS {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(WorkspaceError::random)?;
        let name = format!("point-set-{}.pset", hex(&random));
        let path = scratch.join(name);
        match OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(WorkspaceError::io(
                    "create Point Set spill",
                    path.display(),
                    error,
                ));
            }
        }
    }
    Err(WorkspaceError::invalid_point_set(
        "could not create a unique Point Set spill name",
    ))
}

fn point_id_hasher(provenance: &SnapshotProvenance) -> Hasher {
    let mut hasher = domain_hasher(POINT_ID_HASH_DOMAIN);
    hasher.update(provenance.source().as_bytes());
    hasher
}

fn content_hasher(provenance: &SnapshotProvenance) -> Hasher {
    let mut hasher = domain_hasher(CONTENT_HASH_DOMAIN);
    hasher.update(provenance.workspace().as_bytes());
    hasher.update(provenance.source().as_bytes());
    hasher.update(provenance.revision().as_bytes());
    hasher
}

fn domain_hasher(domain: &[u8]) -> Hasher {
    let mut hasher = Hasher::new();
    hasher.update(domain);
    hasher
}

fn update_hashes(point_ids: &mut Hasher, content: &mut Hasher, record: PointSetRecord) {
    let ordinal = record.ordinal.to_le_bytes();
    point_ids.update(&ordinal);
    content.update(&ordinal);
    content.update(&[record.effective_classification]);
}

fn encode_record(record: PointSetRecord) -> [u8; 9] {
    let mut bytes = [0_u8; 9];
    bytes[..8].copy_from_slice(&record.ordinal.to_le_bytes());
    bytes[8] = record.effective_classification;
    bytes
}

fn decode_record(bytes: [u8; 9]) -> PointSetRecord {
    let mut ordinal = [0_u8; 8];
    ordinal.copy_from_slice(&bytes[..8]);
    PointSetRecord {
        ordinal: u64::from_le_bytes(ordinal),
        effective_classification: bytes[8],
    }
}

fn resident_record_bytes(records: usize) -> u64 {
    batch_record_bytes(records)
}

fn batch_record_bytes(records: usize) -> u64 {
    allocation_bytes::<PointSetRecord>(records)
}

fn allocation_bytes<T>(values: usize) -> u64 {
    u64::try_from(values)
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(mem::size_of::<T>()).unwrap_or(u64::MAX))
}

fn checked_add_with_limit(
    current: u64,
    added: u64,
    limit: &'static str,
    allowed: u64,
) -> Result<u64, WorkspaceError> {
    let required = current.saturating_add(added);
    if required > allowed {
        return Err(WorkspaceError::ResourceLimit {
            limit,
            required,
            allowed,
        });
    }
    Ok(required)
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for &byte in bytes {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, OpenOptions},
        io::{Seek, SeekFrom, Write},
    };

    use point_contracts::{ContentHash, SourceId};

    use super::{
        PointSetRecord, SpillReader, SpillWriter, content_hasher, decode_record, encode_record,
        grow_resident_records, hex, point_id_hasher, resident_record_bytes, update_hashes,
    };
    use crate::{PointIdReadLimits, PointSetMetadata, RevisionId, SnapshotProvenance, WorkspaceId};

    #[test]
    fn record_encoding_is_exact_and_round_trips() {
        let record = PointSetRecord {
            ordinal: 0x0102_0304_0506_0708,
            effective_classification: 42,
        };

        let encoded = encode_record(record);

        assert_eq!(
            encoded,
            [0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 42]
        );
        assert_eq!(decode_record(encoded), record);
    }

    #[test]
    fn spill_names_use_lowercase_fixed_width_hex() {
        assert_eq!(hex(&[0, 1, 0xab, 0xff]), "0001abff");
    }

    #[test]
    fn resident_storage_grows_geometrically_within_its_preflighted_ceiling() {
        const RECORDS: usize = 100_000;
        const AVAILABLE_BYTES: u64 = 4 * 1024 * 1024;

        let mut records = Vec::new();
        let mut growths = 0_u32;
        for ordinal in 0..RECORDS {
            if records.len() == records.capacity() {
                assert!(grow_resident_records(&mut records, ordinal + 1, AVAILABLE_BYTES).unwrap());
                growths += 1;
            }
            records.push(PointSetRecord {
                ordinal: u64::try_from(ordinal).unwrap(),
                effective_classification: 2,
            });
        }

        assert!(growths <= 8, "resident growth count was {growths}");
        assert!(resident_record_bytes(records.capacity()) <= AVAILABLE_BYTES);
    }

    #[test]
    fn changed_frame_never_emits_an_unverified_record() {
        let provenance = SnapshotProvenance::new(
            WorkspaceId::from_bytes([1; 16]).unwrap(),
            SourceId::new([2; 32]),
            RevisionId::from_bytes([3; 32]).unwrap(),
        );
        let record = PointSetRecord {
            ordinal: 8,
            effective_classification: 4,
        };
        let mut point_ids = point_id_hasher(&provenance);
        let mut content = content_hasher(&provenance);
        update_hashes(&mut point_ids, &mut content, record);
        let metadata = PointSetMetadata::new(
            provenance,
            1,
            ContentHash::new(*point_ids.finalize().as_bytes()),
            ContentHash::new(*content.finalize().as_bytes()),
        );
        let mut writer = SpillWriter::create(&std::env::temp_dir(), 4_096, 4_096).unwrap();
        writer.allocate_buffer().unwrap();
        writer.push(record).unwrap();
        let mut descriptor = writer.finish(&metadata).unwrap();

        let mut file = OpenOptions::new()
            .write(true)
            .open(&descriptor.path)
            .unwrap();
        file.seek(SeekFrom::Start(17)).unwrap();
        file.write_all(&[9]).unwrap();
        file.flush().unwrap();
        drop(file);
        descriptor.modified = fs::metadata(&descriptor.path).unwrap().modified().unwrap();

        let limits = PointIdReadLimits::default();
        let mut reader = SpillReader::open(&descriptor, &metadata, limits).unwrap();
        let error = reader.next_record(0, limits).unwrap_err();

        assert!(matches!(
            error,
            crate::WorkspaceError::InvalidPointSet { .. }
        ));
        fs::remove_file(descriptor.path).unwrap();
    }

    #[test]
    fn spill_frame_cannot_grow_beyond_its_sealed_maximum() {
        let provenance = SnapshotProvenance::new(
            WorkspaceId::from_bytes([4; 16]).unwrap(),
            SourceId::new([5; 32]),
            RevisionId::from_bytes([6; 32]).unwrap(),
        );
        let record = PointSetRecord {
            ordinal: 1,
            effective_classification: 2,
        };
        let mut point_ids = point_id_hasher(&provenance);
        let mut content = content_hasher(&provenance);
        update_hashes(&mut point_ids, &mut content, record);
        let sealed_metadata = PointSetMetadata::new(
            provenance,
            1,
            ContentHash::new(*point_ids.finalize().as_bytes()),
            ContentHash::new(*content.finalize().as_bytes()),
        );
        let mut writer = SpillWriter::create(&std::env::temp_dir(), 4_096, 4_096).unwrap();
        writer.allocate_buffer().unwrap();
        writer.push(record).unwrap();
        let mut descriptor = writer.finish(&sealed_metadata).unwrap();
        assert_eq!(descriptor.max_frame_records, 1);

        let mut file = OpenOptions::new()
            .write(true)
            .open(&descriptor.path)
            .unwrap();
        file.seek(SeekFrom::Start(13)).unwrap();
        file.write_all(&2_u32.to_le_bytes()).unwrap();
        file.flush().unwrap();
        drop(file);
        descriptor.modified = fs::metadata(&descriptor.path).unwrap().modified().unwrap();

        let forged_metadata = PointSetMetadata::new(
            provenance,
            2,
            sealed_metadata.point_id_hash(),
            sealed_metadata.content_hash(),
        );
        let limits = PointIdReadLimits::default();
        let mut reader = SpillReader::open(&descriptor, &forged_metadata, limits).unwrap();
        let error = reader.next_record(0, limits).unwrap_err();

        assert!(matches!(
            error,
            crate::WorkspaceError::InvalidPointSet { .. }
        ));
        fs::remove_file(descriptor.path).unwrap();
    }
}
