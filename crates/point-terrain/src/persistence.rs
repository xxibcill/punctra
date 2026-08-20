use std::{
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    mem,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[cfg(test)]
use std::cell::{Cell, RefCell};

#[cfg(not(unix))]
use std::fs::OpenOptions;

use blake3::Hasher;
use foundation_runtime::OperationControl;
use point_contracts::{
    ContentHash, CoordinateReference, LinearUnit, PointId, PositionTransform, SourceId,
    SpatialAxes, SpatialReferenceProfile, SpatialReferenceProvenance, WorldBounds,
};
use point_workspace::{PointQuery, Snapshot, SnapshotProvenance};

use crate::{
    SurfaceFace, SurfaceFaceId, SurfaceReadLimits, SurfaceVertex, SurfaceVertexId, TerrainError,
    TerrainPrepareLimits, TerrainRecipe, TerrainSurface,
    derive::{
        CollectedTerrainInput, GEOMETRY_HASH_DOMAIN, InputVertex, TOPOLOGY_HASH_DOMAIN,
        artifact_hash, canonical_f64_bits, canonical_topology_hash, collect_input,
        derive_collected, domain_hasher, hash_transform, recipe_hash,
    },
};

/// Fixed complete Surface artifact disk contract supported by this crate.
pub const SURFACE_DISK_VERSION: u32 = 1;
const WORK_DISK_VERSION: u32 = 1;

const ARTIFACT_MAGIC: &[u8; 8] = b"PTRNSF1\0";
const WORK_MAGIC: &[u8; 8] = b"PTRNWK1\0";
const ARTIFACT_KIND: &str = "Surface artifact";
const WORK_KIND: &str = "Surface work checkpoint";
const ARTIFACT_HEADER_BYTES: u64 = 576;
const WORK_HEADER_BYTES: u64 = 352;
const CHECKSUM_BYTES: u64 = 32;
const VERTEX_RECORD_BYTES: u64 = 32;
const FACE_RECORD_BYTES: u64 = 12;
const MAX_SURFACE_RECORD_BYTES: usize = 32;
const RECORDS_PER_BLOCK: u64 = 4_096;
const RECORDS_PER_BLOCK_USIZE: usize = 4_096;
const CHECKSUM_DOMAIN: &[u8] = b"punctra-terrain-disk-v1";
const WORK_CHECKSUM_DOMAIN: &[u8] = b"punctra-terrain-work-v1";
const VERTEX_BLOCK_DOMAIN: &[u8] = b"punctra-terrain-vertex-block-v1";
const FACE_BLOCK_DOMAIN: &[u8] = b"punctra-terrain-face-block-v1";
const WORK_BLOCK_DOMAIN: &[u8] = b"punctra-terrain-work-block-v1";
const SNAPSHOT_ROWS_HASH_DOMAIN: &[u8] = b"punctra-snapshot-point-rows-v1";
const MIN_VERIFY_BUFFER_BYTES: u64 = 32;
const MAX_EXACT_F64_INTEGER: i128 = 1_i128 << 53;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PersistedKind {
    Artifact,
    Work,
}

impl PersistedKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Artifact => ARTIFACT_KIND,
            Self::Work => WORK_KIND,
        }
    }

    const fn magic(self) -> [u8; 8] {
        match self {
            Self::Artifact => *ARTIFACT_MAGIC,
            Self::Work => *WORK_MAGIC,
        }
    }

    const fn version(self) -> u32 {
        match self {
            Self::Artifact => SURFACE_DISK_VERSION,
            Self::Work => WORK_DISK_VERSION,
        }
    }

    const fn checksum_domain(self) -> &'static [u8] {
        match self {
            Self::Artifact => CHECKSUM_DOMAIN,
            Self::Work => WORK_CHECKSUM_DOMAIN,
        }
    }

    const fn header_bytes(self) -> u64 {
        match self {
            Self::Artifact => ARTIFACT_HEADER_BYTES,
            Self::Work => WORK_HEADER_BYTES,
        }
    }

    const fn max_file_bytes(self, limits: TerrainPrepareLimits) -> u64 {
        match self {
            Self::Artifact => limits.max_artifact_bytes(),
            Self::Work => limits.max_work_bytes(),
        }
    }

    const fn resource_limit(self) -> &'static str {
        match self {
            Self::Artifact => "Surface artifact bytes",
            Self::Work => "Surface work checkpoint bytes",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PersistenceBoundary {
    WorkCreate,
    WorkWrite,
    WorkFileSync,
    WorkPublish,
    WorkParentSync,
    WorkReadback,
    StageCreate,
    StageWrite,
    StageFileSync,
    StagePublish,
    StageParentSync,
    StageReadback,
    TargetLink,
    TargetIdentity,
    TargetParentSync,
    TargetReadback,
    TargetRevalidation,
    WarmParentSync,
    CancelWarmOpen,
    CancelAfterWork,
    CancelAfterStage,
    CancelAfterTargetLink,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StreamReadBoundary {
    VertexRecordCaptured,
    FaceRecordCaptured,
}

#[cfg(test)]
type StreamMutationHook = Box<dyn FnOnce()>;
#[cfg(test)]
type InjectedStreamMutation = Option<(StreamReadBoundary, StreamMutationHook)>;
#[cfg(test)]
type PublicationRaceHook = Box<dyn FnOnce()>;
#[cfg(test)]
type OpenRaceHook = Box<dyn FnOnce()>;

#[cfg(test)]
thread_local! {
    static INJECTED_IO_FAULT: RefCell<Option<(PersistenceBoundary, io::ErrorKind)>> =
        const { RefCell::new(None) };
    static INJECTED_CANCELLATION: Cell<Option<PersistenceBoundary>> = const { Cell::new(None) };
    static INJECTED_STREAM_MUTATION: RefCell<InjectedStreamMutation> =
        const { RefCell::new(None) };
    static INJECTED_PUBLICATION_RACE: RefCell<Option<PublicationRaceHook>> =
        const { RefCell::new(None) };
    static INJECTED_OPEN_RACE: RefCell<Option<OpenRaceHook>> = const { RefCell::new(None) };
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "production is infallible while private unit tests inject exact I/O boundaries"
)]
fn maybe_injected_io(boundary: PersistenceBoundary) -> io::Result<()> {
    #[cfg(test)]
    {
        INJECTED_IO_FAULT.with(|slot| {
            let mut slot = slot.borrow_mut();
            match *slot {
                Some((expected, kind)) if expected == boundary => {
                    *slot = None;
                    Err(io::Error::new(
                        kind,
                        format!("injected durable boundary fault at {boundary:?}"),
                    ))
                }
                _ => Ok(()),
            }
        })
    }
    #[cfg(not(test))]
    {
        let _ = boundary;
        Ok(())
    }
}

fn maybe_injected_cancellation(boundary: PersistenceBoundary, control: &OperationControl) {
    #[cfg(test)]
    if INJECTED_CANCELLATION.with(|slot| {
        if slot.get() == Some(boundary) {
            slot.set(None);
            true
        } else {
            false
        }
    }) {
        control.cancel();
    }
    #[cfg(not(test))]
    {
        let _ = (boundary, control);
    }
}

fn maybe_injected_stream_mutation(boundary: StreamReadBoundary) {
    #[cfg(test)]
    let hook = INJECTED_STREAM_MUTATION.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot
            .as_ref()
            .is_some_and(|(expected, _)| *expected == boundary)
        {
            slot.take().map(|(_, hook)| hook)
        } else {
            None
        }
    });
    #[cfg(test)]
    if let Some(hook) = hook {
        hook();
    }
    #[cfg(not(test))]
    {
        let _ = boundary;
    }
}

fn maybe_injected_open_race() {
    #[cfg(test)]
    let hook = INJECTED_OPEN_RACE.with(|slot| slot.borrow_mut().take());
    #[cfg(test)]
    if let Some(hook) = hook {
        hook();
    }
}

fn maybe_injected_publication_race() {
    #[cfg(test)]
    if let Some(hook) = INJECTED_PUBLICATION_RACE.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}

/// How one preparation attempt obtained its immutable Surface artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerrainPrepareDisposition {
    /// A matching complete target was verified and opened.
    Opened,
    /// Input was collected, triangulated, and published in this attempt.
    Built,
    /// Verified staged input was reused before triangulation and publication.
    ResumedInput,
    /// A verified complete staging artifact was published without retriangulation.
    ResumedPublication,
}

/// Non-semantic observations from one durable Surface preparation attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerrainPrepareReport {
    disposition: TerrainPrepareDisposition,
    artifact_bytes: u64,
    reused_input_points: u64,
    source_points_read: u64,
    peak_temporary_disk_bytes: u64,
    accounted_handle_bytes: u64,
    accounted_peak_working_bytes: Option<u64>,
    topology_steps: Option<u64>,
}

impl TerrainPrepareReport {
    #[allow(clippy::too_many_arguments)]
    const fn new(
        disposition: TerrainPrepareDisposition,
        artifact_bytes: u64,
        reused_input_points: u64,
        source_points_read: u64,
        peak_temporary_disk_bytes: u64,
        accounted_handle_bytes: u64,
        accounted_peak_working_bytes: Option<u64>,
        topology_steps: Option<u64>,
    ) -> Self {
        Self {
            disposition,
            artifact_bytes,
            reused_input_points,
            source_points_read,
            peak_temporary_disk_bytes,
            accounted_handle_bytes,
            accounted_peak_working_bytes,
            topology_steps,
        }
    }

    /// Returns whether this attempt opened, built, or resumed durable work.
    #[must_use]
    pub const fn disposition(self) -> TerrainPrepareDisposition {
        self.disposition
    }

    /// Returns the exact complete artifact byte length.
    #[must_use]
    pub const fn artifact_bytes(self) -> u64 {
        self.artifact_bytes
    }

    /// Returns input Points read from a verified durable checkpoint.
    #[must_use]
    pub const fn reused_input_points(self) -> u64 {
        self.reused_input_points
    }

    /// Returns exact Ground Input Points read from the Snapshot in this attempt.
    #[must_use]
    pub const fn source_points_read(self) -> u64 {
        self.source_points_read
    }

    /// Returns peak logical bytes retained in owned work, staging, and private
    /// publication-copy files.
    #[must_use]
    pub const fn peak_temporary_disk_bytes(self) -> u64 {
        self.peak_temporary_disk_bytes
    }

    /// Returns bytes accounted to the retained file handle and its metadata.
    #[must_use]
    pub const fn accounted_handle_bytes(self) -> u64 {
        self.accounted_handle_bytes
    }

    /// Returns the derivation working-byte observation when derivation ran.
    #[must_use]
    pub const fn accounted_peak_working_bytes(self) -> Option<u64> {
        self.accounted_peak_working_bytes
    }

    /// Returns triangulation primitive operations when triangulation ran.
    #[must_use]
    pub const fn topology_steps(self) -> Option<u64> {
        self.topology_steps
    }
}

#[derive(Clone, Copy)]
struct AttemptObservations {
    disposition: TerrainPrepareDisposition,
    reused_input_points: u64,
    source_points_read: u64,
    peak_temporary_disk_bytes: u64,
    accounted_peak_working_bytes: Option<u64>,
    topology_steps: Option<u64>,
}

impl AttemptObservations {
    const fn without_derivation(disposition: TerrainPrepareDisposition) -> Self {
        Self {
            disposition,
            reused_input_points: 0,
            source_points_read: 0,
            peak_temporary_disk_bytes: 0,
            accounted_peak_working_bytes: None,
            topology_steps: None,
        }
    }

    const fn from_report(report: TerrainPrepareReport) -> Self {
        Self {
            disposition: report.disposition,
            reused_input_points: report.reused_input_points,
            source_points_read: report.source_points_read,
            peak_temporary_disk_bytes: report.peak_temporary_disk_bytes,
            accounted_peak_working_bytes: report.accounted_peak_working_bytes,
            topology_steps: report.topology_steps,
        }
    }

    const fn with_disposition(mut self, disposition: TerrainPrepareDisposition) -> Self {
        self.disposition = disposition;
        self
    }

    const fn with_peak_temporary_disk_bytes(mut self, bytes: u64) -> Self {
        self.peak_temporary_disk_bytes = bytes;
        self
    }
}

/// Immutable semantic facts bound into one disk-v1 Surface artifact.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceArtifactDescriptor {
    snapshot: SnapshotProvenance,
    recipe: TerrainRecipe,
    recipe_hash: ContentHash,
    transform: PositionTransform,
    coordinate_reference: CoordinateReference,
    input_hash: ContentHash,
    geometry_hash: ContentHash,
    topology_hash: ContentHash,
    artifact_hash: ContentHash,
    input_point_count: u64,
    vertex_count: u64,
    face_count: u64,
    hull_vertex_count: u64,
    bounds: WorldBounds,
}

impl SurfaceArtifactDescriptor {
    fn from_surface(surface: &TerrainSurface) -> Self {
        let descriptor = surface.descriptor();
        Self {
            snapshot: descriptor.snapshot(),
            recipe: descriptor.recipe(),
            recipe_hash: descriptor.recipe_hash(),
            transform: descriptor.position_transform(),
            coordinate_reference: descriptor.coordinate_reference().clone(),
            input_hash: descriptor.input_hash(),
            geometry_hash: descriptor.geometry_hash(),
            topology_hash: descriptor.topology_hash(),
            artifact_hash: descriptor.artifact_hash(),
            input_point_count: descriptor.input_point_count(),
            vertex_count: descriptor.vertex_count(),
            face_count: descriptor.face_count(),
            hull_vertex_count: descriptor.hull_vertex_count(),
            bounds: descriptor.bounds(),
        }
    }

    /// Returns the exact immutable Snapshot identity chain.
    #[must_use]
    pub const fn snapshot(&self) -> SnapshotProvenance {
        self.snapshot
    }

    /// Returns the explicit-AOI terrain recipe.
    #[must_use]
    pub const fn recipe(&self) -> TerrainRecipe {
        self.recipe
    }

    /// Returns the canonical recipe digest.
    #[must_use]
    pub const fn recipe_hash(&self) -> ContentHash {
        self.recipe_hash
    }

    /// Returns the deterministic terrain algorithm contract version.
    #[must_use]
    pub const fn algorithm_version(&self) -> u32 {
        crate::ALGORITHM_VERSION
    }

    /// Returns the exact Source position transform.
    #[must_use]
    pub const fn position_transform(&self) -> PositionTransform {
        self.transform
    }

    /// Returns the Source-declared structured coordinate reference.
    #[must_use]
    pub const fn coordinate_reference(&self) -> &CoordinateReference {
        &self.coordinate_reference
    }

    /// Returns the complete Snapshot Point-row content digest.
    #[must_use]
    pub const fn input_hash(&self) -> ContentHash {
        self.input_hash
    }

    /// Returns the canonical vertex-and-face geometry digest.
    #[must_use]
    pub const fn geometry_hash(&self) -> ContentHash {
        self.geometry_hash
    }

    /// Returns the canonical face-connectivity digest.
    #[must_use]
    pub const fn topology_hash(&self) -> ContentHash {
        self.topology_hash
    }

    /// Returns the provenance-sensitive Surface artifact digest.
    #[must_use]
    pub const fn artifact_hash(&self) -> ContentHash {
        self.artifact_hash
    }

    /// Returns exact Ground Input rows.
    #[must_use]
    pub const fn input_point_count(&self) -> u64 {
        self.input_point_count
    }

    /// Returns canonical Surface vertices.
    #[must_use]
    pub const fn vertex_count(&self) -> u64 {
        self.vertex_count
    }

    /// Returns canonical Surface faces.
    #[must_use]
    pub const fn face_count(&self) -> u64 {
        self.face_count
    }

    /// Returns convex-hull vertices.
    #[must_use]
    pub const fn hull_vertex_count(&self) -> u64 {
        self.hull_vertex_count
    }

    /// Returns inclusive three-dimensional geometry bounds.
    #[must_use]
    pub const fn bounds(&self) -> WorldBounds {
        self.bounds
    }
}

struct PreparedSurfaceData {
    path: PathBuf,
    file: Arc<Mutex<File>>,
    opened_metadata: fs::Metadata,
    complete_checksum: ContentHash,
    descriptor: SurfaceArtifactDescriptor,
    report: TerrainPrepareReport,
    vertex_offset: u64,
    face_offset: u64,
    vertex_directory_offset: u64,
    face_directory_offset: u64,
    block_checksums: Arc<VerifiedBlockChecksums>,
}

struct VerifiedBlockChecksums {
    vertices: Vec<[u8; 32]>,
    faces: Vec<[u8; 32]>,
}

/// Immutable file-backed disk-v1 Terrain Surface.
#[derive(Clone)]
pub struct PreparedTerrainSurface {
    inner: Arc<PreparedSurfaceData>,
}

impl PreparedTerrainSurface {
    /// Returns immutable semantic provenance and topology facts.
    #[must_use]
    pub fn descriptor(&self) -> &SurfaceArtifactDescriptor {
        &self.inner.descriptor
    }

    /// Returns observations from the attempt that opened this handle.
    #[must_use]
    pub fn report(&self) -> TerrainPrepareReport {
        self.inner.report
    }

    /// Returns the bound artifact path used by subsequent streams.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    fn complete_checksum(&self) -> ContentHash {
        self.inner.complete_checksum
    }

    fn verify_path_binding(&self, parent: &DirectoryWitness) -> Result<(), TerrainError> {
        parent.verify()?;
        let file = self.inner.file.lock().map_err(|_| {
            TerrainError::corrupt_surface(
                ARTIFACT_KIND,
                self.inner.path.display(),
                "verified artifact reader lock was poisoned",
            )
        })?;
        verify_opened_binding(
            &file,
            &self.inner.opened_metadata,
            &self.inner.path,
            ARTIFACT_KIND,
        )?;
        parent.verify()
    }

    fn publish_open_file(
        &self,
        parent: &DirectoryWitness,
        target: &Path,
        limits: TerrainPrepareLimits,
        control: &OperationControl,
    ) -> Result<(), DescriptorPublicationError> {
        let file = self.inner.file.lock().map_err(|_| {
            TerrainError::corrupt_surface(
                ARTIFACT_KIND,
                self.inner.path.display(),
                "verified artifact reader lock was poisoned",
            )
        })?;
        parent.publish_open_file(&file, target, self.complete_checksum(), limits, control)
    }

    fn set_peak_temporary_disk_bytes(&mut self, bytes: u64) {
        let inner = Arc::get_mut(&mut self.inner)
            .expect("a newly opened staged Surface has one owning handle");
        inner.report.peak_temporary_disk_bytes = bytes;
    }

    /// Opens a bounded pull stream of canonical vertices.
    ///
    /// # Errors
    ///
    /// Returns an error when the requested batch cannot fit its limits or the
    /// verified artifact reader is unavailable.
    pub fn vertex_batches(
        &self,
        limits: SurfaceReadLimits,
    ) -> Result<SurfaceVertexBatches, TerrainError> {
        Ok(SurfaceVertexBatches {
            stream: SurfaceBatchStream::new::<SurfaceVertex>(
                &self.inner,
                limits,
                StreamRecordKind::Vertex,
            )?,
            source: self.inner.descriptor.snapshot.source(),
        })
    }

    /// Opens a bounded pull stream of canonical faces.
    ///
    /// # Errors
    ///
    /// Returns an error when the requested batch cannot fit its limits or the
    /// verified artifact reader is unavailable.
    pub fn face_batches(
        &self,
        limits: SurfaceReadLimits,
    ) -> Result<SurfaceFaceBatches, TerrainError> {
        Ok(SurfaceFaceBatches {
            stream: SurfaceBatchStream::new::<SurfaceFace>(
                &self.inner,
                limits,
                StreamRecordKind::Face,
            )?,
            vertex_count: self.inner.descriptor.vertex_count,
        })
    }
}

impl std::fmt::Debug for PreparedTerrainSurface {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedTerrainSurface")
            .field("path", &self.inner.path)
            .field("descriptor", &self.inner.descriptor)
            .field("report", &self.inner.report)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy)]
enum StreamRecordKind {
    Vertex,
    Face,
}

impl StreamRecordKind {
    fn section(self, surface: &PreparedSurfaceData) -> SurfaceSectionSpec<'_> {
        match self {
            Self::Vertex => SurfaceSectionSpec {
                label: "Surface vertices",
                batch_bytes_label: "Surface vertex batch bytes",
                record_bytes: VERTEX_RECORD_BYTES,
                record_count: surface.descriptor.vertex_count,
                record_offset: surface.vertex_offset,
                directory_offset: surface.vertex_directory_offset,
                domain: VERTEX_BLOCK_DOMAIN,
                boundary: StreamReadBoundary::VertexRecordCaptured,
                checksums: &surface.block_checksums.vertices,
            },
            Self::Face => SurfaceSectionSpec {
                label: "Surface faces",
                batch_bytes_label: "Surface face batch bytes",
                record_bytes: FACE_RECORD_BYTES,
                record_count: surface.descriptor.face_count,
                record_offset: surface.face_offset,
                directory_offset: surface.face_directory_offset,
                domain: FACE_BLOCK_DOMAIN,
                boundary: StreamReadBoundary::FaceRecordCaptured,
                checksums: &surface.block_checksums.faces,
            },
        }
    }
}

struct SurfaceSectionSpec<'a> {
    label: &'static str,
    batch_bytes_label: &'static str,
    record_bytes: u64,
    record_count: u64,
    record_offset: u64,
    directory_offset: u64,
    domain: &'static [u8],
    boundary: StreamReadBoundary,
    checksums: &'a [[u8; 32]],
}

struct SurfaceBatchStream {
    surface: Arc<PreparedSurfaceData>,
    kind: StreamRecordKind,
    next_id: u64,
    remaining: u64,
    verify_buffer: Vec<u8>,
    batch_records: u64,
    max_batch_payload_bytes: u64,
    max_working_bytes: u64,
    max_work_units: u64,
    used_work_units: u64,
}

impl SurfaceBatchStream {
    fn new<T>(
        surface: &Arc<PreparedSurfaceData>,
        limits: SurfaceReadLimits,
        kind: StreamRecordKind,
    ) -> Result<Self, TerrainError> {
        let section = kind.section(surface);
        let plan = stream_plan::<T>(
            limits,
            section.label,
            section.record_bytes,
            section.record_count,
        )?;
        Ok(Self {
            surface: Arc::clone(surface),
            kind,
            next_id: 1,
            remaining: section.record_count,
            verify_buffer: plan.verify_buffer,
            batch_records: plan.batch_records,
            max_batch_payload_bytes: limits.max_batch_payload_bytes(),
            max_working_bytes: limits.max_working_bytes(),
            max_work_units: limits.max_work_units(),
            used_work_units: 0,
        })
    }

    fn next_batch<T>(
        &mut self,
        decode: impl FnMut(u64, &[u8], &Path) -> Result<T, TerrainError>,
    ) -> Option<Result<Vec<T>, TerrainError>> {
        if self.remaining == 0 {
            return None;
        }
        let count = self.remaining.min(self.batch_records);
        let result = read_surface_batch(self, count, decode);
        if result.is_err() {
            self.remaining = 0;
        }
        Some(result)
    }
}

/// Bounded pull stream of canonical file-backed Surface vertices.
pub struct SurfaceVertexBatches {
    stream: SurfaceBatchStream,
    source: SourceId,
}

impl Iterator for SurfaceVertexBatches {
    type Item = Result<Vec<SurfaceVertex>, TerrainError>;

    fn next(&mut self) -> Option<Self::Item> {
        let source = self.source;
        self.stream.next_batch(move |record_index, bytes, path| {
            decode_vertex_record(source, record_index, bytes, path)
        })
    }
}

/// Bounded pull stream of canonical file-backed Surface faces.
pub struct SurfaceFaceBatches {
    stream: SurfaceBatchStream,
    vertex_count: u64,
}

impl Iterator for SurfaceFaceBatches {
    type Item = Result<Vec<SurfaceFace>, TerrainError>;

    fn next(&mut self) -> Option<Self::Item> {
        let vertex_count = self.vertex_count;
        self.stream.next_batch(move |record_index, bytes, path| {
            decode_face_record(vertex_count, record_index, bytes, path)
        })
    }
}

/// Starts durable explicit-AOI Surface preparation at one no-replace target.
#[must_use]
pub fn prepare(
    snapshot: Snapshot,
    target: impl AsRef<Path>,
    recipe: TerrainRecipe,
    limits: TerrainPrepareLimits,
) -> crate::TerrainPrepareJob {
    let target = owned_target_path(target.as_ref(), limits);
    crate::TerrainPrepareJob::spawn(move |control| {
        let target = target?;
        run_prepare(&snapshot, &target, recipe, limits, &control)
    })
}

fn owned_target_path(target: &Path, limits: TerrainPrepareLimits) -> Result<PathBuf, TerrainError> {
    require_target_family_paths_within(target, limits)?;
    Ok(target.to_path_buf())
}

fn require_target_family_paths_within(
    target: &Path,
    limits: TerrainPrepareLimits,
) -> Result<(), TerrainError> {
    if target.file_name().is_none() {
        return Err(TerrainError::invalid(
            "Surface target",
            "target must have a file name",
        ));
    }
    require_path_within(target, limits)?;
    for suffix in [".surface-stage-v1", ".surface-work-v1"] {
        let required = path_encoded_bytes(target)
            .saturating_add(u64::try_from(suffix.len()).unwrap_or(u64::MAX));
        require_within(
            "Surface retained path bytes",
            required,
            limits.max_path_bytes(),
        )?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct ExpectedBinding {
    snapshot: SnapshotProvenance,
    recipe: TerrainRecipe,
    recipe_hash: ContentHash,
    transform: PositionTransform,
    profile: SpatialReferenceProfile,
}

impl ExpectedBinding {
    fn new(
        snapshot: &Snapshot,
        recipe: TerrainRecipe,
        limits: TerrainPrepareLimits,
    ) -> Result<Self, TerrainError> {
        let bounds = recipe.bounds().ok_or_else(|| {
            TerrainError::invalid(
                "persistent Terrain Recipe bounds",
                "durable Surface preparation requires one explicit inclusive AOI",
            )
        })?;
        let query = PointQuery::within(bounds).classification_is(recipe.ground_classification());
        let rows = snapshot.point_rows(query, limits.derivation().point_rows())?;
        let transform = rows.source_metadata().position_transform();
        let profile = supported_profile(rows.source_metadata().coordinate_reference())?;
        Ok(Self {
            snapshot: *snapshot.provenance(),
            recipe,
            recipe_hash: recipe_hash(recipe),
            transform,
            profile,
        })
    }
}

#[allow(clippy::too_many_lines)]
fn run_prepare(
    snapshot: &Snapshot,
    target: &Path,
    recipe: TerrainRecipe,
    limits: TerrainPrepareLimits,
    control: &OperationControl,
) -> Result<PreparedTerrainSurface, TerrainError> {
    control.check_cancelled()?;
    validate_prepare_limits(limits)?;
    let expected = ExpectedBinding::new(snapshot, recipe, limits)?;
    let parent = Arc::new(DirectoryWitness::capture(target)?);

    if path_exists_in(&parent, target)? {
        maybe_injected_cancellation(PersistenceBoundary::CancelWarmOpen, control);
        let opened = open_artifact(
            &parent,
            target,
            DurablePathProvenance::UnprovenTarget,
            expected,
            AttemptObservations::without_derivation(TerrainPrepareDisposition::Opened),
            limits,
            control,
        )?;
        maybe_injected_io(PersistenceBoundary::WarmParentSync).map_err(|error| {
            TerrainError::io("sync Surface parent directory", target.display(), error)
        })?;
        parent.sync()?;
        control.complete_progress(0)?;
        return Ok(opened);
    }

    let stage_path = sibling_path(target, ".surface-stage-v1")?;
    let work_path = sibling_path(target, ".surface-work-v1")?;
    if path_exists_in(&parent, &stage_path)? {
        maybe_injected_io(PersistenceBoundary::StageReadback).map_err(|error| {
            TerrainError::io(
                "reopen staged Surface artifact",
                stage_path.display(),
                error,
            )
        })?;
        let mut staged = open_artifact(
            &parent,
            &stage_path,
            DurablePathProvenance::OwnerNamed,
            expected,
            AttemptObservations::without_derivation(TerrainPrepareDisposition::ResumedPublication),
            limits,
            control,
        )?;
        let peak_temporary_disk_bytes = staged.report().artifact_bytes();
        require_within(
            "Surface temporary bytes",
            peak_temporary_disk_bytes,
            limits.max_temporary_bytes(),
        )?;
        staged.set_peak_temporary_disk_bytes(peak_temporary_disk_bytes);
        maybe_injected_cancellation(PersistenceBoundary::CancelAfterStage, control);
        control.check_cancelled()?;
        let result = publish_verified_stage(
            &parent,
            &staged,
            PublicationAttempt {
                target,
                expected,
                observations: AttemptObservations::from_report(staged.report()),
                work: None,
                limits,
            },
            control,
        )?;
        control.complete_progress(0)?;
        return Ok(result);
    }

    let (input, disposition, reused_input_points, work_witness) =
        if path_exists_in(&parent, &work_path)? {
            let opened = open_work(&parent, &work_path, expected, limits, control)?;
            let reused = u64::try_from(opened.input.vertices.len()).unwrap_or(u64::MAX);
            (
                opened.input,
                TerrainPrepareDisposition::ResumedInput,
                reused,
                opened.witness,
            )
        } else {
            let input = collect_input(snapshot, recipe, limits.derivation(), control)?;
            let attempt_work = input.attempt_work;
            let attempt_peak_working_bytes = input.attempt_peak_working_bytes;
            let written = write_work(Arc::clone(&parent), &work_path, &input, limits, control)?;
            drop(input);
            let mut opened = open_work(&parent, &work_path, expected, limits, control)?;
            if !written.matches_metadata(&opened.witness.metadata)? {
                return Err(TerrainError::corrupt_surface(
                    WORK_KIND,
                    work_path.display(),
                    "created work checkpoint changed before verified readback",
                ));
            }
            opened.input.attempt_work = attempt_work;
            opened.input.attempt_peak_working_bytes = opened
                .input
                .attempt_peak_working_bytes
                .max(attempt_peak_working_bytes);
            (
                opened.input,
                TerrainPrepareDisposition::Built,
                0,
                opened.witness,
            )
        };

    maybe_injected_cancellation(PersistenceBoundary::CancelAfterWork, control);
    control.check_cancelled()?;
    let derived = derive_collected(input, limits.derivation(), control)?;
    let surface = derived.surface;
    let completion_work_units = derived.work_units;
    let peak = surface.descriptor().accounted_peak_working_bytes();
    let topology_steps = surface.descriptor().topology_steps();
    let retained_temporary_bytes = work_witness.byte_len()?;
    let written_stage = write_artifact(
        Arc::clone(&parent),
        &stage_path,
        &surface,
        retained_temporary_bytes,
        limits,
        control,
    )?;
    let source_points_read = if disposition == TerrainPrepareDisposition::Built {
        surface.descriptor().input_point_count()
    } else {
        0
    };
    drop(surface);
    let peak_temporary_disk_bytes = retained_temporary_bytes
        .checked_add(written_stage.byte_len()?)
        .ok_or_else(|| {
            TerrainError::resource(
                "Surface temporary bytes",
                u64::MAX,
                limits.max_temporary_bytes(),
            )
        })?;
    control.check_cancelled()?;
    maybe_injected_io(PersistenceBoundary::StageReadback).map_err(|error| {
        TerrainError::io(
            "reopen staged Surface artifact",
            stage_path.display(),
            error,
        )
    })?;
    let observations = AttemptObservations {
        disposition,
        reused_input_points,
        source_points_read,
        peak_temporary_disk_bytes,
        accounted_peak_working_bytes: Some(peak),
        topology_steps: Some(topology_steps),
    };
    let staged = open_artifact(
        &parent,
        &stage_path,
        DurablePathProvenance::OwnerNamed,
        expected,
        observations,
        limits,
        control,
    )?;
    if !written_stage.matches_metadata(&staged.inner.opened_metadata)? {
        return Err(TerrainError::corrupt_surface(
            ARTIFACT_KIND,
            stage_path.display(),
            "created staging path changed before verification",
        ));
    }
    maybe_injected_cancellation(PersistenceBoundary::CancelAfterStage, control);
    control.check_cancelled()?;
    let result = publish_verified_stage(
        &parent,
        &staged,
        PublicationAttempt {
            target,
            expected,
            observations,
            work: Some(work_witness),
            limits,
        },
        control,
    )?;
    control.complete_progress(completion_work_units)?;
    Ok(result)
}

fn validate_prepare_limits(limits: TerrainPrepareLimits) -> Result<(), TerrainError> {
    if limits.max_verify_buffer_bytes() < MIN_VERIFY_BUFFER_BYTES {
        return Err(TerrainError::resource(
            "Surface checksum verification buffer bytes",
            MIN_VERIFY_BUFFER_BYTES,
            limits.max_verify_buffer_bytes(),
        ));
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct WorkLayout {
    record_offset: u64,
    record_bytes: u64,
    directory_offset: u64,
    directory_bytes: u64,
    block_count: u64,
    file_bytes: u64,
}

impl WorkLayout {
    fn new(record_count: u64) -> Result<Self, TerrainError> {
        let record_bytes = record_count
            .checked_mul(VERTEX_RECORD_BYTES)
            .ok_or_else(|| {
                TerrainError::resource("Surface work checkpoint bytes", u64::MAX, u64::MAX - 1)
            })?;
        let block_count = block_count(record_count);
        let directory_bytes = directory_bytes(block_count);
        let record_offset = WORK_HEADER_BYTES;
        let directory_offset = record_offset.checked_add(record_bytes).ok_or_else(|| {
            TerrainError::resource("Surface work checkpoint bytes", u64::MAX, u64::MAX - 1)
        })?;
        let file_bytes = directory_offset
            .checked_add(directory_bytes)
            .and_then(|bytes| bytes.checked_add(CHECKSUM_BYTES))
            .ok_or_else(|| {
                TerrainError::resource("Surface work checkpoint bytes", u64::MAX, u64::MAX - 1)
            })?;
        Ok(Self {
            record_offset,
            record_bytes,
            directory_offset,
            directory_bytes,
            block_count,
            file_bytes,
        })
    }
}

fn write_work(
    parent: Arc<DirectoryWitness>,
    path: &Path,
    input: &CollectedTerrainInput,
    limits: TerrainPrepareLimits,
    control: &OperationControl,
) -> Result<OwnedPathWitness, TerrainError> {
    let count = u64::try_from(input.vertices.len()).unwrap_or(u64::MAX);
    let layout = WorkLayout::new(count)?;
    require_within(
        "Surface work checkpoint bytes",
        layout.file_bytes,
        limits.max_work_bytes(),
    )?;
    require_within(
        "Surface temporary bytes",
        layout.file_bytes,
        limits.max_temporary_bytes(),
    )?;

    let header = encode_work_header(input, layout)?;
    maybe_injected_io(PersistenceBoundary::WorkCreate).map_err(|error| {
        TerrainError::io("create Surface work checkpoint", path.display(), error)
    })?;
    let mut created = create_named_checkpoint(parent, path, WORK_KIND)?;
    let write_result = write_work_file(created.file_mut(), path, &header, &input.vertices, control);
    finish_named_checkpoint(
        created,
        WORK_KIND,
        PersistenceBoundary::WorkPublish,
        PersistenceBoundary::WorkParentSync,
        write_result,
    )
}

fn write_work_file(
    file: &mut File,
    path: &Path,
    header: &[u8],
    vertices: &[InputVertex],
    control: &OperationControl,
) -> Result<(), TerrainError> {
    maybe_injected_io(PersistenceBoundary::WorkWrite)
        .map_err(|error| TerrainError::io("write durable Surface file", path.display(), error))?;
    let mut hasher = checksum_hasher(WORK_CHECKSUM_DOMAIN);
    write_hashed(file, &mut hasher, header, path)?;
    for (index, vertex) in vertices.iter().enumerate() {
        if index.is_multiple_of(RECORDS_PER_BLOCK_USIZE) {
            control.check_cancelled()?;
        }
        let bytes = encode_input_vertex(*vertex);
        write_hashed(file, &mut hasher, &bytes, path)?;
    }
    write_record_block_directory(
        file,
        &mut hasher,
        vertices,
        encode_input_vertex,
        WORK_BLOCK_DOMAIN,
        path,
        control,
    )?;
    write_all(file, hasher.finalize().as_bytes(), path)?;
    maybe_injected_io(PersistenceBoundary::WorkFileSync)
        .map_err(|error| TerrainError::io("sync durable Surface file", path.display(), error))?;
    sync_file(file, path)
}

struct OpenedWork {
    input: CollectedTerrainInput,
    witness: OwnedPathWitness,
}

#[allow(clippy::too_many_lines)]
fn open_work(
    parent: &DirectoryWitness,
    path: &Path,
    expected: ExpectedBinding,
    limits: TerrainPrepareLimits,
    control: &OperationControl,
) -> Result<OpenedWork, TerrainError> {
    maybe_injected_io(PersistenceBoundary::WorkReadback).map_err(|error| {
        TerrainError::io("reopen Surface work checkpoint", path.display(), error)
    })?;
    let mut opened = open_regular_in(parent, path, WORK_KIND, DurablePathProvenance::OwnerNamed)?;
    let verified = verified_file(
        &mut opened.file,
        path,
        PersistedKind::Work,
        limits,
        limits.max_verify_buffer_bytes(),
        control,
    )?;
    require_within(
        "Surface temporary bytes",
        verified.bytes,
        limits.max_temporary_bytes(),
    )?;
    opened
        .file
        .seek(SeekFrom::Start(0))
        .map_err(|error| TerrainError::io("seek Surface work checkpoint", path.display(), error))?;
    let header = read_exact_vec(&mut opened.file, WORK_HEADER_BYTES, path, WORK_KIND)?;
    let work = decode_work_header(&header, path, expected, verified.bytes)?;
    validate_record_blocks(
        &mut opened.file,
        RecordBlockLayout {
            record_offset: work.layout.record_offset,
            record_count: work.input_point_count,
            record_bytes: VERTEX_RECORD_BYTES,
            directory_offset: work.layout.directory_offset,
            block_count: work.layout.block_count,
            domain: WORK_BLOCK_DOMAIN,
        },
        limits.max_verify_buffer_bytes(),
        path,
        WORK_KIND,
        Some(control),
        None,
    )?;
    require_within(
        "Ground Input Points",
        work.input_point_count,
        limits.derivation().max_input_points(),
    )?;
    let retained_bytes = work
        .input_point_count
        .saturating_mul(u64::try_from(mem::size_of::<InputVertex>()).unwrap_or(u64::MAX));
    require_within(
        "Ground Input allocation bytes",
        retained_bytes,
        limits.derivation().max_working_bytes(),
    )?;
    let count = usize::try_from(work.input_point_count).map_err(|_| {
        TerrainError::resource(
            "Ground Input allocation bytes",
            retained_bytes,
            limits.derivation().max_working_bytes(),
        )
    })?;
    let mut vertices = Vec::new();
    vertices.try_reserve_exact(count).map_err(|_| {
        TerrainError::resource(
            "Ground Input allocation bytes",
            retained_bytes,
            limits.derivation().max_working_bytes(),
        )
    })?;
    let retained_bytes = u64::try_from(vertices.capacity())
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(mem::size_of::<InputVertex>()).unwrap_or(u64::MAX));
    require_within(
        "Ground Input allocation bytes",
        retained_bytes,
        limits.derivation().max_working_bytes(),
    )?;
    opened
        .file
        .seek(SeekFrom::Start(work.layout.record_offset))
        .map_err(|error| TerrainError::io("seek Surface work rows", path.display(), error))?;
    let mut previous_ordinal = None;
    let mut input_hasher = snapshot_input_hasher(expected.snapshot);
    for index in 0..work.input_point_count {
        if index.is_multiple_of(RECORDS_PER_BLOCK) {
            control.check_cancelled()?;
        }
        let bytes = read_exact_array::<32>(&mut opened.file, path, WORK_KIND)?;
        let ordinal = u64::from_le_bytes(bytes[0..8].try_into().expect("fixed slice"));
        if previous_ordinal.is_some_and(|previous| ordinal <= previous) {
            return Err(TerrainError::corrupt_surface(
                WORK_KIND,
                path.display(),
                "input ordinals are not strictly increasing",
            ));
        }
        previous_ordinal = Some(ordinal);
        let ticks = ticks_from_input_bytes(&bytes);
        validate_record_world_position(
            work.transform,
            ticks,
            expected.recipe,
            path,
            WORK_KIND,
            "input record",
        )?;
        hash_snapshot_input_record(
            &mut input_hasher,
            ordinal,
            ticks,
            expected.recipe.ground_classification(),
        );
        vertices.push(InputVertex {
            point: PointId::new(expected.snapshot.source(), ordinal),
            ticks,
        });
    }
    control.check_cancelled()?;
    if ContentHash::new(*input_hasher.finalize().as_bytes()) != work.input_hash {
        return Err(TerrainError::corrupt_surface(
            WORK_KIND,
            path.display(),
            "Snapshot Point content hash does not match staged input records",
        ));
    }
    opened.verify_binding(path, WORK_KIND)?;
    parent.verify()?;
    let input = CollectedTerrainInput {
        snapshot: expected.snapshot,
        recipe: expected.recipe,
        transform: work.transform,
        coordinate_reference: CoordinateReference::profile(work.profile),
        input_hash: work.input_hash,
        vertices,
        attempt_work: 0,
        attempt_peak_working_bytes: retained_bytes,
    };
    let witness = OwnedPathWitness::from_opened(path.to_path_buf(), opened);
    Ok(OpenedWork { input, witness })
}

fn snapshot_input_hasher(snapshot: SnapshotProvenance) -> Hasher {
    let mut hasher = Hasher::new();
    hasher.update(SNAPSHOT_ROWS_HASH_DOMAIN);
    hasher.update(snapshot.workspace().as_bytes());
    hasher.update(snapshot.source().as_bytes());
    hasher.update(snapshot.revision().as_bytes());
    hasher
}

fn hash_snapshot_input_record(
    hasher: &mut Hasher,
    ordinal: u64,
    ticks: [i64; 3],
    classification: u8,
) {
    hasher.update(&ordinal.to_le_bytes());
    for tick in ticks {
        hasher.update(&tick.to_le_bytes());
    }
    hasher.update(&[classification]);
}

fn write_artifact(
    parent: Arc<DirectoryWitness>,
    path: &Path,
    surface: &TerrainSurface,
    retained_temporary_bytes: u64,
    limits: TerrainPrepareLimits,
    control: &OperationControl,
) -> Result<OwnedPathWitness, TerrainError> {
    let descriptor = SurfaceArtifactDescriptor::from_surface(surface);
    let layout = ArtifactLayout::new(descriptor.vertex_count, descriptor.face_count)?;
    require_within(
        "Surface artifact bytes",
        layout.file_bytes,
        limits.max_artifact_bytes(),
    )?;
    let total_temporary_bytes = retained_temporary_bytes
        .checked_add(layout.file_bytes)
        .ok_or_else(|| {
            TerrainError::resource(
                "Surface temporary bytes",
                u64::MAX,
                limits.max_temporary_bytes(),
            )
        })?;
    require_within(
        "Surface temporary bytes",
        total_temporary_bytes,
        limits.max_temporary_bytes(),
    )?;
    let header = encode_artifact_header(&descriptor, layout)?;
    maybe_injected_io(PersistenceBoundary::StageCreate).map_err(|error| {
        TerrainError::io("create staged Surface artifact", path.display(), error)
    })?;
    let mut created = create_named_checkpoint(parent, path, ARTIFACT_KIND)?;
    let write_result = write_artifact_file(created.file_mut(), path, &header, surface, control);
    finish_named_checkpoint(
        created,
        ARTIFACT_KIND,
        PersistenceBoundary::StagePublish,
        PersistenceBoundary::StageParentSync,
        write_result,
    )
}

fn write_artifact_file(
    file: &mut File,
    path: &Path,
    header: &[u8],
    surface: &TerrainSurface,
    control: &OperationControl,
) -> Result<(), TerrainError> {
    maybe_injected_io(PersistenceBoundary::StageWrite)
        .map_err(|error| TerrainError::io("write durable Surface file", path.display(), error))?;
    let mut hasher = checksum_hasher(CHECKSUM_DOMAIN);
    write_hashed(file, &mut hasher, header, path)?;
    for (index, vertex) in surface.vertices().iter().enumerate() {
        if index.is_multiple_of(RECORDS_PER_BLOCK_USIZE) {
            control.check_cancelled()?;
        }
        let bytes = encode_surface_vertex(*vertex);
        write_hashed(file, &mut hasher, &bytes, path)?;
    }
    for (index, face) in surface.faces().iter().enumerate() {
        if index.is_multiple_of(RECORDS_PER_BLOCK_USIZE) {
            control.check_cancelled()?;
        }
        let bytes = encode_surface_face(*face);
        write_hashed(file, &mut hasher, &bytes, path)?;
    }
    write_record_block_directory(
        file,
        &mut hasher,
        surface.vertices(),
        encode_surface_vertex,
        VERTEX_BLOCK_DOMAIN,
        path,
        control,
    )?;
    write_record_block_directory(
        file,
        &mut hasher,
        surface.faces(),
        encode_surface_face,
        FACE_BLOCK_DOMAIN,
        path,
        control,
    )?;
    write_all(file, hasher.finalize().as_bytes(), path)?;
    maybe_injected_io(PersistenceBoundary::StageFileSync)
        .map_err(|error| TerrainError::io("sync durable Surface file", path.display(), error))?;
    sync_file(file, path)
}

fn open_artifact(
    parent: &DirectoryWitness,
    path: &Path,
    provenance: DurablePathProvenance,
    expected: ExpectedBinding,
    observations: AttemptObservations,
    limits: TerrainPrepareLimits,
    control: &OperationControl,
) -> Result<PreparedTerrainSurface, TerrainError> {
    let mut opened = open_regular_in(parent, path, ARTIFACT_KIND, provenance)?;
    require_recognized_artifact_target(&mut opened.file, path, provenance)?;
    let verified = verified_file(
        &mut opened.file,
        path,
        PersistedKind::Artifact,
        limits,
        limits.max_verify_buffer_bytes(),
        control,
    )?;
    opened
        .file
        .seek(SeekFrom::Start(0))
        .map_err(|error| TerrainError::io("seek Surface artifact", path.display(), error))?;
    let header = read_exact_vec(&mut opened.file, ARTIFACT_HEADER_BYTES, path, ARTIFACT_KIND)?;
    let decoded = decode_artifact_header(&header, path, expected, verified.bytes)?;
    require_within(
        "Ground Input Points",
        decoded.descriptor.input_point_count,
        limits.derivation().max_input_points(),
    )?;
    require_within(
        "Terrain vertices",
        decoded.descriptor.vertex_count,
        limits.derivation().max_vertices(),
    )?;
    require_within(
        "Terrain faces",
        decoded.descriptor.face_count,
        limits.derivation().max_faces(),
    )?;
    let block_checksums = validate_artifact_blocks(
        &mut opened.file,
        decoded.layout,
        &decoded.descriptor,
        limits.max_verify_buffer_bytes(),
        limits.max_retained_handle_bytes(),
        path,
        control,
    )?;
    validate_artifact_payload(
        &mut opened.file,
        path,
        &decoded.descriptor,
        decoded.layout,
        limits.derivation(),
        control,
    )?;
    control.check_cancelled()?;
    opened.verify_binding(path, ARTIFACT_KIND)?;
    parent.verify()?;
    let accounted_handle_bytes = accounted_retained_handle_bytes(
        path,
        block_checksums.vertices.capacity(),
        block_checksums.faces.capacity(),
    );
    require_within(
        "Surface retained handle bytes",
        accounted_handle_bytes,
        limits.max_retained_handle_bytes(),
    )?;
    let report = TerrainPrepareReport::new(
        observations.disposition,
        verified.bytes,
        observations.reused_input_points,
        observations.source_points_read,
        observations.peak_temporary_disk_bytes,
        accounted_handle_bytes,
        observations.accounted_peak_working_bytes,
        observations.topology_steps,
    );
    Ok(PreparedTerrainSurface {
        inner: Arc::new(PreparedSurfaceData {
            path: path.to_path_buf(),
            file: Arc::new(Mutex::new(opened.file)),
            opened_metadata: opened.metadata,
            complete_checksum: verified.checksum,
            descriptor: decoded.descriptor,
            report,
            vertex_offset: decoded.layout.vertex_offset,
            face_offset: decoded.layout.face_offset,
            vertex_directory_offset: decoded.layout.vertex_directory_offset,
            face_directory_offset: decoded.layout.face_directory_offset,
            block_checksums: Arc::new(block_checksums),
        }),
    })
}

#[derive(Clone, Copy)]
struct RecordBlockLayout {
    record_offset: u64,
    record_count: u64,
    record_bytes: u64,
    directory_offset: u64,
    block_count: u64,
    domain: &'static [u8],
}

fn check_verification_cancelled(control: Option<&OperationControl>) -> Result<(), TerrainError> {
    match control {
        Some(control) => control.check_cancelled().map_err(TerrainError::from),
        None => Ok(()),
    }
}

fn allocate_block_checksums(
    layout: ArtifactLayout,
    path: &Path,
    max_retained_handle_bytes: u64,
) -> Result<VerifiedBlockChecksums, TerrainError> {
    let vertex_count = usize::try_from(layout.vertex_block_count).map_err(|_| {
        TerrainError::resource(
            "Surface retained handle bytes",
            u64::MAX,
            max_retained_handle_bytes,
        )
    })?;
    let face_count = usize::try_from(layout.face_block_count).map_err(|_| {
        TerrainError::resource(
            "Surface retained handle bytes",
            u64::MAX,
            max_retained_handle_bytes,
        )
    })?;
    let required = accounted_retained_handle_bytes(path, vertex_count, face_count);
    require_within(
        "Surface retained handle bytes",
        required,
        max_retained_handle_bytes,
    )?;
    let mut vertices = Vec::new();
    vertices.try_reserve_exact(vertex_count).map_err(|_| {
        TerrainError::resource(
            "Surface retained handle bytes",
            required,
            max_retained_handle_bytes,
        )
    })?;
    let mut faces = Vec::new();
    faces.try_reserve_exact(face_count).map_err(|_| {
        TerrainError::resource(
            "Surface retained handle bytes",
            required,
            max_retained_handle_bytes,
        )
    })?;
    let checksums = VerifiedBlockChecksums { vertices, faces };
    let allocated = accounted_retained_handle_bytes(
        path,
        checksums.vertices.capacity(),
        checksums.faces.capacity(),
    );
    require_within(
        "Surface retained handle bytes",
        allocated,
        max_retained_handle_bytes,
    )?;
    Ok(checksums)
}

fn validate_artifact_blocks(
    file: &mut File,
    layout: ArtifactLayout,
    descriptor: &SurfaceArtifactDescriptor,
    max_verify_buffer_bytes: u64,
    max_retained_handle_bytes: u64,
    path: &Path,
    control: &OperationControl,
) -> Result<VerifiedBlockChecksums, TerrainError> {
    let mut checksums = allocate_block_checksums(layout, path, max_retained_handle_bytes)?;
    validate_record_blocks(
        file,
        RecordBlockLayout {
            record_offset: layout.vertex_offset,
            record_count: descriptor.vertex_count,
            record_bytes: VERTEX_RECORD_BYTES,
            directory_offset: layout.vertex_directory_offset,
            block_count: layout.vertex_block_count,
            domain: VERTEX_BLOCK_DOMAIN,
        },
        max_verify_buffer_bytes,
        path,
        ARTIFACT_KIND,
        Some(control),
        Some(&mut checksums.vertices),
    )?;
    validate_record_blocks(
        file,
        RecordBlockLayout {
            record_offset: layout.face_offset,
            record_count: descriptor.face_count,
            record_bytes: FACE_RECORD_BYTES,
            directory_offset: layout.face_directory_offset,
            block_count: layout.face_block_count,
            domain: FACE_BLOCK_DOMAIN,
        },
        max_verify_buffer_bytes,
        path,
        ARTIFACT_KIND,
        Some(control),
        Some(&mut checksums.faces),
    )?;
    Ok(checksums)
}

fn validate_record_blocks(
    file: &mut File,
    layout: RecordBlockLayout,
    max_verify_buffer_bytes: u64,
    path: &Path,
    kind: &'static str,
    control: Option<&OperationControl>,
    mut captured_checksums: Option<&mut Vec<[u8; 32]>>,
) -> Result<(), TerrainError> {
    if layout.block_count != block_count(layout.record_count) {
        return Err(TerrainError::corrupt_surface(
            kind,
            path.display(),
            "record block count is not canonical",
        ));
    }
    let max_block_bytes = RECORDS_PER_BLOCK.saturating_mul(layout.record_bytes);
    let mut buffer = verification_buffer(max_verify_buffer_bytes, max_block_bytes)?;
    for block_index in 0..layout.block_count {
        check_verification_cancelled(control)?;
        let expected =
            verify_record_block(file, layout, block_index, &mut buffer, path, kind, control)?;
        if let Some(checksums) = captured_checksums.as_deref_mut() {
            checksums.push(expected);
        }
    }
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "the verifier keeps the file, immutable open state, disk layout, range, buffer, path, and decoder explicit"
)]
fn read_verified_records(
    file: &mut File,
    opened_metadata: &fs::Metadata,
    layout: RecordBlockLayout,
    verified_checksums: &[[u8; 32]],
    first_record: u64,
    record_count: u64,
    buffer: &mut [u8],
    path: &Path,
    kind: &'static str,
    boundary: StreamReadBoundary,
    mut decode: impl FnMut(u64, &[u8]) -> Result<(), TerrainError>,
) -> Result<(), TerrainError> {
    if usize::try_from(layout.block_count).ok() != Some(verified_checksums.len()) {
        return Err(TerrainError::corrupt_surface(
            kind,
            path.display(),
            "verified block checksum count does not match the artifact layout",
        ));
    }
    let end_record = first_record.checked_add(record_count).ok_or_else(|| {
        TerrainError::corrupt_surface(kind, path.display(), "batch record range overflows")
    })?;
    if record_count == 0 || end_record > layout.record_count {
        return Err(TerrainError::corrupt_surface(
            kind,
            path.display(),
            "batch record range exceeds the artifact section",
        ));
    }
    if buffer.is_empty() {
        return Err(TerrainError::corrupt_surface(
            kind,
            path.display(),
            "record verification buffer is empty",
        ));
    }
    let record_bytes = usize::try_from(layout.record_bytes).map_err(|_| {
        TerrainError::corrupt_surface(kind, path.display(), "record width exceeds usize")
    })?;
    let mut record = [0_u8; MAX_SURFACE_RECORD_BYTES];
    if record_bytes == 0 || record_bytes > record.len() {
        return Err(TerrainError::corrupt_surface(
            kind,
            path.display(),
            "record width exceeds the stream decoder",
        ));
    }
    verify_stream_file_state(file, opened_metadata, path, kind)?;
    let last_record = end_record - 1;
    let first_block = first_record / RECORDS_PER_BLOCK;
    let last_block = last_record / RECORDS_PER_BLOCK;
    for block_index in first_block..=last_block {
        let block_first_record = block_index.checked_mul(RECORDS_PER_BLOCK).ok_or_else(|| {
            TerrainError::corrupt_surface(kind, path.display(), "block index overflows")
        })?;
        let block_records = layout
            .record_count
            .saturating_sub(block_first_record)
            .min(RECORDS_PER_BLOCK);
        if block_records == 0 || block_index >= layout.block_count {
            return Err(TerrainError::corrupt_surface(
                kind,
                path.display(),
                "checksum directory references an empty record block",
            ));
        }
        let block_byte_offset = block_first_record
            .checked_mul(layout.record_bytes)
            .and_then(|bytes| layout.record_offset.checked_add(bytes))
            .ok_or_else(|| {
                TerrainError::corrupt_surface(kind, path.display(), "block offset overflows")
            })?;
        let block_bytes = block_records
            .checked_mul(layout.record_bytes)
            .ok_or_else(|| {
                TerrainError::corrupt_surface(kind, path.display(), "block byte length overflows")
            })?;
        let selected_first_record = first_record.max(block_first_record);
        let selected_end_record = end_record.min(block_first_record + block_records);
        let selected_start_byte = (selected_first_record - block_first_record)
            .checked_mul(layout.record_bytes)
            .ok_or_else(|| {
                TerrainError::corrupt_surface(kind, path.display(), "batch byte range overflows")
            })?;
        let selected_end_byte = (selected_end_record - block_first_record)
            .checked_mul(layout.record_bytes)
            .ok_or_else(|| {
                TerrainError::corrupt_surface(kind, path.display(), "batch byte range overflows")
            })?;
        file.seek(SeekFrom::Start(block_byte_offset))
            .map_err(|error| {
                TerrainError::io("seek Surface record block", path.display(), error)
            })?;
        let mut hasher = block_hasher(layout.domain, block_index, block_records);
        let mut chunk_start = 0_u64;
        let mut record_fill = 0_usize;
        let mut next_record = selected_first_record;
        while chunk_start < block_bytes {
            let count = usize::try_from(
                (block_bytes - chunk_start).min(u64::try_from(buffer.len()).unwrap_or(u64::MAX)),
            )
            .expect("verification read is bounded by its buffer");
            read_exact(file, &mut buffer[..count], path, kind)?;
            hasher.update(&buffer[..count]);
            let chunk_end = chunk_start
                .checked_add(u64::try_from(count).expect("buffer length fits u64"))
                .ok_or_else(|| {
                    TerrainError::corrupt_surface(kind, path.display(), "block read overflows")
                })?;
            let capture_start = selected_start_byte.max(chunk_start);
            let capture_end = selected_end_byte.min(chunk_end);
            if capture_start < capture_end {
                maybe_injected_stream_mutation(boundary);
                let mut source_start = usize::try_from(capture_start - chunk_start)
                    .expect("capture offset is bounded by the verification buffer");
                let source_end = usize::try_from(capture_end - chunk_start)
                    .expect("capture offset is bounded by the verification buffer");
                while source_start < source_end {
                    let copied = (record_bytes - record_fill).min(source_end - source_start);
                    record[record_fill..record_fill + copied]
                        .copy_from_slice(&buffer[source_start..source_start + copied]);
                    record_fill += copied;
                    source_start += copied;
                    if record_fill == record_bytes {
                        decode(next_record, &record[..record_bytes])?;
                        next_record += 1;
                        record_fill = 0;
                    }
                }
            }
            chunk_start = chunk_end;
        }
        let checksum_offset = block_index
            .checked_mul(CHECKSUM_BYTES)
            .and_then(|bytes| layout.directory_offset.checked_add(bytes))
            .ok_or_else(|| {
                TerrainError::corrupt_surface(kind, path.display(), "checksum offset overflows")
            })?;
        file.seek(SeekFrom::Start(checksum_offset))
            .map_err(|error| {
                TerrainError::io("seek Surface block checksum", path.display(), error)
            })?;
        let live_checksum = read_exact_array::<32>(file, path, kind)?;
        let verified_checksum = verified_checksums
            .get(usize::try_from(block_index).unwrap_or(usize::MAX))
            .ok_or_else(|| {
                TerrainError::corrupt_surface(
                    kind,
                    path.display(),
                    "record block lies outside the verified checksum directory",
                )
            })?;
        if live_checksum != *verified_checksum
            || *verified_checksum != *hasher.finalize().as_bytes()
        {
            return Err(TerrainError::corrupt_surface(
                kind,
                path.display(),
                "record block checksum does not match",
            ));
        }
        if record_fill != 0 || next_record != selected_end_record {
            return Err(TerrainError::corrupt_surface(
                kind,
                path.display(),
                "verified record range was not decoded completely",
            ));
        }
    }
    verify_stream_file_state(file, opened_metadata, path, kind)
}

fn verify_stream_file_state(
    file: &File,
    opened_metadata: &fs::Metadata,
    path: &Path,
    kind: &'static str,
) -> Result<(), TerrainError> {
    let current = file.metadata().map_err(|error| {
        TerrainError::io("inspect verified Surface stream", path.display(), error)
    })?;
    if same_file_content_state(opened_metadata, &current) {
        return Ok(());
    }
    Err(TerrainError::corrupt_surface(
        kind,
        path.display(),
        "artifact file state changed after complete open verification",
    ))
}

fn verify_record_block(
    file: &mut File,
    layout: RecordBlockLayout,
    block_index: u64,
    buffer: &mut [u8],
    path: &Path,
    kind: &'static str,
    control: Option<&OperationControl>,
) -> Result<[u8; 32], TerrainError> {
    let first_record = block_index.checked_mul(RECORDS_PER_BLOCK).ok_or_else(|| {
        TerrainError::corrupt_surface(kind, path.display(), "block index overflows")
    })?;
    let records = layout
        .record_count
        .saturating_sub(first_record)
        .min(RECORDS_PER_BLOCK);
    if records == 0 || block_index >= layout.block_count {
        return Err(TerrainError::corrupt_surface(
            kind,
            path.display(),
            "checksum directory references an empty record block",
        ));
    }
    let block_byte_offset = first_record
        .checked_mul(layout.record_bytes)
        .and_then(|bytes| layout.record_offset.checked_add(bytes))
        .ok_or_else(|| {
            TerrainError::corrupt_surface(kind, path.display(), "block offset overflows")
        })?;
    let block_bytes = records.checked_mul(layout.record_bytes).ok_or_else(|| {
        TerrainError::corrupt_surface(kind, path.display(), "block byte length overflows")
    })?;
    file.seek(SeekFrom::Start(block_byte_offset))
        .map_err(|error| TerrainError::io("seek Surface record block", path.display(), error))?;
    let mut hasher = block_hasher(layout.domain, block_index, records);
    let mut remaining = block_bytes;
    while remaining > 0 {
        check_verification_cancelled(control)?;
        let count = usize::try_from(remaining.min(u64::try_from(buffer.len()).unwrap_or(u64::MAX)))
            .expect("verification read is bounded by its buffer");
        read_exact(file, &mut buffer[..count], path, kind)?;
        hasher.update(&buffer[..count]);
        remaining -= u64::try_from(count).expect("buffer length fits u64");
    }
    let checksum_offset = block_index
        .checked_mul(CHECKSUM_BYTES)
        .and_then(|bytes| layout.directory_offset.checked_add(bytes))
        .ok_or_else(|| {
            TerrainError::corrupt_surface(kind, path.display(), "checksum offset overflows")
        })?;
    file.seek(SeekFrom::Start(checksum_offset))
        .map_err(|error| TerrainError::io("seek Surface block checksum", path.display(), error))?;
    let expected = read_exact_array::<32>(file, path, kind)?;
    if expected != *hasher.finalize().as_bytes() {
        return Err(TerrainError::corrupt_surface(
            kind,
            path.display(),
            "record block checksum does not match",
        ));
    }
    Ok(expected)
}

fn verification_buffer(
    max_verify_buffer_bytes: u64,
    max_block_bytes: u64,
) -> Result<Vec<u8>, TerrainError> {
    let bytes = max_verify_buffer_bytes.min(max_block_bytes).max(1);
    let count = usize::try_from(bytes).map_err(|_| {
        TerrainError::resource(
            "Surface checksum verification buffer bytes",
            bytes,
            u64::try_from(usize::MAX).unwrap_or(u64::MAX),
        )
    })?;
    let mut buffer = Vec::new();
    buffer.try_reserve_exact(count).map_err(|_| {
        TerrainError::resource(
            "Surface checksum verification buffer bytes",
            bytes,
            max_verify_buffer_bytes,
        )
    })?;
    require_within(
        "Surface checksum verification buffer bytes",
        u64::try_from(buffer.capacity()).unwrap_or(u64::MAX),
        max_verify_buffer_bytes,
    )?;
    buffer.resize(count, 0);
    Ok(buffer)
}

fn validate_record_world_position(
    transform: PositionTransform,
    ticks: [i64; 3],
    recipe: TerrainRecipe,
    path: &Path,
    kind: &'static str,
    record: &'static str,
) -> Result<[f64; 3], TerrainError> {
    let world = transform.world_f64(ticks);
    if world.iter().any(|value| !value.is_finite()) {
        return Err(TerrainError::corrupt_surface(
            kind,
            path.display(),
            format!("{record} transforms to a non-finite world coordinate"),
        ));
    }
    let bounds = recipe.bounds().ok_or_else(|| {
        TerrainError::corrupt_surface(kind, path.display(), "stored Terrain AOI is absent")
    })?;
    if (0..3).any(|axis| world[axis] < bounds.min()[axis] || world[axis] > bounds.max()[axis]) {
        return Err(TerrainError::corrupt_surface(
            kind,
            path.display(),
            format!("{record} lies outside the bound Terrain Recipe AOI"),
        ));
    }
    Ok(world)
}

#[allow(clippy::too_many_lines)]
fn validate_artifact_payload(
    file: &mut File,
    path: &Path,
    descriptor: &SurfaceArtifactDescriptor,
    layout: ArtifactLayout,
    limits: crate::TerrainLimits,
    control: &OperationControl,
) -> Result<(), TerrainError> {
    file.seek(SeekFrom::Start(layout.vertex_offset))
        .map_err(|error| TerrainError::io("seek Surface vertices", path.display(), error))?;
    let mut geometry = domain_hasher(GEOMETRY_HASH_DOMAIN);
    let mut topology = domain_hasher(TOPOLOGY_HASH_DOMAIN);
    hash_transform(&mut geometry, descriptor.transform);
    geometry.update(&descriptor.vertex_count.to_le_bytes());
    topology.update(&descriptor.vertex_count.to_le_bytes());
    let mut world_min = [f64::INFINITY; 3];
    let mut world_max = [f64::NEG_INFINITY; 3];
    let mut min_xy = [i64::MAX; 2];
    let mut max_xy = [i64::MIN; 2];
    let mut previous_vertex: Option<([i64; 3], u64)> = None;
    let mut input_records =
        allocate_artifact_input_records(descriptor.vertex_count, limits.max_working_bytes())?;
    for index in 0..descriptor.vertex_count {
        if index.is_multiple_of(RECORDS_PER_BLOCK) {
            control.check_cancelled()?;
        }
        let bytes = read_exact_array::<32>(file, path, ARTIFACT_KIND)?;
        let ordinal = u64::from_le_bytes(bytes[0..8].try_into().expect("fixed slice"));
        let ticks = [
            i64::from_le_bytes(bytes[8..16].try_into().expect("fixed slice")),
            i64::from_le_bytes(bytes[16..24].try_into().expect("fixed slice")),
            i64::from_le_bytes(bytes[24..32].try_into().expect("fixed slice")),
        ];
        let key = (ticks, ordinal);
        if previous_vertex.is_some_and(|(previous_ticks, _)| previous_ticks[..2] == ticks[..2]) {
            return Err(TerrainError::corrupt_surface(
                ARTIFACT_KIND,
                path.display(),
                "vertices contain a duplicate horizontal position",
            ));
        }
        if previous_vertex.is_some_and(|previous| key <= previous) {
            return Err(TerrainError::corrupt_surface(
                ARTIFACT_KIND,
                path.display(),
                "vertices are not in canonical order",
            ));
        }
        previous_vertex = Some(key);
        input_records.push(InputVertex {
            ticks,
            point: PointId::new(descriptor.snapshot.source(), ordinal),
        });
        let id = index.saturating_add(1);
        geometry.update(&u32::try_from(id).unwrap_or(u32::MAX).to_le_bytes());
        geometry.update(descriptor.snapshot.source().as_bytes());
        geometry.update(&ordinal.to_le_bytes());
        for tick in ticks {
            geometry.update(&tick.to_le_bytes());
        }
        for axis in 0..2 {
            min_xy[axis] = min_xy[axis].min(ticks[axis]);
            max_xy[axis] = max_xy[axis].max(ticks[axis]);
        }
        let world = validate_record_world_position(
            descriptor.transform,
            ticks,
            descriptor.recipe,
            path,
            ARTIFACT_KIND,
            "vertex record",
        )?;
        for axis in 0..3 {
            world_min[axis] = world_min[axis].min(world[axis]);
            world_max[axis] = world_max[axis].max(world[axis]);
        }
    }
    if (0..2)
        .any(|axis| i128::from(max_xy[axis]) - i128::from(min_xy[axis]) > MAX_EXACT_F64_INTEGER)
    {
        return Err(TerrainError::corrupt_surface(
            ARTIFACT_KIND,
            path.display(),
            "XY tick span exceeds the exact normalized f64 integer range",
        ));
    }
    geometry.update(&descriptor.face_count.to_le_bytes());
    topology.update(&descriptor.face_count.to_le_bytes());
    let mut previous_face = None;
    for index in 0..descriptor.face_count {
        if index.is_multiple_of(RECORDS_PER_BLOCK) {
            control.check_cancelled()?;
        }
        let face_offset = layout
            .face_offset
            .checked_add(index.saturating_mul(FACE_RECORD_BYTES))
            .ok_or_else(|| {
                TerrainError::corrupt_surface(
                    ARTIFACT_KIND,
                    path.display(),
                    "face record offset overflows",
                )
            })?;
        file.seek(SeekFrom::Start(face_offset))
            .map_err(|error| TerrainError::io("seek Surface face", path.display(), error))?;
        let bytes = read_exact_array::<12>(file, path, ARTIFACT_KIND)?;
        let vertices = [
            u32::from_le_bytes(bytes[0..4].try_into().expect("fixed slice")),
            u32::from_le_bytes(bytes[4..8].try_into().expect("fixed slice")),
            u32::from_le_bytes(bytes[8..12].try_into().expect("fixed slice")),
        ];
        validate_face_record(vertices, descriptor.vertex_count, previous_face, path)?;
        validate_face_orientation(file, layout.vertex_offset, vertices, path)?;
        previous_face = Some(vertices);
        let id = u32::try_from(index.saturating_add(1)).unwrap_or(u32::MAX);
        geometry.update(&id.to_le_bytes());
        topology.update(&id.to_le_bytes());
        for vertex in vertices {
            geometry.update(&vertex.to_le_bytes());
            topology.update(&vertex.to_le_bytes());
        }
    }
    let geometry_hash = ContentHash::new(*geometry.finalize().as_bytes());
    let topology_hash = ContentHash::new(*topology.finalize().as_bytes());
    if geometry_hash != descriptor.geometry_hash {
        return Err(TerrainError::corrupt_surface(
            ARTIFACT_KIND,
            path.display(),
            "geometry hash does not match canonical records",
        ));
    }
    if topology_hash != descriptor.topology_hash {
        return Err(TerrainError::corrupt_surface(
            ARTIFACT_KIND,
            path.display(),
            "topology hash does not match canonical faces",
        ));
    }
    let bounds = WorldBounds::new(world_min, world_max).map_err(|_| {
        TerrainError::corrupt_surface(
            ARTIFACT_KIND,
            path.display(),
            "canonical vertices produce invalid world bounds",
        )
    })?;
    if bounds != descriptor.bounds {
        return Err(TerrainError::corrupt_surface(
            ARTIFACT_KIND,
            path.display(),
            "geometry bounds do not match canonical vertices",
        ));
    }
    validate_artifact_input_hash(&mut input_records, descriptor, path)?;
    let canonical_hash =
        canonical_topology_hash(input_records, descriptor.transform, limits, control)
            .map_err(|error| map_topology_validation_error(error, path))?;
    if canonical_hash != descriptor.topology_hash {
        return Err(TerrainError::corrupt_surface(
            ARTIFACT_KIND,
            path.display(),
            "faces do not match the canonical Delaunay topology",
        ));
    }
    Ok(())
}

fn map_topology_validation_error(error: TerrainError, path: &Path) -> TerrainError {
    match error {
        limit @ (TerrainError::ResourceLimit { .. } | TerrainError::Cancelled) => limit,
        _ => TerrainError::corrupt_surface(
            ARTIFACT_KIND,
            path.display(),
            "canonical Delaunay topology could not be reproduced from the vertex records",
        ),
    }
}

fn allocate_artifact_input_records(
    record_count: u64,
    max_working_bytes: u64,
) -> Result<Vec<InputVertex>, TerrainError> {
    let required_bytes = record_count
        .saturating_mul(u64::try_from(mem::size_of::<InputVertex>()).unwrap_or(u64::MAX));
    require_within(
        "Surface input-hash verification bytes",
        required_bytes,
        max_working_bytes,
    )?;
    let count = usize::try_from(record_count).map_err(|_| {
        TerrainError::resource(
            "Surface input-hash verification bytes",
            required_bytes,
            max_working_bytes,
        )
    })?;
    let mut records = Vec::new();
    records.try_reserve_exact(count).map_err(|_| {
        TerrainError::resource(
            "Surface input-hash verification bytes",
            required_bytes,
            max_working_bytes,
        )
    })?;
    let allocated_bytes = u64::try_from(records.capacity())
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(mem::size_of::<InputVertex>()).unwrap_or(u64::MAX));
    require_within(
        "Surface input-hash verification bytes",
        allocated_bytes,
        max_working_bytes,
    )?;
    Ok(records)
}

fn validate_artifact_input_hash(
    records: &mut [InputVertex],
    descriptor: &SurfaceArtifactDescriptor,
    path: &Path,
) -> Result<(), TerrainError> {
    records.sort_unstable_by_key(|record| record.point.ordinal());
    if records
        .windows(2)
        .any(|pair| pair[0].point.ordinal() == pair[1].point.ordinal())
    {
        return Err(TerrainError::corrupt_surface(
            ARTIFACT_KIND,
            path.display(),
            "canonical vertices contain duplicate Source ordinals",
        ));
    }
    let mut hasher = snapshot_input_hasher(descriptor.snapshot);
    for record in records {
        hash_snapshot_input_record(
            &mut hasher,
            record.point.ordinal(),
            record.ticks,
            descriptor.recipe.ground_classification(),
        );
    }
    if ContentHash::new(*hasher.finalize().as_bytes()) != descriptor.input_hash {
        return Err(TerrainError::corrupt_surface(
            ARTIFACT_KIND,
            path.display(),
            "Snapshot Point content hash does not match canonical vertex records",
        ));
    }
    Ok(())
}

fn validate_face_record(
    vertices: [u32; 3],
    vertex_count: u64,
    previous: Option<[u32; 3]>,
    path: &Path,
) -> Result<(), TerrainError> {
    if vertices
        .iter()
        .any(|&id| id == 0 || u64::from(id) > vertex_count)
        || vertices[0] == vertices[1]
        || vertices[1] == vertices[2]
        || vertices[0] == vertices[2]
    {
        return Err(TerrainError::corrupt_surface(
            ARTIFACT_KIND,
            path.display(),
            "face contains an invalid vertex identity",
        ));
    }
    if vertices[0] != *vertices.iter().min().expect("three vertices") {
        return Err(TerrainError::corrupt_surface(
            ARTIFACT_KIND,
            path.display(),
            "face does not start with its minimum canonical vertex",
        ));
    }
    if previous.is_some_and(|value| vertices <= value) {
        return Err(TerrainError::corrupt_surface(
            ARTIFACT_KIND,
            path.display(),
            "faces are not in canonical order",
        ));
    }
    Ok(())
}

fn validate_face_orientation(
    file: &mut File,
    vertex_offset: u64,
    vertices: [u32; 3],
    path: &Path,
) -> Result<(), TerrainError> {
    let [a, b, c] = vertices.map(|id| read_vertex_ticks(file, vertex_offset, id, path));
    let a = a?;
    let b = b?;
    let c = c?;
    let edge_one = [
        i128::from(b[0]) - i128::from(a[0]),
        i128::from(b[1]) - i128::from(a[1]),
    ];
    let edge_two = [
        i128::from(c[0]) - i128::from(a[0]),
        i128::from(c[1]) - i128::from(a[1]),
    ];
    if edge_one[0] * edge_two[1] - edge_one[1] * edge_two[0] <= 0 {
        return Err(TerrainError::corrupt_surface(
            ARTIFACT_KIND,
            path.display(),
            "face orientation is not strictly counter-clockwise",
        ));
    }
    Ok(())
}

fn read_vertex_ticks(
    file: &mut File,
    vertex_offset: u64,
    vertex_id: u32,
    path: &Path,
) -> Result<[i64; 3], TerrainError> {
    let record_offset = u64::from(vertex_id - 1)
        .checked_mul(VERTEX_RECORD_BYTES)
        .and_then(|bytes| vertex_offset.checked_add(bytes))
        .ok_or_else(|| {
            TerrainError::corrupt_surface(
                ARTIFACT_KIND,
                path.display(),
                "vertex record offset overflows",
            )
        })?;
    file.seek(SeekFrom::Start(record_offset))
        .map_err(|error| TerrainError::io("seek Surface face vertex", path.display(), error))?;
    let bytes = read_exact_array::<32>(file, path, ARTIFACT_KIND)?;
    Ok(ticks_from_input_bytes(&bytes))
}

#[derive(Clone, Copy)]
struct ArtifactLayout {
    vertex_offset: u64,
    vertex_bytes: u64,
    face_offset: u64,
    face_bytes: u64,
    vertex_directory_offset: u64,
    vertex_directory_bytes: u64,
    vertex_block_count: u64,
    face_directory_offset: u64,
    face_directory_bytes: u64,
    face_block_count: u64,
    file_bytes: u64,
}

impl ArtifactLayout {
    fn new(vertex_count: u64, face_count: u64) -> Result<Self, TerrainError> {
        let vertex_bytes = vertex_count
            .checked_mul(VERTEX_RECORD_BYTES)
            .ok_or_else(|| {
                TerrainError::resource("Surface artifact bytes", u64::MAX, u64::MAX - 1)
            })?;
        let face_bytes = face_count.checked_mul(FACE_RECORD_BYTES).ok_or_else(|| {
            TerrainError::resource("Surface artifact bytes", u64::MAX, u64::MAX - 1)
        })?;
        let vertex_block_count = block_count(vertex_count);
        let face_block_count = block_count(face_count);
        let vertex_directory_bytes = directory_bytes(vertex_block_count);
        let face_directory_bytes = directory_bytes(face_block_count);
        let vertex_offset = ARTIFACT_HEADER_BYTES;
        let face_offset = vertex_offset.checked_add(vertex_bytes).ok_or_else(|| {
            TerrainError::resource("Surface artifact bytes", u64::MAX, u64::MAX - 1)
        })?;
        let vertex_directory_offset = face_offset.checked_add(face_bytes).ok_or_else(|| {
            TerrainError::resource("Surface artifact bytes", u64::MAX, u64::MAX - 1)
        })?;
        let face_directory_offset = vertex_directory_offset
            .checked_add(vertex_directory_bytes)
            .ok_or_else(|| {
                TerrainError::resource("Surface artifact bytes", u64::MAX, u64::MAX - 1)
            })?;
        let file_bytes = face_directory_offset
            .checked_add(face_directory_bytes)
            .and_then(|bytes| bytes.checked_add(CHECKSUM_BYTES))
            .ok_or_else(|| {
                TerrainError::resource("Surface artifact bytes", u64::MAX, u64::MAX - 1)
            })?;
        Ok(Self {
            vertex_offset,
            vertex_bytes,
            face_offset,
            face_bytes,
            vertex_directory_offset,
            vertex_directory_bytes,
            vertex_block_count,
            face_directory_offset,
            face_directory_bytes,
            face_block_count,
            file_bytes,
        })
    }
}

struct DecodedArtifact {
    descriptor: SurfaceArtifactDescriptor,
    layout: ArtifactLayout,
}

fn encode_artifact_header(
    descriptor: &SurfaceArtifactDescriptor,
    layout: ArtifactLayout,
) -> Result<Vec<u8>, TerrainError> {
    let profile = supported_profile(&descriptor.coordinate_reference)?;
    let mut bytes =
        Vec::with_capacity(usize::try_from(ARTIFACT_HEADER_BYTES).expect("small header"));
    bytes.extend_from_slice(ARTIFACT_MAGIC);
    push_u32(&mut bytes, SURFACE_DISK_VERSION);
    push_u32(&mut bytes, crate::ALGORITHM_VERSION);
    push_u64(&mut bytes, ARTIFACT_HEADER_BYTES);
    push_u64(&mut bytes, layout.file_bytes);
    encode_provenance(&mut bytes, descriptor.snapshot);
    bytes.push(descriptor.recipe.ground_classification());
    encode_required_recipe_bounds(&mut bytes, descriptor.recipe)?;
    bytes.extend_from_slice(descriptor.recipe_hash.as_bytes());
    encode_transform(&mut bytes, descriptor.transform);
    bytes.extend_from_slice(&profile.canonical_bytes());
    bytes.extend_from_slice(descriptor.input_hash.as_bytes());
    bytes.extend_from_slice(descriptor.geometry_hash.as_bytes());
    bytes.extend_from_slice(descriptor.topology_hash.as_bytes());
    bytes.extend_from_slice(descriptor.artifact_hash.as_bytes());
    push_u64(&mut bytes, descriptor.input_point_count);
    push_u64(&mut bytes, descriptor.vertex_count);
    push_u64(&mut bytes, descriptor.face_count);
    push_u64(&mut bytes, descriptor.hull_vertex_count);
    encode_bounds(&mut bytes, descriptor.bounds);
    push_u64(&mut bytes, layout.vertex_offset);
    push_u64(&mut bytes, layout.vertex_bytes);
    push_u64(&mut bytes, layout.face_offset);
    push_u64(&mut bytes, layout.face_bytes);
    push_u64(&mut bytes, layout.vertex_directory_offset);
    push_u64(&mut bytes, layout.vertex_directory_bytes);
    push_u64(&mut bytes, layout.vertex_block_count);
    push_u64(&mut bytes, layout.face_directory_offset);
    push_u64(&mut bytes, layout.face_directory_bytes);
    push_u64(&mut bytes, layout.face_block_count);
    pad_header(&mut bytes, ARTIFACT_HEADER_BYTES, ARTIFACT_KIND)?;
    Ok(bytes)
}

#[allow(clippy::too_many_lines)]
fn decode_artifact_header(
    header: &[u8],
    path: &Path,
    expected: ExpectedBinding,
    file_bytes: u64,
) -> Result<DecodedArtifact, TerrainError> {
    let mut decoder = Decoder::new(header, path, PersistedKind::Artifact);
    decoder.require_magic()?;
    decoder.require_version()?;
    let binding = decode_surface_binding(
        &mut decoder,
        expected,
        file_bytes,
        Some(ARTIFACT_HEADER_BYTES),
    )?;
    let transform = binding.transform;
    let profile = binding.profile;
    let stored_recipe_hash = binding.recipe_hash;
    let coordinate_reference = CoordinateReference::profile(profile);
    let input_hash = decoder.hash()?;
    let geometry_hash = decoder.hash()?;
    let topology_hash = decoder.hash()?;
    let stored_artifact_hash = decoder.hash()?;
    let input_point_count = decoder.u64()?;
    let vertex_count = decoder.u64()?;
    let face_count = decoder.u64()?;
    let hull_vertex_count = decoder.u64()?;
    let bounds = decoder.bounds()?;
    let layout = ArtifactLayout {
        vertex_offset: decoder.u64()?,
        vertex_bytes: decoder.u64()?,
        face_offset: decoder.u64()?,
        face_bytes: decoder.u64()?,
        vertex_directory_offset: decoder.u64()?,
        vertex_directory_bytes: decoder.u64()?,
        vertex_block_count: decoder.u64()?,
        face_directory_offset: decoder.u64()?,
        face_directory_bytes: decoder.u64()?,
        face_block_count: decoder.u64()?,
        file_bytes,
    };
    decoder.require_zero_padding()?;
    if input_point_count != vertex_count {
        return Err(TerrainError::corrupt_surface(
            ARTIFACT_KIND,
            path.display(),
            "Ground Input and canonical vertex counts differ",
        ));
    }
    if vertex_count > u64::from(u32::MAX) || face_count > u64::from(u32::MAX) {
        return Err(TerrainError::corrupt_surface(
            ARTIFACT_KIND,
            path.display(),
            "Surface identity count exceeds the disk-v1 u32 range",
        ));
    }
    let canonical_layout = ArtifactLayout::new(vertex_count, face_count).map_err(|_| {
        TerrainError::corrupt_surface(
            ARTIFACT_KIND,
            path.display(),
            "declared counts overflow the canonical section layout",
        )
    })?;
    if layout.vertex_offset != canonical_layout.vertex_offset
        || layout.vertex_bytes != canonical_layout.vertex_bytes
        || layout.face_offset != canonical_layout.face_offset
        || layout.face_bytes != canonical_layout.face_bytes
        || layout.vertex_directory_offset != canonical_layout.vertex_directory_offset
        || layout.vertex_directory_bytes != canonical_layout.vertex_directory_bytes
        || layout.vertex_block_count != canonical_layout.vertex_block_count
        || layout.face_directory_offset != canonical_layout.face_directory_offset
        || layout.face_directory_bytes != canonical_layout.face_directory_bytes
        || layout.face_block_count != canonical_layout.face_block_count
        || file_bytes != canonical_layout.file_bytes
    {
        return Err(TerrainError::corrupt_surface(
            ARTIFACT_KIND,
            path.display(),
            "section layout is not canonical for the declared counts",
        ));
    }
    let expected_hull = hull_count(vertex_count, face_count).ok_or_else(|| {
        TerrainError::corrupt_surface(
            ARTIFACT_KIND,
            path.display(),
            "counts violate planar triangulation facts",
        )
    })?;
    if hull_vertex_count != expected_hull {
        return Err(TerrainError::corrupt_surface(
            ARTIFACT_KIND,
            path.display(),
            "hull count violates planar triangulation facts",
        ));
    }
    let computed_artifact_hash = artifact_hash(
        expected.snapshot,
        stored_recipe_hash,
        transform,
        &coordinate_reference,
        input_hash,
        geometry_hash,
        topology_hash,
    );
    if computed_artifact_hash != stored_artifact_hash {
        return Err(TerrainError::corrupt_surface(
            ARTIFACT_KIND,
            path.display(),
            "semantic artifact hash does not match its bindings",
        ));
    }
    Ok(DecodedArtifact {
        descriptor: SurfaceArtifactDescriptor {
            snapshot: expected.snapshot,
            recipe: expected.recipe,
            recipe_hash: stored_recipe_hash,
            transform,
            coordinate_reference,
            input_hash,
            geometry_hash,
            topology_hash,
            artifact_hash: stored_artifact_hash,
            input_point_count,
            vertex_count,
            face_count,
            hull_vertex_count,
            bounds,
        },
        layout: canonical_layout,
    })
}

struct DecodedWork {
    transform: PositionTransform,
    profile: SpatialReferenceProfile,
    input_hash: ContentHash,
    input_point_count: u64,
    layout: WorkLayout,
}

fn encode_work_header(
    input: &CollectedTerrainInput,
    layout: WorkLayout,
) -> Result<Vec<u8>, TerrainError> {
    let profile = supported_profile(&input.coordinate_reference)?;
    let mut bytes = Vec::with_capacity(usize::try_from(WORK_HEADER_BYTES).expect("small header"));
    bytes.extend_from_slice(WORK_MAGIC);
    push_u32(&mut bytes, WORK_DISK_VERSION);
    push_u32(&mut bytes, crate::ALGORITHM_VERSION);
    push_u64(&mut bytes, layout.file_bytes);
    encode_provenance(&mut bytes, input.snapshot);
    bytes.push(input.recipe.ground_classification());
    encode_required_recipe_bounds(&mut bytes, input.recipe)?;
    bytes.extend_from_slice(recipe_hash(input.recipe).as_bytes());
    encode_transform(&mut bytes, input.transform);
    bytes.extend_from_slice(&profile.canonical_bytes());
    bytes.extend_from_slice(input.input_hash.as_bytes());
    push_u64(
        &mut bytes,
        u64::try_from(input.vertices.len()).unwrap_or(u64::MAX),
    );
    push_u64(&mut bytes, layout.record_offset);
    push_u64(&mut bytes, layout.record_bytes);
    push_u64(&mut bytes, layout.directory_offset);
    push_u64(&mut bytes, layout.directory_bytes);
    push_u64(&mut bytes, layout.block_count);
    pad_header(&mut bytes, WORK_HEADER_BYTES, WORK_KIND)?;
    Ok(bytes)
}

fn decode_work_header(
    header: &[u8],
    path: &Path,
    expected: ExpectedBinding,
    file_bytes: u64,
) -> Result<DecodedWork, TerrainError> {
    let mut decoder = Decoder::new(header, path, PersistedKind::Work);
    decoder.require_magic()?;
    decoder.require_version()?;
    let binding = decode_surface_binding(&mut decoder, expected, file_bytes, None)?;
    let transform = binding.transform;
    let profile = binding.profile;
    let input_hash = decoder.hash()?;
    let input_point_count = decoder.u64()?;
    let layout = WorkLayout {
        record_offset: decoder.u64()?,
        record_bytes: decoder.u64()?,
        directory_offset: decoder.u64()?,
        directory_bytes: decoder.u64()?,
        block_count: decoder.u64()?,
        file_bytes,
    };
    decoder.require_zero_padding()?;
    let canonical_layout = WorkLayout::new(input_point_count).map_err(|_| {
        TerrainError::corrupt_surface(
            WORK_KIND,
            path.display(),
            "declared count overflows the canonical work layout",
        )
    })?;
    if layout.record_offset != canonical_layout.record_offset
        || layout.record_bytes != canonical_layout.record_bytes
        || layout.directory_offset != canonical_layout.directory_offset
        || layout.directory_bytes != canonical_layout.directory_bytes
        || layout.block_count != canonical_layout.block_count
        || layout.file_bytes != canonical_layout.file_bytes
    {
        return Err(TerrainError::corrupt_surface(
            WORK_KIND,
            path.display(),
            "record or checksum-directory layout is not canonical",
        ));
    }
    Ok(DecodedWork {
        transform,
        profile,
        input_hash,
        input_point_count,
        layout: canonical_layout,
    })
}

struct DecodedSurfaceBinding {
    recipe_hash: ContentHash,
    transform: PositionTransform,
    profile: SpatialReferenceProfile,
}

fn decode_surface_binding(
    decoder: &mut Decoder<'_>,
    expected: ExpectedBinding,
    file_bytes: u64,
    encoded_header_bytes: Option<u64>,
) -> Result<DecodedSurfaceBinding, TerrainError> {
    let algorithm = decoder.u32()?;
    if algorithm != crate::ALGORITHM_VERSION {
        return Err(TerrainError::stale_surface(
            decoder.kind.label(),
            "terrain algorithm version",
            decoder.path.display(),
        ));
    }
    if let Some(header_bytes) = encoded_header_bytes {
        decoder.require_u64(header_bytes, "header length")?;
    }
    decoder.require_u64(file_bytes, "file length")?;
    decoder.require_provenance(expected.snapshot)?;
    let ground = decoder.u8()?;
    let bounds = decoder.bounds()?;
    let recipe = TerrainRecipe::new(ground).within(bounds);
    let stored_recipe_hash = decoder.hash()?;
    if stored_recipe_hash != expected.recipe_hash || recipe_hash(recipe) != expected.recipe_hash {
        return Err(TerrainError::stale_surface(
            decoder.kind.label(),
            "Terrain Recipe",
            decoder.path.display(),
        ));
    }
    let transform = decoder.transform()?;
    let profile = decoder.profile()?;
    if !transform_bits_equal(transform, expected.transform) {
        return Err(TerrainError::stale_surface(
            decoder.kind.label(),
            "position transform",
            decoder.path.display(),
        ));
    }
    if profile != expected.profile {
        return Err(TerrainError::stale_surface(
            decoder.kind.label(),
            "spatial reference",
            decoder.path.display(),
        ));
    }
    Ok(DecodedSurfaceBinding {
        recipe_hash: stored_recipe_hash,
        transform,
        profile,
    })
}

struct Decoder<'a> {
    bytes: &'a [u8],
    offset: usize,
    path: &'a Path,
    kind: PersistedKind,
}

impl<'a> Decoder<'a> {
    const fn new(bytes: &'a [u8], path: &'a Path, kind: PersistedKind) -> Self {
        Self {
            bytes,
            offset: 0,
            path,
            kind,
        }
    }

    fn take<const N: usize>(&mut self) -> Result<[u8; N], TerrainError> {
        let end = self
            .offset
            .checked_add(N)
            .ok_or_else(|| self.corrupt("header offset overflows"))?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| self.corrupt("header is truncated"))?;
        self.offset = end;
        Ok(bytes.try_into().expect("checked fixed slice"))
    }

    fn u8(&mut self) -> Result<u8, TerrainError> {
        Ok(self.take::<1>()?[0])
    }

    fn u32(&mut self) -> Result<u32, TerrainError> {
        Ok(u32::from_le_bytes(self.take()?))
    }

    fn u64(&mut self) -> Result<u64, TerrainError> {
        Ok(u64::from_le_bytes(self.take()?))
    }

    fn hash(&mut self) -> Result<ContentHash, TerrainError> {
        Ok(ContentHash::new(self.take()?))
    }

    fn bounds(&mut self) -> Result<WorldBounds, TerrainError> {
        let min = [self.f64()?, self.f64()?, self.f64()?];
        let max = [self.f64()?, self.f64()?, self.f64()?];
        WorldBounds::new(min, max).map_err(|_| self.corrupt("header contains invalid bounds"))
    }

    fn transform(&mut self) -> Result<PositionTransform, TerrainError> {
        let offset = [self.f64()?, self.f64()?, self.f64()?];
        let scale = [self.f64()?, self.f64()?, self.f64()?];
        PositionTransform::new(offset, scale)
            .map_err(|_| self.corrupt("header contains an invalid position transform"))
    }

    fn f64(&mut self) -> Result<f64, TerrainError> {
        let value = f64::from_bits(self.u64()?);
        if !value.is_finite() {
            return Err(self.corrupt("header contains a non-finite coordinate"));
        }
        Ok(value)
    }

    fn profile(&mut self) -> Result<SpatialReferenceProfile, TerrainError> {
        decode_profile(self.take()?, self.path, self.kind.label())
    }

    fn require_magic(&mut self) -> Result<(), TerrainError> {
        if self.take::<8>()? != self.kind.magic() {
            return Err(TerrainError::incompatible_surface(
                self.kind.label(),
                self.path.display(),
                0,
                self.kind.version(),
            ));
        }
        Ok(())
    }

    fn require_version(&mut self) -> Result<(), TerrainError> {
        let version = self.u32()?;
        if version != self.kind.version() {
            return Err(TerrainError::incompatible_surface(
                self.kind.label(),
                self.path.display(),
                version,
                self.kind.version(),
            ));
        }
        Ok(())
    }

    fn require_u64(&mut self, expected: u64, field: &'static str) -> Result<(), TerrainError> {
        if self.u64()? != expected {
            return Err(self.corrupt(format!("{field} is not canonical")));
        }
        Ok(())
    }

    fn require_provenance(&mut self, expected: SnapshotProvenance) -> Result<(), TerrainError> {
        let workspace = self.take::<16>()?;
        let source = self.take::<32>()?;
        let revision = self.take::<32>()?;
        let binding = if workspace != *expected.workspace().as_bytes() {
            Some("Workspace identity")
        } else if source != *expected.source().as_bytes() {
            Some("Source identity")
        } else if revision != *expected.revision().as_bytes() {
            Some("Snapshot Revision")
        } else {
            None
        };
        if let Some(binding) = binding {
            return Err(TerrainError::stale_surface(
                self.kind.label(),
                binding,
                self.path.display(),
            ));
        }
        Ok(())
    }

    fn require_zero_padding(&self) -> Result<(), TerrainError> {
        if self.bytes[self.offset..].iter().any(|&byte| byte != 0) {
            return Err(self.corrupt("reserved header bytes are not zero"));
        }
        Ok(())
    }

    fn corrupt(&self, reason: impl AsRef<str>) -> TerrainError {
        TerrainError::corrupt_surface(self.kind.label(), self.path.display(), reason)
    }
}

fn decode_profile(
    bytes: [u8; 16],
    path: &Path,
    kind: &'static str,
) -> Result<SpatialReferenceProfile, TerrainError> {
    if bytes[0..4] != [1, 0, 0, 0]
        || bytes[12] != 1
        || bytes[13] != 1
        || bytes[14] != 1
        || !matches!(bytes[15], 1 | 2)
    {
        return Err(TerrainError::corrupt_surface(
            kind,
            path.display(),
            "spatial-reference profile bytes are unsupported or malformed",
        ));
    }
    let provenance = match bytes[15] {
        1 => SpatialReferenceProvenance::SourceMetadata,
        2 => SpatialReferenceProvenance::CallerDeclaration,
        _ => unreachable!("validated profile provenance"),
    };
    SpatialReferenceProfile::new(
        u32::from_le_bytes(bytes[4..8].try_into().expect("fixed slice")),
        u32::from_le_bytes(bytes[8..12].try_into().expect("fixed slice")),
        SpatialAxes::EastingNorthingElevation,
        LinearUnit::Metre,
        LinearUnit::Metre,
        provenance,
    )
    .map_err(|_| {
        TerrainError::corrupt_surface(
            kind,
            path.display(),
            "spatial-reference profile contains a zero EPSG identity",
        )
    })
}

fn supported_profile(
    coordinate_reference: &CoordinateReference,
) -> Result<SpatialReferenceProfile, TerrainError> {
    coordinate_reference
        .spatial_profile()
        .filter(|profile| profile.is_supported_metric_survey())
        .ok_or_else(|| {
            TerrainError::unsupported_spatial_reference(
                "persistent Surface artifacts require the supported structured metre survey profile",
            )
        })
}

struct VerifiedFile {
    bytes: u64,
    checksum: ContentHash,
}

fn verified_file(
    file: &mut File,
    path: &Path,
    kind: PersistedKind,
    limits: TerrainPrepareLimits,
    max_buffer_bytes: u64,
    control: &OperationControl,
) -> Result<VerifiedFile, TerrainError> {
    let file_bytes = file
        .metadata()
        .map_err(|error| TerrainError::io("inspect durable Surface file", path.display(), error))?
        .len();
    require_within(
        kind.resource_limit(),
        file_bytes,
        kind.max_file_bytes(limits),
    )?;
    let minimum = kind.header_bytes().saturating_add(CHECKSUM_BYTES);
    if file_bytes < minimum {
        return Err(TerrainError::corrupt_surface(
            kind.label(),
            path.display(),
            "file is shorter than its fixed header and checksum",
        ));
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|error| TerrainError::io("seek durable Surface file", path.display(), error))?;
    let payload_bytes = file_bytes - CHECKSUM_BYTES;
    let buffer_bytes =
        usize::try_from(max_buffer_bytes.min(payload_bytes).max(1)).map_err(|_| {
            TerrainError::resource(
                "Surface checksum verification buffer bytes",
                max_buffer_bytes,
                u64::try_from(usize::MAX).unwrap_or(u64::MAX),
            )
        })?;
    let mut buffer = Vec::new();
    buffer.try_reserve_exact(buffer_bytes).map_err(|_| {
        TerrainError::resource(
            "Surface checksum verification buffer bytes",
            u64::try_from(buffer_bytes).unwrap_or(u64::MAX),
            max_buffer_bytes,
        )
    })?;
    require_within(
        "Surface checksum verification buffer bytes",
        u64::try_from(buffer.capacity()).unwrap_or(u64::MAX),
        max_buffer_bytes,
    )?;
    buffer.resize(buffer_bytes, 0);
    let mut hasher = checksum_hasher(kind.checksum_domain());
    let mut remaining = payload_bytes;
    while remaining > 0 {
        control.check_cancelled()?;
        let count = usize::try_from(remaining.min(u64::try_from(buffer.len()).unwrap_or(u64::MAX)))
            .expect("bounded by buffer length");
        read_exact(file, &mut buffer[..count], path, kind.label())?;
        hasher.update(&buffer[..count]);
        remaining -= u64::try_from(count).expect("count fits u64");
    }
    let expected = read_exact_array::<32>(file, path, kind.label())?;
    if expected != *hasher.finalize().as_bytes() {
        return Err(TerrainError::corrupt_surface(
            kind.label(),
            path.display(),
            "whole-file checksum does not match",
        ));
    }
    Ok(VerifiedFile {
        bytes: file_bytes,
        checksum: ContentHash::new(expected),
    })
}

fn read_surface_batch<T>(
    stream: &mut SurfaceBatchStream,
    count: u64,
    mut decode: impl FnMut(u64, &[u8], &Path) -> Result<T, TerrainError>,
) -> Result<Vec<T>, TerrainError> {
    let surface = Arc::clone(&stream.surface);
    let section = stream.kind.section(&surface);
    let batch_work = batch_work_units(section.record_count, stream.next_id - 1, count)?;
    let next_work = stream
        .used_work_units
        .checked_add(batch_work)
        .ok_or_else(|| {
            TerrainError::resource("Surface read work units", u64::MAX, stream.max_work_units)
        })?;
    require_within("Surface read work units", next_work, stream.max_work_units)?;
    let count_usize = usize::try_from(count).expect("batch count was bounded by usize");
    let mut result = allocate_surface_batch::<T>(
        count_usize,
        stream.max_batch_payload_bytes,
        stream.max_working_bytes,
        stream.verify_buffer.capacity(),
        section.batch_bytes_label,
    )?;
    let file = Arc::clone(&surface.file);
    let mut file = file.lock().map_err(|_| {
        TerrainError::corrupt_surface(
            ARTIFACT_KIND,
            surface.path.display(),
            "verified artifact reader lock was poisoned",
        )
    })?;
    let first_record = stream.next_id - 1;
    let path = &surface.path;
    read_verified_records(
        &mut file,
        &surface.opened_metadata,
        RecordBlockLayout {
            record_offset: section.record_offset,
            record_count: section.record_count,
            record_bytes: section.record_bytes,
            directory_offset: section.directory_offset,
            block_count: block_count(section.record_count),
            domain: section.domain,
        },
        section.checksums,
        first_record,
        count,
        &mut stream.verify_buffer,
        path,
        ARTIFACT_KIND,
        section.boundary,
        |record_index, bytes| {
            result.push(decode(record_index, bytes, path)?);
            Ok(())
        },
    )?;
    stream.next_id += count;
    stream.remaining -= count;
    stream.used_work_units = next_work;
    Ok(result)
}

fn decode_vertex_record(
    source: SourceId,
    record_index: u64,
    bytes: &[u8],
    path: &Path,
) -> Result<SurfaceVertex, TerrainError> {
    let id = SurfaceVertexId::from_zero_based(usize::try_from(record_index).unwrap_or(usize::MAX))
        .ok_or_else(|| {
            TerrainError::corrupt_surface(
                ARTIFACT_KIND,
                path.display(),
                "vertex identity exceeds u32",
            )
        })?;
    Ok(SurfaceVertex::new(
        id,
        PointId::new(
            source,
            u64::from_le_bytes(bytes[0..8].try_into().expect("fixed slice")),
        ),
        [
            i64::from_le_bytes(bytes[8..16].try_into().expect("fixed slice")),
            i64::from_le_bytes(bytes[16..24].try_into().expect("fixed slice")),
            i64::from_le_bytes(bytes[24..32].try_into().expect("fixed slice")),
        ],
    ))
}

fn decode_face_record(
    vertex_count: u64,
    record_index: u64,
    bytes: &[u8],
    path: &Path,
) -> Result<SurfaceFace, TerrainError> {
    let raw = [
        u32::from_le_bytes(bytes[0..4].try_into().expect("fixed slice")),
        u32::from_le_bytes(bytes[4..8].try_into().expect("fixed slice")),
        u32::from_le_bytes(bytes[8..12].try_into().expect("fixed slice")),
    ];
    validate_face_record(raw, vertex_count, None, path)?;
    let id = SurfaceFaceId::from_zero_based(usize::try_from(record_index).unwrap_or(usize::MAX))
        .ok_or_else(|| {
            TerrainError::corrupt_surface(
                ARTIFACT_KIND,
                path.display(),
                "face identity exceeds u32",
            )
        })?;
    let vertices = raw.map(|value| {
        SurfaceVertexId::from_zero_based(usize::try_from(value - 1).unwrap_or(usize::MAX))
            .expect("validated vertex identity")
    });
    Ok(SurfaceFace::new(id, vertices))
}

struct SurfaceStreamPlan {
    batch_records: u64,
    verify_buffer: Vec<u8>,
}

fn stream_plan<T>(
    limits: SurfaceReadLimits,
    label: &'static str,
    disk_record_bytes: u64,
    total_records: u64,
) -> Result<SurfaceStreamPlan, TerrainError> {
    if limits.max_batch_records() == 0 {
        return Err(TerrainError::resource(label, 1, 0));
    }
    let payload_record_bytes = u64::try_from(mem::size_of::<T>()).unwrap_or(u64::MAX);
    require_within(
        "Surface batch payload bytes",
        payload_record_bytes,
        limits.max_batch_payload_bytes(),
    )?;
    require_within(
        "Surface read verification buffer bytes",
        1,
        limits.max_verify_buffer_bytes(),
    )?;
    let minimum_working_bytes = payload_record_bytes.saturating_add(1);
    require_within(
        "Surface read working bytes",
        minimum_working_bytes,
        limits.max_working_bytes(),
    )?;
    let requested_verify_buffer_bytes = limits
        .max_verify_buffer_bytes()
        .min(RECORDS_PER_BLOCK.saturating_mul(disk_record_bytes))
        .min(limits.max_working_bytes() - payload_record_bytes)
        .max(1);
    let verify_buffer = verification_buffer(
        requested_verify_buffer_bytes,
        RECORDS_PER_BLOCK.saturating_mul(disk_record_bytes),
    )?;
    let verify_capacity_bytes = u64::try_from(verify_buffer.capacity()).unwrap_or(u64::MAX);
    require_within(
        "Surface read verification buffer bytes",
        verify_capacity_bytes,
        limits.max_verify_buffer_bytes(),
    )?;
    let payload_records = limits.max_batch_payload_bytes() / payload_record_bytes;
    let working_records = limits
        .max_working_bytes()
        .saturating_sub(verify_capacity_bytes)
        / payload_record_bytes;
    let batch_records = limits
        .max_batch_records()
        .min(payload_records)
        .min(working_records)
        .min(u64::try_from(usize::MAX).unwrap_or(u64::MAX));
    if batch_records == 0 {
        return Err(TerrainError::resource(
            "Surface read working bytes",
            minimum_working_bytes,
            limits.max_working_bytes(),
        ));
    }
    let _capacity_probe = allocate_surface_batch::<T>(
        usize::try_from(batch_records).expect("batch count fits usize"),
        limits.max_batch_payload_bytes(),
        limits.max_working_bytes(),
        verify_buffer.capacity(),
        "Surface batch payload bytes",
    )?;
    let required_work = complete_stream_work_units(total_records, batch_records)?;
    require_within(
        "Surface read work units",
        required_work,
        limits.max_work_units(),
    )?;
    Ok(SurfaceStreamPlan {
        batch_records,
        verify_buffer,
    })
}

fn allocate_surface_batch<T>(
    records: usize,
    max_payload_bytes: u64,
    max_working_bytes: u64,
    verify_capacity: usize,
    label: &'static str,
) -> Result<Vec<T>, TerrainError> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(records)
        .map_err(|_| TerrainError::resource(label, u64::MAX, max_payload_bytes))?;
    let capacity_bytes = u64::try_from(result.capacity())
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(mem::size_of::<T>()).unwrap_or(u64::MAX));
    require_within(label, capacity_bytes, max_payload_bytes)?;
    let working_bytes =
        capacity_bytes.saturating_add(u64::try_from(verify_capacity).unwrap_or(u64::MAX));
    require_within(
        "Surface read working bytes",
        working_bytes,
        max_working_bytes,
    )?;
    Ok(result)
}

fn complete_stream_work_units(total_records: u64, batch_records: u64) -> Result<u64, TerrainError> {
    if total_records == 0 {
        return Ok(0);
    }
    let full_blocks = total_records / RECORDS_PER_BLOCK;
    let partial_records = total_records % RECORDS_PER_BLOCK;
    let full_span = full_blocks
        .checked_mul(RECORDS_PER_BLOCK)
        .ok_or_else(work_units_overflow)?;
    let full_block_batch_intersections = if full_blocks == 0 {
        0
    } else {
        let boundary_period =
            batch_records / greatest_common_divisor(batch_records, RECORDS_PER_BLOCK);
        let internal_boundaries = full_blocks - 1;
        let non_aligned_boundaries =
            internal_boundaries.saturating_sub(internal_boundaries / boundary_period);
        ceil_div(full_span, batch_records)
            .checked_add(non_aligned_boundaries)
            .ok_or_else(work_units_overflow)?
    };
    let full_block_work = full_block_batch_intersections
        .checked_mul(RECORDS_PER_BLOCK)
        .ok_or_else(work_units_overflow)?;
    let partial_work = if partial_records == 0 {
        0
    } else {
        let intersections = ceil_div(total_records, batch_records)
            .checked_sub(full_span / batch_records)
            .ok_or_else(work_units_overflow)?;
        intersections
            .checked_mul(partial_records)
            .ok_or_else(work_units_overflow)?
    };
    total_records
        .checked_add(full_block_work)
        .and_then(|work| work.checked_add(partial_work))
        .ok_or_else(work_units_overflow)
}

const fn greatest_common_divisor(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

const fn ceil_div(value: u64, divisor: u64) -> u64 {
    let quotient = value / divisor;
    if value.is_multiple_of(divisor) {
        quotient
    } else {
        quotient + 1
    }
}

fn work_units_overflow() -> TerrainError {
    TerrainError::resource("Surface read work units", u64::MAX, u64::MAX - 1)
}

fn batch_work_units(
    total_records: u64,
    first_record: u64,
    record_count: u64,
) -> Result<u64, TerrainError> {
    let last_record = first_record
        .checked_add(record_count)
        .and_then(|end| end.checked_sub(1))
        .ok_or_else(|| TerrainError::resource("Surface read work units", u64::MAX, u64::MAX - 1))?;
    let first_block_record = (first_record / RECORDS_PER_BLOCK)
        .checked_mul(RECORDS_PER_BLOCK)
        .ok_or_else(|| TerrainError::resource("Surface read work units", u64::MAX, u64::MAX - 1))?;
    let verified_end = (last_record / RECORDS_PER_BLOCK)
        .checked_add(1)
        .and_then(|block| block.checked_mul(RECORDS_PER_BLOCK))
        .map(|end| end.min(total_records))
        .ok_or_else(|| TerrainError::resource("Surface read work units", u64::MAX, u64::MAX - 1))?;
    record_count
        .checked_add(verified_end.saturating_sub(first_block_record))
        .ok_or_else(|| TerrainError::resource("Surface read work units", u64::MAX, u64::MAX - 1))
}

fn encode_provenance(bytes: &mut Vec<u8>, provenance: SnapshotProvenance) {
    bytes.extend_from_slice(provenance.workspace().as_bytes());
    bytes.extend_from_slice(provenance.source().as_bytes());
    bytes.extend_from_slice(provenance.revision().as_bytes());
}

fn encode_required_recipe_bounds(
    bytes: &mut Vec<u8>,
    recipe: TerrainRecipe,
) -> Result<(), TerrainError> {
    let bounds = recipe.bounds().ok_or_else(|| {
        TerrainError::invalid(
            "persistent Terrain Recipe bounds",
            "durable Surface preparation requires one explicit inclusive AOI",
        )
    })?;
    encode_bounds(bytes, bounds);
    Ok(())
}

fn encode_bounds(bytes: &mut Vec<u8>, bounds: WorldBounds) {
    for value in bounds.min().into_iter().chain(bounds.max()) {
        push_u64(bytes, canonical_f64_bits(value));
    }
}

fn encode_transform(bytes: &mut Vec<u8>, transform: PositionTransform) {
    for value in transform.offset().into_iter().chain(transform.scale()) {
        push_u64(bytes, value.to_bits());
    }
}

fn transform_bits_equal(left: PositionTransform, right: PositionTransform) -> bool {
    left.offset().map(f64::to_bits) == right.offset().map(f64::to_bits)
        && left.scale().map(f64::to_bits) == right.scale().map(f64::to_bits)
}

fn encode_input_vertex(vertex: InputVertex) -> [u8; 32] {
    let mut bytes = [0; 32];
    bytes[0..8].copy_from_slice(&vertex.point.ordinal().to_le_bytes());
    for (axis, tick) in vertex.ticks.into_iter().enumerate() {
        let start = 8 + axis * 8;
        bytes[start..start + 8].copy_from_slice(&tick.to_le_bytes());
    }
    bytes
}

fn ticks_from_input_bytes(bytes: &[u8; 32]) -> [i64; 3] {
    [
        i64::from_le_bytes(bytes[8..16].try_into().expect("fixed slice")),
        i64::from_le_bytes(bytes[16..24].try_into().expect("fixed slice")),
        i64::from_le_bytes(bytes[24..32].try_into().expect("fixed slice")),
    ]
}

fn encode_surface_vertex(vertex: SurfaceVertex) -> [u8; 32] {
    let input = InputVertex {
        point: vertex.point(),
        ticks: vertex.ticks(),
    };
    encode_input_vertex(input)
}

fn encode_surface_face(face: SurfaceFace) -> [u8; 12] {
    let mut bytes = [0; 12];
    for (index, vertex) in face.vertices().into_iter().enumerate() {
        let start = index * 4;
        bytes[start..start + 4].copy_from_slice(&vertex.get().to_le_bytes());
    }
    bytes
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn pad_header(
    bytes: &mut Vec<u8>,
    expected_bytes: u64,
    kind: &'static str,
) -> Result<(), TerrainError> {
    let expected = usize::try_from(expected_bytes).expect("small fixed header");
    if bytes.len() > expected {
        return Err(TerrainError::topology(format!(
            "{kind} header exceeds its fixed disk-v1 width"
        )));
    }
    bytes.resize(expected, 0);
    Ok(())
}

fn checksum_hasher(domain: &[u8]) -> Hasher {
    let mut hasher = Hasher::new();
    hasher.update(
        &u64::try_from(domain.len())
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    hasher.update(domain);
    hasher
}

fn write_record_block_directory<T: Copy, const N: usize>(
    file: &mut File,
    whole_file_hasher: &mut Hasher,
    records: &[T],
    encode: fn(T) -> [u8; N],
    domain: &[u8],
    path: &Path,
    control: &OperationControl,
) -> Result<(), TerrainError> {
    for (index, block) in records.chunks(RECORDS_PER_BLOCK_USIZE).enumerate() {
        control.check_cancelled()?;
        let checksum = record_block_checksum(
            domain,
            u64::try_from(index).unwrap_or(u64::MAX),
            block,
            encode,
        );
        write_hashed(file, whole_file_hasher, &checksum, path)?;
    }
    Ok(())
}

fn record_block_checksum<T: Copy, const N: usize>(
    domain: &[u8],
    block_index: u64,
    records: &[T],
    encode: fn(T) -> [u8; N],
) -> [u8; 32] {
    let mut hasher = block_hasher(
        domain,
        block_index,
        u64::try_from(records.len()).unwrap_or(u64::MAX),
    );
    for record in records {
        hasher.update(&encode(*record));
    }
    *hasher.finalize().as_bytes()
}

fn block_hasher(domain: &[u8], block_index: u64, record_count: u64) -> Hasher {
    let mut hasher = checksum_hasher(domain);
    hasher.update(&block_index.to_le_bytes());
    hasher.update(&record_count.to_le_bytes());
    hasher
}

fn hull_count(vertices: u64, faces: u64) -> Option<u64> {
    vertices
        .checked_mul(2)?
        .checked_sub(2)?
        .checked_sub(faces)
        .filter(|&hull| hull >= 3 && hull <= vertices)
}

const fn block_count(record_count: u64) -> u64 {
    let complete = record_count / RECORDS_PER_BLOCK;
    if record_count.is_multiple_of(RECORDS_PER_BLOCK) {
        complete
    } else {
        complete + 1
    }
}

const fn directory_bytes(block_count: u64) -> u64 {
    block_count.saturating_mul(CHECKSUM_BYTES)
}

fn require_within(label: &'static str, required: u64, allowed: u64) -> Result<(), TerrainError> {
    if required > allowed {
        return Err(TerrainError::resource(label, required, allowed));
    }
    Ok(())
}

fn require_path_within(path: &Path, limits: TerrainPrepareLimits) -> Result<(), TerrainError> {
    require_within(
        "Surface retained path bytes",
        path_encoded_bytes(path),
        limits.max_path_bytes(),
    )
}

fn path_exists(path: &Path) -> Result<bool, TerrainError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(TerrainError::io(
            "inspect durable Surface path",
            path.display(),
            error,
        )),
    }
}

fn path_exists_in(parent: &DirectoryWitness, path: &Path) -> Result<bool, TerrainError> {
    parent.verify()?;
    let exists = path_exists(path)?;
    parent.verify()?;
    Ok(exists)
}

#[derive(Debug)]
enum DescriptorPublicationError {
    Io(io::Error),
    Terrain(TerrainError),
}

impl From<io::Error> for DescriptorPublicationError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<TerrainError> for DescriptorPublicationError {
    fn from(error: TerrainError) -> Self {
        Self::Terrain(error)
    }
}

struct DirectoryWitness {
    path: PathBuf,
    metadata: fs::Metadata,
    #[cfg(unix)]
    directory: File,
}

impl DirectoryWitness {
    fn capture(target: &Path) -> Result<Self, TerrainError> {
        let path = target_parent(target);
        let initial = fs::symlink_metadata(path).map_err(|error| {
            TerrainError::io("inspect Surface parent directory", path.display(), error)
        })?;
        if !initial.file_type().is_dir() || initial.file_type().is_symlink() {
            return Err(TerrainError::invalid(
                "Surface target parent",
                "target parent must be an existing non-symbolic-link directory",
            ));
        }
        maybe_injected_open_race();
        #[cfg(unix)]
        {
            let directory = File::from(
                rustix::fs::open(
                    path,
                    rustix::fs::OFlags::RDONLY
                        | rustix::fs::OFlags::CLOEXEC
                        | rustix::fs::OFlags::DIRECTORY
                        | rustix::fs::OFlags::NOFOLLOW
                        | rustix::fs::OFlags::NONBLOCK,
                    rustix::fs::Mode::empty(),
                )
                .map_err(|error| {
                    TerrainError::io(
                        "open Surface parent directory",
                        path.display(),
                        error.into(),
                    )
                })?,
            );
            let opened = directory.metadata().map_err(|error| {
                TerrainError::io("inspect opened Surface parent", path.display(), error)
            })?;
            let current = fs::symlink_metadata(path).map_err(|error| {
                TerrainError::io("reinspect Surface parent directory", path.display(), error)
            })?;
            if !opened.file_type().is_dir()
                || !current.file_type().is_dir()
                || !same_file_identity(&initial, &opened)
                || !same_file_identity(&opened, &current)
            {
                return Err(changed_parent(path));
            }
            Ok(Self {
                path: path.to_path_buf(),
                metadata: opened,
                directory,
            })
        }
        #[cfg(not(unix))]
        {
            Ok(Self {
                path: path.to_path_buf(),
                metadata: initial,
            })
        }
    }

    fn verify(&self) -> Result<(), TerrainError> {
        let current = fs::symlink_metadata(&self.path).map_err(|error| {
            TerrainError::io(
                "reinspect Surface parent directory",
                self.path.display(),
                error,
            )
        })?;
        if !current.file_type().is_dir()
            || current.file_type().is_symlink()
            || !same_file_identity(&self.metadata, &current)
        {
            return Err(changed_parent(&self.path));
        }
        #[cfg(unix)]
        {
            let opened = self.directory.metadata().map_err(|error| {
                TerrainError::io("inspect opened Surface parent", self.path.display(), error)
            })?;
            if !opened.file_type().is_dir() || !same_file_identity(&self.metadata, &opened) {
                return Err(changed_parent(&self.path));
            }
        }
        Ok(())
    }

    fn open_child(&self, path: &Path) -> io::Result<File> {
        let name = child_name(path)?;
        #[cfg(unix)]
        {
            use rustix::fs::{Mode, OFlags, openat};

            let file = openat(
                &self.directory,
                name,
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
                Mode::empty(),
            )?;
            Ok(File::from(file))
        }
        #[cfg(not(unix))]
        {
            self.verify().map_err(terrain_error_as_io)?;
            let file = OpenOptions::new().read(true).open(self.path.join(name))?;
            self.verify().map_err(terrain_error_as_io)?;
            Ok(file)
        }
    }

    fn create_child(&self, path: &Path) -> io::Result<File> {
        let name = child_name(path)?;
        #[cfg(unix)]
        {
            use rustix::fs::{Mode, OFlags, openat};

            let file = openat(
                &self.directory,
                name,
                OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::RUSR | Mode::WUSR | Mode::RGRP | Mode::WGRP | Mode::ROTH | Mode::WOTH,
            )?;
            Ok(File::from(file))
        }
        #[cfg(not(unix))]
        {
            self.verify().map_err(terrain_error_as_io)?;
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(self.path.join(name))?;
            self.verify().map_err(terrain_error_as_io)?;
            Ok(file)
        }
    }

    fn publish_open_file(
        &self,
        source: &File,
        target: &Path,
        expected_complete_checksum: ContentHash,
        limits: TerrainPrepareLimits,
        control: &OperationControl,
    ) -> Result<(), DescriptorPublicationError> {
        let target_name = child_name(target)?;
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;

            use rustix::fs::{AtFlags, Mode, OFlags, copy_file_range, linkat, openat};

            let source_bytes = source.metadata()?.len();
            let anonymous = openat(
                &self.directory,
                ".",
                OFlags::RDWR | OFlags::CLOEXEC | OFlags::TMPFILE,
                Mode::RUSR | Mode::WUSR | Mode::RGRP | Mode::WGRP | Mode::ROTH | Mode::WOTH,
            )
            .map_err(io::Error::from)?;
            let mut anonymous = File::from(anonymous);
            let mut source_offset = 0_u64;
            let mut target_offset = 0_u64;
            while target_offset < source_bytes {
                control.check_cancelled().map_err(TerrainError::from)?;
                let remaining = source_bytes - target_offset;
                let requested =
                    usize::try_from(remaining.min(8 * 1024 * 1024)).unwrap_or(usize::MAX);
                let copied = copy_file_range(
                    source,
                    Some(&mut source_offset),
                    &anonymous,
                    Some(&mut target_offset),
                    requested,
                )
                .map_err(io::Error::from)?;
                if copied == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "Surface stage ended while copying to an anonymous publication file",
                    )
                    .into());
                }
            }
            anonymous.sync_all()?;
            let verified = verified_file(
                &mut anonymous,
                target,
                PersistedKind::Artifact,
                limits,
                limits.max_verify_buffer_bytes(),
                control,
            )?;
            if verified.bytes != source_bytes || verified.checksum != expected_complete_checksum {
                return Err(TerrainError::corrupt_surface(
                    ARTIFACT_KIND,
                    target.display(),
                    "anonymous publication bytes differ from the verified stage",
                )
                .into());
            }
            let descriptor_path = format!("/proc/self/fd/{}", anonymous.as_raw_fd());
            linkat(
                rustix::fs::CWD,
                descriptor_path,
                &self.directory,
                target_name,
                AtFlags::SYMLINK_FOLLOW,
            )
            .map_err(io::Error::from)?;
            Ok(())
        }
        #[cfg(target_os = "macos")]
        {
            rustix::fs::fclonefileat(
                source,
                &self.directory,
                target_name,
                rustix::fs::CloneFlags::NOFOLLOW,
            )
            .map_err(io::Error::from)?;
            let _ = (expected_complete_checksum, limits, control);
            Ok(())
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = (
                source,
                target_name,
                expected_complete_checksum,
                limits,
                control,
            );
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "descriptor-bound Surface publication is unavailable on this platform",
            )
            .into())
        }
    }

    fn sync(&self) -> Result<(), TerrainError> {
        self.verify()?;
        #[cfg(unix)]
        self.directory.sync_all().map_err(|error| {
            TerrainError::io("sync Surface parent directory", self.path.display(), error)
        })?;
        #[cfg(not(unix))]
        File::open(&self.path)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| {
                TerrainError::io("sync Surface parent directory", self.path.display(), error)
            })?;
        self.verify()
    }
}

fn target_parent(target: &Path) -> &Path {
    target
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn child_name(path: &Path) -> io::Result<&std::ffi::OsStr> {
    path.file_name()
        .ok_or_else(|| io::Error::other("Surface child path has no file name"))
}

fn changed_parent(path: &Path) -> TerrainError {
    TerrainError::corrupt_surface(
        ARTIFACT_KIND,
        path.display(),
        "parent directory identity changed during Surface preparation",
    )
}

#[cfg(not(unix))]
fn terrain_error_as_io(error: TerrainError) -> io::Error {
    io::Error::other(error)
}

struct OpenedRegularFile {
    file: File,
    metadata: fs::Metadata,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum DurablePathProvenance {
    OwnerNamed,
    UnprovenTarget,
}

impl DurablePathProvenance {
    fn invalid_path(self, kind: &'static str, path: &Path, reason: &'static str) -> TerrainError {
        match self {
            Self::OwnerNamed => TerrainError::corrupt_surface(kind, path.display(), reason),
            Self::UnprovenTarget => TerrainError::surface_target_conflict(path.display(), reason),
        }
    }
}

impl OpenedRegularFile {
    fn verify_binding(&self, path: &Path, kind: &'static str) -> Result<(), TerrainError> {
        verify_opened_binding(&self.file, &self.metadata, path, kind)
    }
}

fn verify_opened_binding(
    file: &File,
    initial: &fs::Metadata,
    path: &Path,
    kind: &'static str,
) -> Result<(), TerrainError> {
    let opened = file
        .metadata()
        .map_err(|error| TerrainError::io("inspect opened Surface file", path.display(), error))?;
    let current = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            TerrainError::corrupt_surface(
                kind,
                path.display(),
                "path disappeared while its open file was being verified",
            )
        } else {
            TerrainError::io("reinspect durable Surface path", path.display(), error)
        }
    })?;
    if opened.file_type().is_file()
        && current.file_type().is_file()
        && !current.file_type().is_symlink()
        && same_file_state(initial, &opened)
        && same_file_state(&opened, &current)
    {
        return Ok(());
    }
    Err(TerrainError::corrupt_surface(
        kind,
        path.display(),
        "path or file state changed while it was being verified",
    ))
}

fn path_matches_file(
    parent: &DirectoryWitness,
    path: &Path,
    file: &File,
    initial: &fs::Metadata,
) -> Result<bool, TerrainError> {
    parent.verify()?;
    let opened = file.metadata().map_err(|error| {
        TerrainError::io("inspect witnessed Surface file", path.display(), error)
    })?;
    let current = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(TerrainError::io(
                "inspect witnessed Surface path",
                path.display(),
                error,
            ));
        }
    };
    parent.verify()?;
    Ok(opened.file_type().is_file()
        && current.file_type().is_file()
        && !current.file_type().is_symlink()
        && same_file_identity(initial, &opened)
        && same_file_identity(&opened, &current))
}

fn accounted_retained_handle_bytes(
    path: &Path,
    vertex_block_capacity: usize,
    face_block_capacity: usize,
) -> u64 {
    let fixed = mem::size_of::<PreparedTerrainSurface>()
        .saturating_add(mem::size_of::<PreparedSurfaceData>())
        .saturating_add(mem::size_of::<Mutex<File>>())
        .saturating_add(mem::size_of::<VerifiedBlockChecksums>())
        .saturating_add(6_usize.saturating_mul(mem::size_of::<usize>()));
    let checksum_capacity = vertex_block_capacity
        .saturating_add(face_block_capacity)
        .saturating_mul(mem::size_of::<[u8; 32]>());
    u64::try_from(fixed)
        .unwrap_or(u64::MAX)
        .saturating_add(u64::try_from(checksum_capacity).unwrap_or(u64::MAX))
        .saturating_add(path_encoded_bytes(path))
}

fn path_encoded_bytes(path: &Path) -> u64 {
    u64::try_from(path.as_os_str().as_encoded_bytes().len()).unwrap_or(u64::MAX)
}

#[cfg(test)]
fn open_regular(path: &Path, kind: &'static str) -> Result<OpenedRegularFile, TerrainError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| TerrainError::io("inspect durable Surface file", path.display(), error))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(TerrainError::corrupt_surface(
            kind,
            path.display(),
            "path is not a regular non-symbolic-link file",
        ));
    }
    let file = File::open(path)
        .map_err(|error| TerrainError::io("open durable Surface file", path.display(), error))?;
    let opened = OpenedRegularFile { file, metadata };
    opened.verify_binding(path, kind)?;
    Ok(opened)
}

fn open_regular_in(
    parent: &DirectoryWitness,
    path: &Path,
    kind: &'static str,
    provenance: DurablePathProvenance,
) -> Result<OpenedRegularFile, TerrainError> {
    parent.verify()?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| TerrainError::io("inspect durable Surface file", path.display(), error))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(provenance.invalid_path(
            kind,
            path,
            "path is not a regular non-symbolic-link file",
        ));
    }
    maybe_injected_open_race();
    let file = parent
        .open_child(path)
        .map_err(|error| TerrainError::io("open durable Surface file", path.display(), error))?;
    let opened = OpenedRegularFile { file, metadata };
    opened.verify_binding(path, kind)?;
    parent.verify()?;
    Ok(opened)
}

fn require_recognized_artifact_target(
    file: &mut File,
    path: &Path,
    provenance: DurablePathProvenance,
) -> Result<(), TerrainError> {
    if provenance == DurablePathProvenance::OwnerNamed {
        return Ok(());
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|error| TerrainError::io("seek existing Surface target", path.display(), error))?;
    let mut magic = [0_u8; ARTIFACT_MAGIC.len()];
    match file.read_exact(&mut magic) {
        Ok(()) if magic == *ARTIFACT_MAGIC => Ok(()),
        Ok(()) => Err(TerrainError::surface_target_conflict(
            path.display(),
            "existing regular file is not a recognized Surface artifact",
        )),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
            Err(TerrainError::surface_target_conflict(
                path.display(),
                "existing regular file is too short to be a Surface artifact",
            ))
        }
        Err(error) => Err(TerrainError::io(
            "read existing Surface target",
            path.display(),
            error,
        )),
    }
}

#[cfg(unix)]
fn same_file_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(windows)]
fn same_file_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    left.volume_serial_number().is_some()
        && left.volume_serial_number() == right.volume_serial_number()
        && left.file_index().is_some()
        && left.file_index() == right.file_index()
}

#[cfg(not(any(unix, windows)))]
fn same_file_identity(_left: &fs::Metadata, _right: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn same_file_state(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    same_file_identity(left, right)
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}

#[cfg(unix)]
fn same_file_content_state(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    same_file_identity(left, right)
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
}

#[cfg(windows)]
fn same_file_state(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    same_file_identity(left, right)
        && left.len() == right.len()
        && left.creation_time() == right.creation_time()
        && left.last_write_time() == right.last_write_time()
}

#[cfg(windows)]
fn same_file_content_state(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    same_file_identity(left, right)
        && left.len() == right.len()
        && left.last_write_time() == right.last_write_time()
}

#[cfg(not(any(unix, windows)))]
fn same_file_state(_left: &fs::Metadata, _right: &fs::Metadata) -> bool {
    false
}

#[cfg(not(any(unix, windows)))]
fn same_file_content_state(_left: &fs::Metadata, _right: &fs::Metadata) -> bool {
    false
}

struct CreatedNamedCheckpoint {
    path: PathBuf,
    file: File,
    parent: Arc<DirectoryWitness>,
}

impl CreatedNamedCheckpoint {
    fn file_mut(&mut self) -> &mut File {
        &mut self.file
    }
}

struct OwnedPathWitness {
    path: PathBuf,
    file: File,
    metadata: fs::Metadata,
}

impl OwnedPathWitness {
    fn from_opened(path: PathBuf, opened: OpenedRegularFile) -> Self {
        Self {
            path,
            file: opened.file,
            metadata: opened.metadata,
        }
    }

    fn matches_metadata(&self, metadata: &fs::Metadata) -> Result<bool, TerrainError> {
        let opened = self.file.metadata().map_err(|error| {
            TerrainError::io("inspect witnessed Surface file", self.path.display(), error)
        })?;
        Ok(same_file_identity(&self.metadata, &opened) && same_file_identity(&opened, metadata))
    }

    fn byte_len(&self) -> Result<u64, TerrainError> {
        let opened = self.file.metadata().map_err(|error| {
            TerrainError::io("inspect witnessed Surface file", self.path.display(), error)
        })?;
        if !same_file_identity(&self.metadata, &opened) {
            return Err(TerrainError::corrupt_surface(
                ARTIFACT_KIND,
                self.path.display(),
                "witnessed Surface file identity changed before byte accounting",
            ));
        }
        Ok(opened.len())
    }
}

fn create_named_checkpoint(
    parent: Arc<DirectoryWitness>,
    path: &Path,
    kind: &'static str,
) -> Result<CreatedNamedCheckpoint, TerrainError> {
    match parent.create_child(path) {
        Ok(file) => Ok(CreatedNamedCheckpoint {
            path: path.to_path_buf(),
            file,
            parent,
        }),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Err(
            TerrainError::stale_surface(kind, "exclusive durable checkpoint path", path.display()),
        ),
        Err(error) => Err(TerrainError::io(
            "create durable Surface checkpoint",
            path.display(),
            error,
        )),
    }
}

fn finish_named_checkpoint(
    checkpoint: CreatedNamedCheckpoint,
    kind: &'static str,
    complete_boundary: PersistenceBoundary,
    parent_sync_boundary: PersistenceBoundary,
    write_result: Result<(), TerrainError>,
) -> Result<OwnedPathWitness, TerrainError> {
    write_result?;
    let metadata = checkpoint.file.metadata().map_err(|error| {
        TerrainError::io(
            "inspect created Surface checkpoint",
            checkpoint.path.display(),
            error,
        )
    })?;
    if !path_matches_file(
        &checkpoint.parent,
        &checkpoint.path,
        &checkpoint.file,
        &metadata,
    )? {
        return Err(TerrainError::corrupt_surface(
            kind,
            checkpoint.path.display(),
            "created checkpoint path changed before completion",
        ));
    }
    maybe_injected_io(complete_boundary).map_err(|error| {
        TerrainError::io(
            "complete durable Surface checkpoint",
            checkpoint.path.display(),
            error,
        )
    })?;
    maybe_injected_io(parent_sync_boundary).map_err(|error| {
        TerrainError::io(
            "sync Surface parent directory",
            checkpoint.path.display(),
            error,
        )
    })?;
    checkpoint.parent.sync()?;
    Ok(OwnedPathWitness {
        path: checkpoint.path,
        file: checkpoint.file,
        metadata,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublicationOutcome {
    Created,
    Existing,
}

impl PublicationOutcome {
    const fn result_disposition(
        self,
        published: TerrainPrepareDisposition,
    ) -> TerrainPrepareDisposition {
        match self {
            Self::Created => published,
            Self::Existing => TerrainPrepareDisposition::Opened,
        }
    }
}

struct PublicationAttempt<'a> {
    target: &'a Path,
    expected: ExpectedBinding,
    observations: AttemptObservations,
    work: Option<OwnedPathWitness>,
    limits: TerrainPrepareLimits,
}

fn publish_verified_stage(
    parent: &DirectoryWitness,
    stage: &PreparedTerrainSurface,
    attempt: PublicationAttempt<'_>,
    control: &OperationControl,
) -> Result<PreparedTerrainSurface, TerrainError> {
    let PublicationAttempt {
        target,
        expected,
        mut observations,
        work,
        limits,
    } = attempt;
    observations = observations.with_peak_temporary_disk_bytes(publication_peak_temporary_bytes(
        observations.peak_temporary_disk_bytes,
        stage.report().artifact_bytes(),
        limits.max_temporary_bytes(),
    )?);
    let publication = publish_stage(parent, stage, target, limits, control)?;
    let target_observations =
        observations.with_disposition(publication.result_disposition(observations.disposition));
    let result = reopen_published_artifact(
        parent,
        target,
        expected,
        target_observations,
        limits,
        control,
    );
    let result = reconcile_publication(publication, target, stage.complete_checksum(), result)?;
    if publication == PublicationOutcome::Existing {
        parent.sync()?;
    }
    cleanup_after_publication(parent, stage, work);
    Ok(result)
}

fn publication_peak_temporary_bytes(
    retained_bytes: u64,
    artifact_bytes: u64,
    max_temporary_bytes: u64,
) -> Result<u64, TerrainError> {
    #[cfg(target_os = "linux")]
    let publication_copy_bytes = artifact_bytes;
    #[cfg(not(target_os = "linux"))]
    let publication_copy_bytes = {
        let _ = artifact_bytes;
        0
    };
    cumulative_temporary_bytes(retained_bytes, publication_copy_bytes, max_temporary_bytes)
}

fn cumulative_temporary_bytes(
    retained_bytes: u64,
    additional_bytes: u64,
    max_temporary_bytes: u64,
) -> Result<u64, TerrainError> {
    let required = retained_bytes.saturating_add(additional_bytes);
    require_within("Surface temporary bytes", required, max_temporary_bytes)?;
    Ok(required)
}

fn reopen_published_artifact(
    parent: &DirectoryWitness,
    target: &Path,
    expected: ExpectedBinding,
    observations: AttemptObservations,
    limits: TerrainPrepareLimits,
    control: &OperationControl,
) -> Result<PreparedTerrainSurface, TerrainError> {
    maybe_injected_io(PersistenceBoundary::TargetReadback).map_err(|error| {
        TerrainError::io("reopen published Surface target", target.display(), error)
    })?;
    let opened = open_artifact(
        parent,
        target,
        DurablePathProvenance::UnprovenTarget,
        expected,
        observations,
        limits,
        control,
    )?;
    maybe_injected_io(PersistenceBoundary::TargetRevalidation).map_err(|error| {
        TerrainError::io(
            "revalidate published Surface target",
            target.display(),
            error,
        )
    })?;
    Ok(opened)
}

fn publish_stage(
    parent: &DirectoryWitness,
    stage: &PreparedTerrainSurface,
    target: &Path,
    limits: TerrainPrepareLimits,
    control: &OperationControl,
) -> Result<PublicationOutcome, TerrainError> {
    stage.verify_path_binding(parent)?;
    let expected_complete_checksum = stage.complete_checksum();
    maybe_injected_io(PersistenceBoundary::TargetLink).map_err(|error| {
        TerrainError::io(
            "publish Surface artifact without replacement",
            target.display(),
            error,
        )
    })?;
    maybe_injected_publication_race();
    match stage.publish_open_file(parent, target, limits, control) {
        Ok(()) => {
            maybe_injected_cancellation(PersistenceBoundary::CancelAfterTargetLink, control);
            if let Err(error) = maybe_injected_io(PersistenceBoundary::TargetIdentity) {
                return Err(TerrainError::surface_publication_indeterminate(
                    target.display(),
                    expected_complete_checksum,
                    TerrainError::io(
                        "revalidate published Surface target",
                        target.display(),
                        error,
                    ),
                ));
            }
            let opened = open_regular_in(
                parent,
                target,
                ARTIFACT_KIND,
                DurablePathProvenance::UnprovenTarget,
            )
            .map_err(|error| {
                TerrainError::surface_publication_indeterminate(
                    target.display(),
                    expected_complete_checksum,
                    error,
                )
            })?;
            if opened.metadata.len() != stage.report().artifact_bytes() {
                return Err(TerrainError::surface_publication_indeterminate(
                    target.display(),
                    expected_complete_checksum,
                    TerrainError::corrupt_surface(
                        ARTIFACT_KIND,
                        target.display(),
                        "independently published target length differs from its verified stage",
                    ),
                ));
            }
            sync_file(&opened.file, target).map_err(|error| {
                TerrainError::surface_publication_indeterminate(
                    target.display(),
                    expected_complete_checksum,
                    error,
                )
            })?;
            maybe_injected_io(PersistenceBoundary::TargetParentSync)
                .map_err(|error| {
                    TerrainError::io("sync Surface parent directory", target.display(), error)
                })
                .and_then(|()| parent.sync())
                .map_err(|error| {
                    TerrainError::surface_publication_indeterminate(
                        target.display(),
                        expected_complete_checksum,
                        error,
                    )
                })?;
            Ok(PublicationOutcome::Created)
        }
        Err(DescriptorPublicationError::Io(error))
            if error.kind() == io::ErrorKind::AlreadyExists =>
        {
            Ok(PublicationOutcome::Existing)
        }
        Err(DescriptorPublicationError::Io(error)) => Err(TerrainError::io(
            "publish Surface artifact without replacement",
            target.display(),
            error,
        )),
        Err(DescriptorPublicationError::Terrain(error)) => Err(error),
    }
}

fn reconcile_publication<T>(
    outcome: PublicationOutcome,
    target: &Path,
    expected_complete_checksum: ContentHash,
    result: Result<T, TerrainError>,
) -> Result<T, TerrainError> {
    match (outcome, result) {
        (PublicationOutcome::Created, Err(error)) => {
            Err(TerrainError::surface_publication_indeterminate(
                target.display(),
                expected_complete_checksum,
                error,
            ))
        }
        (_, result) => result,
    }
}

fn cleanup_after_publication(
    parent: &DirectoryWitness,
    _stage: &PreparedTerrainSurface,
    work: Option<OwnedPathWitness>,
) {
    // Stage and work pathnames are retained. A check-then-unlink sequence can
    // delete a racing replacement, and no portable conditional unlink exists.
    drop(work);
    let _ = parent.sync();
}

fn sibling_path(target: &Path, suffix: &str) -> Result<PathBuf, TerrainError> {
    let mut name = target
        .file_name()
        .ok_or_else(|| TerrainError::invalid("Surface target", "target must have a file name"))?
        .to_os_string();
    name.push(suffix);
    Ok(target.with_file_name(name))
}

fn sync_file(file: &File, path: &Path) -> Result<(), TerrainError> {
    file.sync_all()
        .map_err(|error| TerrainError::io("sync durable Surface file", path.display(), error))
}

fn write_hashed(
    file: &mut File,
    hasher: &mut Hasher,
    bytes: &[u8],
    path: &Path,
) -> Result<(), TerrainError> {
    write_all(file, bytes, path)?;
    hasher.update(bytes);
    Ok(())
}

fn write_all(file: &mut File, bytes: &[u8], path: &Path) -> Result<(), TerrainError> {
    file.write_all(bytes)
        .map_err(|error| TerrainError::io("write durable Surface file", path.display(), error))
}

fn read_exact_vec(
    file: &mut File,
    bytes: u64,
    path: &Path,
    kind: &'static str,
) -> Result<Vec<u8>, TerrainError> {
    let count = usize::try_from(bytes).map_err(|_| {
        TerrainError::corrupt_surface(kind, path.display(), "fixed header does not fit memory")
    })?;
    let mut result = vec![0; count];
    read_exact(file, &mut result, path, kind)?;
    Ok(result)
}

fn read_exact_array<const N: usize>(
    file: &mut File,
    path: &Path,
    kind: &'static str,
) -> Result<[u8; N], TerrainError> {
    let mut result = [0; N];
    read_exact(file, &mut result, path, kind)?;
    Ok(result)
}

fn read_exact(
    file: &mut File,
    bytes: &mut [u8],
    path: &Path,
    kind: &'static str,
) -> Result<(), TerrainError> {
    file.read_exact(bytes).map_err(|error| {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            TerrainError::corrupt_surface(
                kind,
                path.display(),
                "file ended before declared content",
            )
        } else {
            TerrainError::io("read durable Surface file", path.display(), error)
        }
    })
}

#[cfg(test)]
mod tests {
    use std::{
        fs::OpenOptions,
        sync::atomic::{AtomicU64, Ordering},
    };

    use point_contracts::{
        AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
        AttributeValues, LinearUnit, PositionTransform, SpatialAxes, SpatialReferenceProvenance,
    };
    use point_index::{PrepareLimits, prepare as prepare_index};
    use point_workspace::{OpenLimits, Workspace, WorkspaceSchema, create};
    use source_memory::MemorySource;

    use super::*;

    const ARTIFACT_ALGORITHM_OFFSET: usize = 12;
    const ARTIFACT_WORKSPACE_OFFSET: usize = 32;
    const ARTIFACT_SOURCE_OFFSET: usize = 48;
    const ARTIFACT_RECIPE_GROUND_OFFSET: usize = 112;
    const ARTIFACT_TRANSFORM_OFFSET: usize = 193;
    const ARTIFACT_PROFILE_HORIZONTAL_EPSG_OFFSET: usize = 245;
    const ARTIFACT_INPUT_HASH_OFFSET: usize = 257;
    const ARTIFACT_GEOMETRY_HASH_OFFSET: usize = 289;
    const ARTIFACT_TOPOLOGY_HASH_OFFSET: usize = 321;
    const ARTIFACT_HASH_OFFSET: usize = 353;
    const ARTIFACT_INPUT_COUNT_OFFSET: usize = 385;
    const ARTIFACT_VERTEX_COUNT_OFFSET: usize = 393;
    const WORK_ALGORITHM_OFFSET: usize = 12;
    const WORK_WORKSPACE_OFFSET: usize = 24;
    const WORK_SOURCE_OFFSET: usize = 40;
    const WORK_RECIPE_GROUND_OFFSET: usize = 104;
    const WORK_TRANSFORM_OFFSET: usize = 185;
    const WORK_PROFILE_HORIZONTAL_EPSG_OFFSET: usize = 237;
    const WORK_INPUT_COUNT_OFFSET: usize = 281;
    const WORK_DIRECTORY_OFFSET_FIELD: usize = 305;
    const VERTEX_DIRECTORY_OFFSET_FIELD: usize = 497;
    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn checksum_valid_artifact_block_corruption_is_rejected() {
        let fixture = Fixture::new("artifact-block");
        let target = fixture.path("surface.pterr");
        let snapshot = fixture.workspace.head();
        let recipe = fixture.recipe();
        drop(prepare_surface(snapshot.clone(), &target, recipe).unwrap());
        let mut bytes = fs::read(&target).unwrap();
        bytes[usize::try_from(ARTIFACT_HEADER_BYTES).unwrap()] ^= 0x40;
        rewrite_checksum(&mut bytes, CHECKSUM_DOMAIN);
        fs::write(&target, &bytes).unwrap();

        let error = prepare_surface(snapshot, &target, recipe).unwrap_err();
        assert_corruption_reason(error, "record block checksum does not match");
        assert_eq!(fs::read(&target).unwrap(), bytes);
    }

    #[test]
    fn checksum_valid_artifact_input_hash_must_match_canonical_records() {
        let fixture = Fixture::new("artifact-input-hash");
        let target = fixture.path("surface.pterr");
        let snapshot = fixture.workspace.head();
        let recipe = fixture.recipe();
        let prepared = prepare_surface(snapshot.clone(), &target, recipe).unwrap();
        let descriptor = prepared.descriptor();
        let forged_input_hash = ContentHash::new([0xA5; 32]);
        let forged_artifact_hash = artifact_hash(
            descriptor.snapshot(),
            descriptor.recipe_hash(),
            descriptor.position_transform(),
            descriptor.coordinate_reference(),
            forged_input_hash,
            descriptor.geometry_hash(),
            descriptor.topology_hash(),
        );
        drop(prepared);

        let mut bytes = fs::read(&target).unwrap();
        bytes[ARTIFACT_INPUT_HASH_OFFSET..ARTIFACT_INPUT_HASH_OFFSET + 32]
            .copy_from_slice(forged_input_hash.as_bytes());
        bytes[ARTIFACT_HASH_OFFSET..ARTIFACT_HASH_OFFSET + 32]
            .copy_from_slice(forged_artifact_hash.as_bytes());
        rewrite_checksum(&mut bytes, CHECKSUM_DOMAIN);
        fs::write(&target, &bytes).unwrap();

        assert_corruption_reason(
            prepare_surface(snapshot, &target, recipe).unwrap_err(),
            "Snapshot Point content hash does not match canonical vertex records",
        );
        assert_eq!(fs::read(&target).unwrap(), bytes);
    }

    #[test]
    fn checksum_consistent_alternate_delaunay_diagonal_is_rejected() {
        let fixture = Fixture::with_ticks(
            "artifact-alternate-diagonal",
            vec![[0, 0, 0], [0, 10, 1], [10, 0, 2], [10, 10, 3]],
        );
        let target = fixture.path("surface.pterr");
        let snapshot = fixture.workspace.head();
        let recipe = fixture.recipe();
        let prepared = prepare_surface(snapshot.clone(), &target, recipe).unwrap();
        let descriptor = prepared.descriptor().clone();
        let layout =
            ArtifactLayout::new(descriptor.vertex_count(), descriptor.face_count()).unwrap();
        assert_eq!(descriptor.vertex_count(), 4);
        assert_eq!(descriptor.face_count(), 2);
        drop(prepared);

        let mut bytes = fs::read(&target).unwrap();
        let face_offset = usize::try_from(layout.face_offset).unwrap();
        for (index, face) in [[1_u32, 3, 2], [2, 3, 4]].into_iter().enumerate() {
            let offset = face_offset + index * usize::try_from(FACE_RECORD_BYTES).unwrap();
            for (vertex_index, vertex) in face.into_iter().enumerate() {
                let start = offset + vertex_index * mem::size_of::<u32>();
                bytes[start..start + mem::size_of::<u32>()].copy_from_slice(&vertex.to_le_bytes());
            }
        }
        rewrite_artifact_semantic_hashes(&mut bytes, &descriptor, layout);
        fs::write(&target, &bytes).unwrap();

        assert_corruption_reason(
            prepare_surface(snapshot, &target, recipe).unwrap_err(),
            "faces do not match the canonical Delaunay topology",
        );
        assert_eq!(fs::read(&target).unwrap(), bytes);
    }

    #[test]
    fn checksum_valid_work_block_corruption_is_rejected() {
        let fixture = Fixture::new("work-block");
        let target = fixture.path("surface.pterr");
        let snapshot = fixture.workspace.head();
        let recipe = fixture.recipe();
        let defaults = TerrainPrepareLimits::default();
        let artifact_limited = TerrainPrepareLimits::new(
            defaults.derivation(),
            defaults.max_work_bytes(),
            1,
            defaults.max_temporary_bytes(),
            defaults.max_verify_buffer_bytes(),
            defaults.max_retained_handle_bytes(),
            defaults.max_path_bytes(),
        );
        prepare(snapshot.clone(), &target, recipe, artifact_limited)
            .blocking_wait()
            .unwrap_err();
        let work = sibling_path(&target, ".surface-work-v1").unwrap();
        let mut bytes = fs::read(&work).unwrap();
        let layout = WorkLayout::new(fixture.point_count()).unwrap();
        bytes[usize::try_from(layout.record_offset + 8).unwrap()] ^= 0x01;
        rewrite_checksum(&mut bytes, WORK_CHECKSUM_DOMAIN);
        fs::write(&work, &bytes).unwrap();

        let error = prepare_surface(snapshot, &target, recipe).unwrap_err();
        assert_corruption_reason(error, "record block checksum does not match");
        assert_eq!(fs::read(&work).unwrap(), bytes);
    }

    #[test]
    fn checksum_valid_layout_and_future_version_mutations_fail_closed() {
        let fixture = Fixture::new("header-mutations");
        let recipe = fixture.recipe();
        let original = fixture.path("original.pterr");
        drop(prepare_surface(fixture.workspace.head(), &original, recipe).unwrap());

        let layout_target = fixture.path("layout.pterr");
        let mut layout_bytes = fs::read(&original).unwrap();
        let offset = u64::from_le_bytes(
            layout_bytes[VERTEX_DIRECTORY_OFFSET_FIELD..VERTEX_DIRECTORY_OFFSET_FIELD + 8]
                .try_into()
                .unwrap(),
        );
        layout_bytes[VERTEX_DIRECTORY_OFFSET_FIELD..VERTEX_DIRECTORY_OFFSET_FIELD + 8]
            .copy_from_slice(&offset.saturating_add(1).to_le_bytes());
        rewrite_checksum(&mut layout_bytes, CHECKSUM_DOMAIN);
        fs::write(&layout_target, &layout_bytes).unwrap();
        let layout_error =
            prepare_surface(fixture.workspace.head(), &layout_target, recipe).unwrap_err();
        assert_corruption_reason(
            layout_error,
            "section layout is not canonical for the declared counts",
        );

        let version_target = fixture.path("version.pterr");
        let mut version_bytes = fs::read(&original).unwrap();
        version_bytes[8..12].copy_from_slice(&2_u32.to_le_bytes());
        rewrite_checksum(&mut version_bytes, CHECKSUM_DOMAIN);
        fs::write(&version_target, &version_bytes).unwrap();
        assert!(matches!(
            prepare_surface(fixture.workspace.head(), &version_target, recipe).unwrap_err(),
            TerrainError::IncompatibleSurfaceArtifact {
                found_version: 2,
                supported_version: SURFACE_DISK_VERSION,
                ..
            }
        ));
    }

    #[test]
    fn post_open_record_mutations_fail_before_batches_are_yielded() {
        let fixture = Fixture::new("post-open-blocks");
        let recipe = fixture.recipe();
        let vertex_target = fixture.path("vertices.pterr");
        let face_target = fixture.path("faces.pterr");
        let vertices = prepare_surface(fixture.workspace.head(), &vertex_target, recipe).unwrap();
        let faces = prepare_surface(fixture.workspace.head(), &face_target, recipe).unwrap();
        let layout = ArtifactLayout::new(
            faces.descriptor().vertex_count(),
            faces.descriptor().face_count(),
        )
        .unwrap();
        flip_file_byte(&vertex_target, layout.vertex_offset);
        flip_file_byte(&face_target, layout.face_offset);

        let vertex_error = vertices
            .vertex_batches(SurfaceReadLimits::default())
            .unwrap()
            .next()
            .unwrap()
            .unwrap_err();
        let face_error = faces
            .face_batches(SurfaceReadLimits::default())
            .unwrap()
            .next()
            .unwrap()
            .unwrap_err();
        assert_corruption_reason(
            vertex_error,
            "artifact file state changed after complete open verification",
        );
        assert_corruption_reason(
            face_error,
            "artifact file state changed after complete open verification",
        );
    }

    #[test]
    fn record_capture_to_decode_mutations_are_detected_for_vertex_and_face_streams() {
        let fixture = Fixture::new("stream-capture-race");
        let recipe = fixture.recipe();
        let vertex_target = fixture.path("vertices.pterr");
        let face_target = fixture.path("faces.pterr");
        let vertices = prepare_surface(fixture.workspace.head(), &vertex_target, recipe).unwrap();
        let faces = prepare_surface(fixture.workspace.head(), &face_target, recipe).unwrap();
        let layout = ArtifactLayout::new(
            faces.descriptor().vertex_count(),
            faces.descriptor().face_count(),
        )
        .unwrap();

        install_stream_mutation(StreamReadBoundary::VertexRecordCaptured, {
            let target = vertex_target.clone();
            move || flip_file_byte(&target, layout.vertex_offset)
        });
        let vertex_error = vertices
            .vertex_batches(SurfaceReadLimits::default())
            .unwrap()
            .next()
            .unwrap()
            .unwrap_err();
        assert_stream_mutation_consumed();
        assert_corruption_reason(
            vertex_error,
            "artifact file state changed after complete open verification",
        );

        install_stream_mutation(StreamReadBoundary::FaceRecordCaptured, {
            let target = face_target.clone();
            move || flip_file_byte(&target, layout.face_offset)
        });
        let face_error = faces
            .face_batches(SurfaceReadLimits::default())
            .unwrap()
            .next()
            .unwrap()
            .unwrap_err();
        assert_stream_mutation_consumed();
        assert_corruption_reason(
            face_error,
            "artifact file state changed after complete open verification",
        );
    }

    #[test]
    fn post_open_body_and_block_directory_mutations_cannot_redefine_stream_bytes() {
        let fixture = Fixture::new("post-open-body-directory");
        let recipe = fixture.recipe();
        let vertex_target = fixture.path("vertices.pterr");
        let face_target = fixture.path("faces.pterr");
        let vertices = prepare_surface(fixture.workspace.head(), &vertex_target, recipe).unwrap();
        let faces = prepare_surface(fixture.workspace.head(), &face_target, recipe).unwrap();
        let layout = ArtifactLayout::new(
            faces.descriptor().vertex_count(),
            faces.descriptor().face_count(),
        )
        .unwrap();
        let vertex_modified = fs::metadata(&vertex_target).unwrap().modified().unwrap();
        let face_modified = fs::metadata(&face_target).unwrap().modified().unwrap();

        let mut vertex_bytes = fs::read(&vertex_target).unwrap();
        vertex_bytes[usize::try_from(layout.vertex_offset).unwrap()] ^= 0x80;
        rewrite_record_checksums(
            &mut vertex_bytes,
            layout.vertex_offset,
            vertices.descriptor().vertex_count(),
            VERTEX_RECORD_BYTES,
            layout.vertex_directory_offset,
            VERTEX_BLOCK_DOMAIN,
        );
        fs::write(&vertex_target, vertex_bytes).unwrap();
        OpenOptions::new()
            .write(true)
            .open(&vertex_target)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(vertex_modified))
            .unwrap();

        let mut face_bytes = fs::read(&face_target).unwrap();
        let face_start = usize::try_from(layout.face_offset).unwrap();
        for byte in 0..mem::size_of::<u32>() {
            face_bytes.swap(
                face_start + mem::size_of::<u32>() + byte,
                face_start + 2 * mem::size_of::<u32>() + byte,
            );
        }
        rewrite_record_checksums(
            &mut face_bytes,
            layout.face_offset,
            faces.descriptor().face_count(),
            FACE_RECORD_BYTES,
            layout.face_directory_offset,
            FACE_BLOCK_DOMAIN,
        );
        fs::write(&face_target, face_bytes).unwrap();
        OpenOptions::new()
            .write(true)
            .open(&face_target)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(face_modified))
            .unwrap();

        let vertex_error = vertices
            .vertex_batches(SurfaceReadLimits::default())
            .unwrap()
            .next()
            .unwrap()
            .unwrap_err();
        let face_error = faces
            .face_batches(SurfaceReadLimits::default())
            .unwrap()
            .next()
            .unwrap()
            .unwrap_err();
        assert_corruption_reason(vertex_error, "record block checksum does not match");
        assert_corruption_reason(face_error, "record block checksum does not match");
    }

    #[cfg(unix)]
    #[test]
    fn opened_file_binding_detects_a_same_length_path_replacement() {
        let fixture = Fixture::new("path-replacement");
        let path = fixture.path("plain.bin");
        let moved = fixture.path("moved.bin");
        fs::write(&path, b"original").unwrap();
        let opened = open_regular(&path, ARTIFACT_KIND).unwrap();
        fs::rename(&path, &moved).unwrap();
        fs::write(&path, b"replaced").unwrap();

        assert!(matches!(
            opened.verify_binding(&path, ARTIFACT_KIND).unwrap_err(),
            TerrainError::CorruptSurfaceArtifact { .. }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn raced_fifo_leaf_is_rejected_without_blocking() {
        let fixture = Fixture::new("raced-fifo-leaf");
        let target = fixture.path("surface.pterr");
        fs::write(&target, b"regular").unwrap();
        let parent = DirectoryWitness::capture(&target).unwrap();
        install_open_race({
            let target = target.clone();
            move || {
                fs::remove_file(&target).unwrap();
                let status = std::process::Command::new("mkfifo")
                    .arg(&target)
                    .status()
                    .unwrap();
                assert!(status.success(), "mkfifo must create the raced leaf");
            }
        });

        let error = open_regular_in(
            &parent,
            &target,
            ARTIFACT_KIND,
            DurablePathProvenance::OwnerNamed,
        )
        .err()
        .expect("a raced FIFO must be rejected");

        assert_open_race_consumed();
        assert_corruption_reason(
            error,
            "path or file state changed while it was being verified",
        );
    }

    #[cfg(unix)]
    #[test]
    fn raced_fifo_parent_is_rejected_without_blocking() {
        let fixture = Fixture::new("raced-fifo-parent");
        let parent_path = fixture.path("parent");
        let target = parent_path.join("surface.pterr");
        fs::create_dir(&parent_path).unwrap();
        install_open_race({
            let parent_path = parent_path.clone();
            move || {
                fs::remove_dir(&parent_path).unwrap();
                let status = std::process::Command::new("mkfifo")
                    .arg(&parent_path)
                    .status()
                    .unwrap();
                assert!(status.success(), "mkfifo must create the raced parent");
            }
        });

        let error = DirectoryWitness::capture(&target)
            .err()
            .expect("a raced FIFO parent must be rejected");

        assert_open_race_consumed();
        assert!(matches!(
            error,
            TerrainError::Io {
                operation: "open Surface parent directory",
                ..
            }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn owned_work_byte_accounting_uses_the_open_inode_after_path_replacement() {
        let fixture = Fixture::new("witness-byte-accounting");
        let path = fixture.path("surface.pterr.surface-work-v1");
        let moved = fixture.path("owned-original.surface-work-v1");
        fs::write(&path, vec![0x5A; 4_096]).unwrap();
        let opened = open_regular(&path, WORK_KIND).unwrap();
        let witness = OwnedPathWitness::from_opened(path.clone(), opened);
        fs::rename(&path, &moved).unwrap();
        fs::write(&path, b"racing replacement").unwrap();

        assert_eq!(witness.byte_len().unwrap(), 4_096);
        assert_eq!(fs::metadata(&path).unwrap().len(), 18);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn descriptor_bound_publication_ignores_a_racing_source_path_replacement() {
        let fixture = Fixture::new("descriptor-publication");
        let source_path = fixture.path("source.pterr");
        let target = fixture.path("published.pterr");
        let displaced = fixture.path("displaced-source.pterr");
        let source = run_prepare_direct(&fixture, &source_path, OperationControl::new()).unwrap();
        let expected = fs::read(&source_path).unwrap();
        fs::rename(&source_path, &displaced).unwrap();
        fs::write(&source_path, b"racing replacement").unwrap();
        let parent = DirectoryWitness::capture(&target).unwrap();

        source
            .publish_open_file(
                &parent,
                &target,
                TerrainPrepareLimits::default(),
                &OperationControl::new(),
            )
            .unwrap();

        assert_eq!(fs::read(&target).unwrap(), expected);
        assert_eq!(fs::read(&source_path).unwrap(), b"racing replacement");
    }

    #[test]
    fn compatible_target_winning_publication_race_is_reported_as_opened() {
        let fixture = Fixture::new("compatible-publication-race");
        let target = fixture.path("surface.pterr");
        let stage = sibling_path(&target, ".surface-stage-v1").unwrap();
        install_publication_race({
            let target = target.clone();
            move || {
                fs::copy(&stage, &target).unwrap();
            }
        });

        let prepared = run_prepare_direct(&fixture, &target, OperationControl::new()).unwrap();

        assert_publication_race_consumed();
        assert_eq!(
            prepared.report().disposition(),
            TerrainPrepareDisposition::Opened
        );
        assert_eq!(
            prepared.report().source_points_read(),
            fixture.point_count()
        );
    }

    #[test]
    fn cold_prepare_retains_only_the_exact_named_checkpoints() {
        let fixture = Fixture::new("named-checkpoints");
        let target = fixture.path("surface.pterr");
        let prepared =
            prepare_surface(fixture.workspace.head(), &target, fixture.recipe()).unwrap();
        let work = sibling_path(&target, ".surface-work-v1").unwrap();
        let stage = sibling_path(&target, ".surface-stage-v1").unwrap();

        assert!(work.is_file());
        assert!(stage.is_file());
        let named_temporary_bytes =
            fs::metadata(&work).unwrap().len() + fs::metadata(&stage).unwrap().len();
        let expected_peak = publication_peak_temporary_bytes(
            named_temporary_bytes,
            prepared.report().artifact_bytes(),
            u64::MAX,
        )
        .unwrap();
        assert_eq!(prepared.report().peak_temporary_disk_bytes(), expected_peak);
        let hidden_temporary_names: Vec<_> = fs::read_dir(&fixture.directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(
            hidden_temporary_names.is_empty(),
            "hidden temporary siblings leaked: {hidden_temporary_names:?}"
        );
    }

    #[test]
    fn warm_open_enforces_derivation_count_limits_without_modifying_the_artifact() {
        let fixture = Fixture::new("warm-count-limits");
        let target = fixture.path("surface.pterr");
        let prepared =
            prepare_surface(fixture.workspace.head(), &target, fixture.recipe()).unwrap();
        let original = fs::read(&target).unwrap();
        let defaults = TerrainPrepareLimits::default();
        let derivation = defaults.derivation();

        for (max_input_points, max_vertices, max_faces, expected_limit) in [
            (
                prepared.descriptor().input_point_count() - 1,
                derivation.max_vertices(),
                derivation.max_faces(),
                "Ground Input Points",
            ),
            (
                derivation.max_input_points(),
                prepared.descriptor().vertex_count() - 1,
                derivation.max_faces(),
                "Terrain vertices",
            ),
            (
                derivation.max_input_points(),
                derivation.max_vertices(),
                prepared.descriptor().face_count() - 1,
                "Terrain faces",
            ),
        ] {
            let limits = TerrainPrepareLimits::new(
                crate::TerrainLimits::new(
                    derivation.point_rows(),
                    max_input_points,
                    max_vertices,
                    max_faces,
                    derivation.max_working_bytes(),
                    derivation.max_surface_bytes(),
                    derivation.max_work_units(),
                ),
                defaults.max_work_bytes(),
                defaults.max_artifact_bytes(),
                defaults.max_temporary_bytes(),
                defaults.max_verify_buffer_bytes(),
                defaults.max_retained_handle_bytes(),
                defaults.max_path_bytes(),
            );
            let error = prepare(fixture.workspace.head(), &target, fixture.recipe(), limits)
                .blocking_wait()
                .unwrap_err();
            let TerrainError::ResourceLimit { limit, .. } = error else {
                panic!("expected warm count ResourceLimit, got {error:?}");
            };
            assert_eq!(limit, expected_limit);
            assert_eq!(fs::read(&target).unwrap(), original);
        }
    }

    #[test]
    fn resumed_publication_accounts_for_each_live_private_artifact() {
        let fixture = Fixture::new("resumed-stage-accounting");
        let recipe = fixture.recipe();
        let baseline = fixture.path("baseline.pterr");
        drop(prepare_surface(fixture.workspace.head(), &baseline, recipe).unwrap());
        let target = fixture.path("resumed.pterr");
        let stage = sibling_path(&target, ".surface-stage-v1").unwrap();
        let work = sibling_path(&target, ".surface-work-v1").unwrap();
        fs::copy(&baseline, &stage).unwrap();
        let stage_bytes = fs::metadata(&stage).unwrap().len();
        let expected_peak =
            publication_peak_temporary_bytes(stage_bytes, stage_bytes, u64::MAX).unwrap();
        let arbitrary_work = vec![0xA5; usize::try_from(stage_bytes + 1).unwrap()];
        fs::write(&work, &arbitrary_work).unwrap();
        let defaults = TerrainPrepareLimits::default();
        let limits = TerrainPrepareLimits::new(
            defaults.derivation(),
            defaults.max_work_bytes(),
            defaults.max_artifact_bytes(),
            expected_peak,
            defaults.max_verify_buffer_bytes(),
            defaults.max_retained_handle_bytes(),
            defaults.max_path_bytes(),
        );

        let opened = prepare(fixture.workspace.head(), &target, recipe, limits)
            .blocking_wait()
            .unwrap();
        assert_eq!(
            opened.report().disposition(),
            TerrainPrepareDisposition::ResumedPublication
        );
        assert_eq!(opened.report().peak_temporary_disk_bytes(), expected_peak);
        assert_eq!(fs::read(&work).unwrap(), arbitrary_work);
        assert_eq!(fs::read(&target).unwrap(), fs::read(&baseline).unwrap());
    }

    #[test]
    fn publication_copy_respects_the_inclusive_temporary_byte_ceiling() {
        assert_eq!(cumulative_temporary_bytes(13, 8, 21).unwrap(), 21);
        let error = cumulative_temporary_bytes(13, 8, 20).unwrap_err();
        assert!(matches!(
            error,
            TerrainError::ResourceLimit {
                limit: "Surface temporary bytes",
                required: 21,
                allowed: 20,
            }
        ));
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn checksum_refreshed_algorithm_transform_and_reference_bindings_are_stale() {
        let fixture = Fixture::new("stale-header-bindings");
        let snapshot = fixture.workspace.head();
        let recipe = fixture.recipe();
        let original_target = fixture.path("original.pterr");
        drop(prepare_surface(snapshot.clone(), &original_target, recipe).unwrap());
        let artifact = fs::read(&original_target).unwrap();
        let work = fs::read(sibling_path(&original_target, ".surface-work-v1").unwrap()).unwrap();

        for (label, offset, replacement, binding) in [
            (
                "algorithm",
                ARTIFACT_ALGORITHM_OFFSET,
                crate::ALGORITHM_VERSION
                    .saturating_add(1)
                    .to_le_bytes()
                    .to_vec(),
                "terrain algorithm version",
            ),
            (
                "workspace",
                ARTIFACT_WORKSPACE_OFFSET,
                vec![0xA5],
                "Workspace identity",
            ),
            (
                "source",
                ARTIFACT_SOURCE_OFFSET,
                vec![0xA5],
                "Source identity",
            ),
            (
                "recipe",
                ARTIFACT_RECIPE_GROUND_OFFSET,
                vec![3],
                "Terrain Recipe",
            ),
            (
                "transform",
                ARTIFACT_TRANSFORM_OFFSET,
                0.25_f64.to_bits().to_le_bytes().to_vec(),
                "position transform",
            ),
            (
                "signed-zero-transform",
                ARTIFACT_TRANSFORM_OFFSET,
                (-0.0_f64).to_bits().to_le_bytes().to_vec(),
                "position transform",
            ),
            (
                "reference",
                ARTIFACT_PROFILE_HORIZONTAL_EPSG_OFFSET,
                32_648_u32.to_le_bytes().to_vec(),
                "spatial reference",
            ),
        ] {
            let target = fixture.path(&format!("artifact-{label}.pterr"));
            let mut mutated = artifact.clone();
            if binding.ends_with("identity") {
                mutated[offset] ^= 0xFF;
            } else {
                mutated[offset..offset + replacement.len()].copy_from_slice(&replacement);
            }
            rewrite_checksum(&mut mutated, CHECKSUM_DOMAIN);
            fs::write(&target, &mutated).unwrap();
            let error = prepare_surface(snapshot.clone(), &target, recipe).unwrap_err();
            assert_stale_binding(&error, ARTIFACT_KIND, binding);
            assert_eq!(fs::read(&target).unwrap(), mutated);
        }

        for (label, offset, replacement, binding) in [
            (
                "algorithm",
                WORK_ALGORITHM_OFFSET,
                crate::ALGORITHM_VERSION
                    .saturating_add(1)
                    .to_le_bytes()
                    .to_vec(),
                "terrain algorithm version",
            ),
            (
                "workspace",
                WORK_WORKSPACE_OFFSET,
                vec![0xA5],
                "Workspace identity",
            ),
            ("source", WORK_SOURCE_OFFSET, vec![0xA5], "Source identity"),
            (
                "recipe",
                WORK_RECIPE_GROUND_OFFSET,
                vec![3],
                "Terrain Recipe",
            ),
            (
                "transform",
                WORK_TRANSFORM_OFFSET,
                0.25_f64.to_bits().to_le_bytes().to_vec(),
                "position transform",
            ),
            (
                "signed-zero-transform",
                WORK_TRANSFORM_OFFSET,
                (-0.0_f64).to_bits().to_le_bytes().to_vec(),
                "position transform",
            ),
            (
                "reference",
                WORK_PROFILE_HORIZONTAL_EPSG_OFFSET,
                32_648_u32.to_le_bytes().to_vec(),
                "spatial reference",
            ),
        ] {
            let target = fixture.path(&format!("work-{label}.pterr"));
            let work_path = sibling_path(&target, ".surface-work-v1").unwrap();
            let mut mutated = work.clone();
            if binding.ends_with("identity") {
                mutated[offset] ^= 0xFF;
            } else {
                mutated[offset..offset + replacement.len()].copy_from_slice(&replacement);
            }
            rewrite_checksum(&mut mutated, WORK_CHECKSUM_DOMAIN);
            fs::write(&work_path, &mutated).unwrap();
            let error = prepare_surface(snapshot.clone(), &target, recipe).unwrap_err();
            assert_stale_binding(&error, WORK_KIND, binding);
            assert_eq!(fs::read(&work_path).unwrap(), mutated);
            assert!(!target.exists());
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn hostile_disk_v1_layout_order_reference_and_suffix_mutations_fail_closed() {
        let fixture = Fixture::new("hostile-disk-v1");
        let snapshot = fixture.workspace.head();
        let recipe = fixture.recipe();
        let original_target = fixture.path("original.pterr");
        drop(prepare_surface(snapshot.clone(), &original_target, recipe).unwrap());
        let artifact = fs::read(&original_target).unwrap();
        let work = fs::read(sibling_path(&original_target, ".surface-work-v1").unwrap()).unwrap();
        let layout = ArtifactLayout::new(fixture.point_count(), 6).unwrap();

        let truncated_target = fixture.path("artifact-truncated.pterr");
        fs::write(&truncated_target, &artifact[..artifact.len() - 1]).unwrap();
        assert_corrupt_and_preserved(
            prepare_surface(snapshot.clone(), &truncated_target, recipe).unwrap_err(),
            &truncated_target,
        );

        let count_target = fixture.path("artifact-count.pterr");
        let mut count_bytes = artifact.clone();
        count_bytes[ARTIFACT_INPUT_COUNT_OFFSET..ARTIFACT_INPUT_COUNT_OFFSET + 8]
            .copy_from_slice(&u64::MAX.to_le_bytes());
        count_bytes[ARTIFACT_VERTEX_COUNT_OFFSET..ARTIFACT_VERTEX_COUNT_OFFSET + 8]
            .copy_from_slice(&u64::MAX.to_le_bytes());
        rewrite_checksum(&mut count_bytes, CHECKSUM_DOMAIN);
        fs::write(&count_target, &count_bytes).unwrap();
        assert_corrupt_and_preserved(
            prepare_surface(snapshot.clone(), &count_target, recipe).unwrap_err(),
            &count_target,
        );

        let vertex_order_target = fixture.path("artifact-vertex-order.pterr");
        let mut vertex_order = artifact.clone();
        swap_records(
            &mut vertex_order,
            usize::try_from(layout.vertex_offset).unwrap(),
            usize::try_from(VERTEX_RECORD_BYTES).unwrap(),
            0,
            1,
        );
        rewrite_artifact_record_checksums(&mut vertex_order, layout);
        fs::write(&vertex_order_target, &vertex_order).unwrap();
        assert_corruption_reason(
            prepare_surface(snapshot.clone(), &vertex_order_target, recipe).unwrap_err(),
            "vertices are not in canonical order",
        );
        assert_eq!(fs::read(&vertex_order_target).unwrap(), vertex_order);

        let face_reference_target = fixture.path("artifact-face-reference.pterr");
        let mut face_reference = artifact.clone();
        let face_offset = usize::try_from(layout.face_offset).unwrap();
        face_reference[face_offset..face_offset + 4].copy_from_slice(&0_u32.to_le_bytes());
        rewrite_artifact_record_checksums(&mut face_reference, layout);
        fs::write(&face_reference_target, &face_reference).unwrap();
        assert_corruption_reason(
            prepare_surface(snapshot.clone(), &face_reference_target, recipe).unwrap_err(),
            "face contains an invalid vertex identity",
        );
        assert_eq!(fs::read(&face_reference_target).unwrap(), face_reference);

        let face_order_target = fixture.path("artifact-face-order.pterr");
        let mut face_order = artifact.clone();
        swap_records(
            &mut face_order,
            face_offset,
            usize::try_from(FACE_RECORD_BYTES).unwrap(),
            0,
            1,
        );
        rewrite_artifact_record_checksums(&mut face_order, layout);
        fs::write(&face_order_target, &face_order).unwrap();
        assert_corruption_reason(
            prepare_surface(snapshot.clone(), &face_order_target, recipe).unwrap_err(),
            "faces are not in canonical order",
        );
        assert_eq!(fs::read(&face_order_target).unwrap(), face_order);

        let face_orientation_target = fixture.path("artifact-face-orientation.pterr");
        let mut face_orientation = artifact.clone();
        for byte in 0..4 {
            face_orientation.swap(face_offset + 4 + byte, face_offset + 8 + byte);
        }
        rewrite_artifact_record_checksums(&mut face_orientation, layout);
        fs::write(&face_orientation_target, &face_orientation).unwrap();
        assert_corruption_reason(
            prepare_surface(snapshot.clone(), &face_orientation_target, recipe).unwrap_err(),
            "face orientation is not strictly counter-clockwise",
        );
        assert_eq!(
            fs::read(&face_orientation_target).unwrap(),
            face_orientation
        );

        let geometry_target = fixture.path("artifact-geometry-hash.pterr");
        let mut geometry_bytes = artifact.clone();
        let first_z = usize::try_from(layout.vertex_offset + 24).unwrap();
        geometry_bytes[first_z] ^= 0x01;
        rewrite_artifact_record_checksums(&mut geometry_bytes, layout);
        fs::write(&geometry_target, &geometry_bytes).unwrap();
        assert_corruption_reason(
            prepare_surface(snapshot.clone(), &geometry_target, recipe).unwrap_err(),
            "geometry hash does not match canonical records",
        );
        assert_eq!(fs::read(&geometry_target).unwrap(), geometry_bytes);

        let artifact_aoi_target = fixture.path("artifact-outside-aoi.pterr");
        let mut artifact_aoi = artifact.clone();
        artifact_aoi[first_z..first_z + 8].copy_from_slice(&1_000_i64.to_le_bytes());
        rewrite_artifact_record_checksums(&mut artifact_aoi, layout);
        fs::write(&artifact_aoi_target, &artifact_aoi).unwrap();
        assert_corruption_reason(
            prepare_surface(snapshot.clone(), &artifact_aoi_target, recipe).unwrap_err(),
            "vertex record lies outside the bound Terrain Recipe AOI",
        );
        assert_eq!(fs::read(&artifact_aoi_target).unwrap(), artifact_aoi);

        let work_truncated_target = fixture.path("work-truncated.pterr");
        let work_truncated_path = sibling_path(&work_truncated_target, ".surface-work-v1").unwrap();
        fs::write(&work_truncated_path, &work[..work.len() - 1]).unwrap();
        assert_corrupt_and_preserved(
            prepare_surface(snapshot.clone(), &work_truncated_target, recipe).unwrap_err(),
            &work_truncated_path,
        );
        assert!(!work_truncated_target.exists());

        let work_version_target = fixture.path("work-version.pterr");
        let work_version_path = sibling_path(&work_version_target, ".surface-work-v1").unwrap();
        let mut work_version = work.clone();
        work_version[8..12].copy_from_slice(&2_u32.to_le_bytes());
        rewrite_checksum(&mut work_version, WORK_CHECKSUM_DOMAIN);
        fs::write(&work_version_path, &work_version).unwrap();
        assert!(matches!(
            prepare_surface(snapshot.clone(), &work_version_target, recipe).unwrap_err(),
            TerrainError::IncompatibleSurfaceArtifact {
                kind: WORK_KIND,
                found_version: 2,
                supported_version: WORK_DISK_VERSION,
                ..
            }
        ));
        assert_eq!(fs::read(&work_version_path).unwrap(), work_version);

        let work_checksum_target = fixture.path("work-checksum.pterr");
        let work_checksum_path = sibling_path(&work_checksum_target, ".surface-work-v1").unwrap();
        let mut work_checksum = work.clone();
        let last = work_checksum.len() - 1;
        work_checksum[last] ^= 0x80;
        fs::write(&work_checksum_path, &work_checksum).unwrap();
        assert_corruption_reason(
            prepare_surface(snapshot.clone(), &work_checksum_target, recipe).unwrap_err(),
            "whole-file checksum does not match",
        );
        assert_eq!(fs::read(&work_checksum_path).unwrap(), work_checksum);

        let work_count_target = fixture.path("work-count.pterr");
        let work_count_path = sibling_path(&work_count_target, ".surface-work-v1").unwrap();
        let mut work_count = work.clone();
        work_count[WORK_INPUT_COUNT_OFFSET..WORK_INPUT_COUNT_OFFSET + 8]
            .copy_from_slice(&u64::MAX.to_le_bytes());
        rewrite_checksum(&mut work_count, WORK_CHECKSUM_DOMAIN);
        fs::write(&work_count_path, &work_count).unwrap();
        assert_corrupt_and_preserved(
            prepare_surface(snapshot.clone(), &work_count_target, recipe).unwrap_err(),
            &work_count_path,
        );

        let work_layout_target = fixture.path("work-layout.pterr");
        let work_layout_path = sibling_path(&work_layout_target, ".surface-work-v1").unwrap();
        let mut work_layout_bytes = work.clone();
        let directory_offset = u64::from_le_bytes(
            work_layout_bytes[WORK_DIRECTORY_OFFSET_FIELD..WORK_DIRECTORY_OFFSET_FIELD + 8]
                .try_into()
                .unwrap(),
        );
        work_layout_bytes[WORK_DIRECTORY_OFFSET_FIELD..WORK_DIRECTORY_OFFSET_FIELD + 8]
            .copy_from_slice(&directory_offset.saturating_add(1).to_le_bytes());
        rewrite_checksum(&mut work_layout_bytes, WORK_CHECKSUM_DOMAIN);
        fs::write(&work_layout_path, &work_layout_bytes).unwrap();
        assert_corruption_reason(
            prepare_surface(snapshot.clone(), &work_layout_target, recipe).unwrap_err(),
            "record or checksum-directory layout is not canonical",
        );
        assert_eq!(fs::read(&work_layout_path).unwrap(), work_layout_bytes);

        let work_content_target = fixture.path("work-content-hash.pterr");
        let work_content_path = sibling_path(&work_content_target, ".surface-work-v1").unwrap();
        let work_layout = WorkLayout::new(fixture.point_count()).unwrap();
        let mut work_content = work.clone();
        let first_work_z = usize::try_from(work_layout.record_offset + 24).unwrap();
        work_content[first_work_z] ^= 0x01;
        rewrite_work_record_checksums(&mut work_content, work_layout);
        fs::write(&work_content_path, &work_content).unwrap();
        assert_corruption_reason(
            prepare_surface(snapshot.clone(), &work_content_target, recipe).unwrap_err(),
            "Snapshot Point content hash does not match staged input records",
        );
        assert_eq!(fs::read(&work_content_path).unwrap(), work_content);

        let work_aoi_target = fixture.path("work-outside-aoi.pterr");
        let work_aoi_path = sibling_path(&work_aoi_target, ".surface-work-v1").unwrap();
        let mut work_aoi = work.clone();
        work_aoi[first_work_z..first_work_z + 8].copy_from_slice(&1_000_i64.to_le_bytes());
        rewrite_work_record_checksums(&mut work_aoi, work_layout);
        fs::write(&work_aoi_path, &work_aoi).unwrap();
        assert_corruption_reason(
            prepare_surface(snapshot.clone(), &work_aoi_target, recipe).unwrap_err(),
            "input record lies outside the bound Terrain Recipe AOI",
        );
        assert_eq!(fs::read(&work_aoi_path).unwrap(), work_aoi);

        let work_order_target = fixture.path("work-order.pterr");
        let work_order_path = sibling_path(&work_order_target, ".surface-work-v1").unwrap();
        let mut work_order = work;
        let work_layout = WorkLayout::new(fixture.point_count()).unwrap();
        swap_records(
            &mut work_order,
            usize::try_from(work_layout.record_offset).unwrap(),
            usize::try_from(VERTEX_RECORD_BYTES).unwrap(),
            0,
            1,
        );
        rewrite_work_record_checksums(&mut work_order, work_layout);
        fs::write(&work_order_path, &work_order).unwrap();
        assert_corruption_reason(
            prepare_surface(snapshot, &work_order_target, recipe).unwrap_err(),
            "input ordinals are not strictly increasing",
        );
        assert_eq!(fs::read(&work_order_path).unwrap(), work_order);
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn every_injected_durable_io_boundary_has_explicit_recovery_or_indeterminate_certainty() {
        let cases = [
            (
                PersistenceBoundary::WorkCreate,
                io::ErrorKind::Other,
                Some(TerrainPrepareDisposition::Built),
                false,
            ),
            (
                PersistenceBoundary::WorkWrite,
                io::ErrorKind::StorageFull,
                None,
                false,
            ),
            (
                PersistenceBoundary::WorkFileSync,
                io::ErrorKind::Other,
                Some(TerrainPrepareDisposition::ResumedInput),
                false,
            ),
            (
                PersistenceBoundary::WorkPublish,
                io::ErrorKind::Other,
                Some(TerrainPrepareDisposition::ResumedInput),
                false,
            ),
            (
                PersistenceBoundary::WorkParentSync,
                io::ErrorKind::Other,
                Some(TerrainPrepareDisposition::ResumedInput),
                false,
            ),
            (
                PersistenceBoundary::StageCreate,
                io::ErrorKind::Other,
                Some(TerrainPrepareDisposition::ResumedInput),
                false,
            ),
            (
                PersistenceBoundary::StageWrite,
                io::ErrorKind::StorageFull,
                None,
                false,
            ),
            (
                PersistenceBoundary::StageFileSync,
                io::ErrorKind::Other,
                Some(TerrainPrepareDisposition::ResumedPublication),
                false,
            ),
            (
                PersistenceBoundary::StagePublish,
                io::ErrorKind::Other,
                Some(TerrainPrepareDisposition::ResumedPublication),
                false,
            ),
            (
                PersistenceBoundary::StageParentSync,
                io::ErrorKind::Other,
                Some(TerrainPrepareDisposition::ResumedPublication),
                false,
            ),
            (
                PersistenceBoundary::StageReadback,
                io::ErrorKind::Other,
                Some(TerrainPrepareDisposition::ResumedPublication),
                false,
            ),
            (
                PersistenceBoundary::TargetLink,
                io::ErrorKind::Other,
                Some(TerrainPrepareDisposition::ResumedPublication),
                false,
            ),
            (
                PersistenceBoundary::TargetIdentity,
                io::ErrorKind::Other,
                Some(TerrainPrepareDisposition::Opened),
                true,
            ),
            (
                PersistenceBoundary::TargetParentSync,
                io::ErrorKind::Other,
                Some(TerrainPrepareDisposition::Opened),
                true,
            ),
            (
                PersistenceBoundary::TargetReadback,
                io::ErrorKind::Other,
                Some(TerrainPrepareDisposition::Opened),
                true,
            ),
            (
                PersistenceBoundary::TargetRevalidation,
                io::ErrorKind::Other,
                Some(TerrainPrepareDisposition::Opened),
                true,
            ),
        ];
        for (index, (boundary, kind, retry_disposition, indeterminate)) in
            cases.into_iter().enumerate()
        {
            let fixture = Fixture::new(&format!("fault-{index}"));
            let target = fixture.path("surface.pterr");
            install_io_fault(boundary, kind);
            let error = run_prepare_direct(&fixture, &target, OperationControl::new()).unwrap_err();
            assert_io_fault_consumed();
            if indeterminate {
                let TerrainError::SurfacePublicationIndeterminate {
                    expected_complete_checksum,
                    ..
                } = error
                else {
                    panic!("post-link {boundary:?} fault was not indeterminate: {error:?}");
                };
                let bytes = fs::read(&target).unwrap();
                assert_eq!(
                    expected_complete_checksum.as_bytes(),
                    &bytes[bytes.len() - usize::try_from(CHECKSUM_BYTES).unwrap()..]
                );
            } else {
                assert!(
                    matches!(error, TerrainError::Io { .. }),
                    "{boundary:?}: {error:?}"
                );
                assert!(
                    !target.exists(),
                    "{boundary:?} published a target before commit"
                );
            }
            if let Some(retry_disposition) = retry_disposition {
                let retried = prepare_surface(fixture.workspace.head(), &target, fixture.recipe())
                    .unwrap_or_else(|error| panic!("retry after {boundary:?} failed: {error:?}"));
                assert_eq!(
                    retried.report().disposition(),
                    retry_disposition,
                    "{boundary:?}"
                );
            } else {
                let torn_checkpoint = match boundary {
                    PersistenceBoundary::WorkWrite => {
                        sibling_path(&target, ".surface-work-v1").unwrap()
                    }
                    PersistenceBoundary::StageWrite => {
                        sibling_path(&target, ".surface-stage-v1").unwrap()
                    }
                    _ => unreachable!("only write faults retain torn checkpoints"),
                };
                assert!(torn_checkpoint.is_file());
                assert!(matches!(
                    prepare_surface(fixture.workspace.head(), &target, fixture.recipe()),
                    Err(TerrainError::CorruptSurfaceArtifact { .. })
                ));
            }
        }

        let work_readback = Fixture::new("fault-work-readback");
        let work_readback_target = work_readback.path("surface.pterr");
        install_cancellation(PersistenceBoundary::CancelAfterWork);
        assert!(matches!(
            run_prepare_direct(
                &work_readback,
                &work_readback_target,
                OperationControl::new()
            ),
            Err(TerrainError::Cancelled)
        ));
        assert_cancellation_consumed();
        install_io_fault(PersistenceBoundary::WorkReadback, io::ErrorKind::Other);
        assert!(matches!(
            run_prepare_direct(
                &work_readback,
                &work_readback_target,
                OperationControl::new()
            ),
            Err(TerrainError::Io { .. })
        ));
        assert_io_fault_consumed();
        assert!(!work_readback_target.exists());
        assert_eq!(
            prepare_surface(
                work_readback.workspace.head(),
                &work_readback_target,
                work_readback.recipe()
            )
            .unwrap()
            .report()
            .disposition(),
            TerrainPrepareDisposition::ResumedInput
        );
    }

    #[test]
    fn cancellation_is_observed_before_publication_and_during_warm_verification() {
        let immediate = Fixture::new("cancel-immediate");
        let immediate_target = immediate.path("surface.pterr");
        let control = OperationControl::new();
        control.cancel();
        assert!(matches!(
            run_prepare_direct(&immediate, &immediate_target, control),
            Err(TerrainError::Cancelled)
        ));
        assert!(!immediate_target.exists());

        for (index, (boundary, retry_disposition)) in [
            (
                PersistenceBoundary::CancelAfterWork,
                TerrainPrepareDisposition::ResumedInput,
            ),
            (
                PersistenceBoundary::CancelAfterStage,
                TerrainPrepareDisposition::ResumedPublication,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let fixture = Fixture::new(&format!("cancel-boundary-{index}"));
            let target = fixture.path("surface.pterr");
            install_cancellation(boundary);
            assert!(matches!(
                run_prepare_direct(&fixture, &target, OperationControl::new()),
                Err(TerrainError::Cancelled)
            ));
            assert_cancellation_consumed();
            assert!(!target.exists());
            let retried =
                prepare_surface(fixture.workspace.head(), &target, fixture.recipe()).unwrap();
            assert_eq!(retried.report().disposition(), retry_disposition);
        }

        let warm = Fixture::new("cancel-warm");
        let warm_target = warm.path("surface.pterr");
        drop(prepare_surface(warm.workspace.head(), &warm_target, warm.recipe()).unwrap());
        let original = fs::read(&warm_target).unwrap();
        install_cancellation(PersistenceBoundary::CancelWarmOpen);
        assert!(matches!(
            run_prepare_direct(&warm, &warm_target, OperationControl::new()),
            Err(TerrainError::Cancelled)
        ));
        assert_cancellation_consumed();
        assert_eq!(fs::read(&warm_target).unwrap(), original);

        let post_link = Fixture::new("cancel-post-target-link");
        let post_link_target = post_link.path("surface.pterr");
        install_cancellation(PersistenceBoundary::CancelAfterTargetLink);
        let error =
            run_prepare_direct(&post_link, &post_link_target, OperationControl::new()).unwrap_err();
        assert_cancellation_consumed();
        let TerrainError::SurfacePublicationIndeterminate { source, .. } = error else {
            panic!("post-link cancellation was not indeterminate: {error:?}");
        };
        assert!(matches!(*source, TerrainError::Cancelled));
        assert!(post_link_target.is_file());
        assert_eq!(
            prepare_surface(
                post_link.workspace.head(),
                &post_link_target,
                post_link.recipe()
            )
            .unwrap()
            .report()
            .disposition(),
            TerrainPrepareDisposition::Opened
        );
    }

    #[test]
    fn prepare_progress_is_terminal_only_after_durable_success() {
        let completed = Fixture::new("prepare-progress-completed");
        let completed_target = completed.path("surface.pterr");
        let cold_control = OperationControl::new();
        let cold_handle = cold_control.handle();
        let cold = run_prepare_direct(&completed, &completed_target, cold_control).unwrap();
        assert_eq!(
            cold.report().disposition(),
            TerrainPrepareDisposition::Built
        );
        assert_eq!(
            cold_handle.progress().phase(),
            foundation_runtime::ProgressPhase::COMPLETE
        );

        let warm_control = OperationControl::new();
        let warm_handle = warm_control.handle();
        let warm = run_prepare_direct(&completed, &completed_target, warm_control).unwrap();
        assert_eq!(
            warm.report().disposition(),
            TerrainPrepareDisposition::Opened
        );
        assert_eq!(
            warm_handle.progress().phase(),
            foundation_runtime::ProgressPhase::COMPLETE
        );

        let failed = Fixture::new("prepare-progress-failed");
        let failed_target = failed.path("surface.pterr");
        install_io_fault(PersistenceBoundary::StageCreate, io::ErrorKind::Other);
        let failed_control = OperationControl::new();
        let failed_handle = failed_control.handle();
        assert!(matches!(
            run_prepare_direct(&failed, &failed_target, failed_control),
            Err(TerrainError::Io { .. })
        ));
        assert_io_fault_consumed();
        assert_ne!(
            failed_handle.progress().phase(),
            foundation_runtime::ProgressPhase::COMPLETE
        );

        let resumed_control = OperationControl::new();
        let resumed_handle = resumed_control.handle();
        let resumed = run_prepare_direct(&failed, &failed_target, resumed_control).unwrap();
        assert_eq!(
            resumed.report().disposition(),
            TerrainPrepareDisposition::ResumedInput
        );
        assert_eq!(
            resumed_handle.progress().phase(),
            foundation_runtime::ProgressPhase::COMPLETE
        );

        let staged = Fixture::new("prepare-progress-staged");
        let staged_target = staged.path("surface.pterr");
        install_io_fault(PersistenceBoundary::TargetLink, io::ErrorKind::Other);
        let staged_failure_control = OperationControl::new();
        let staged_failure_handle = staged_failure_control.handle();
        assert!(matches!(
            run_prepare_direct(&staged, &staged_target, staged_failure_control),
            Err(TerrainError::Io { .. })
        ));
        assert_io_fault_consumed();
        assert_ne!(
            staged_failure_handle.progress().phase(),
            foundation_runtime::ProgressPhase::COMPLETE
        );

        let publication_control = OperationControl::new();
        let publication_handle = publication_control.handle();
        let publication = run_prepare_direct(&staged, &staged_target, publication_control).unwrap();
        assert_eq!(
            publication.report().disposition(),
            TerrainPrepareDisposition::ResumedPublication
        );
        assert_eq!(
            publication_handle.progress().phase(),
            foundation_runtime::ProgressPhase::COMPLETE
        );
    }

    #[test]
    fn warm_open_syncs_the_parent_before_acknowledging_success() {
        let fixture = Fixture::new("warm-parent-sync");
        let target = fixture.path("surface.pterr");
        drop(prepare_surface(fixture.workspace.head(), &target, fixture.recipe()).unwrap());
        let original = fs::read(&target).unwrap();
        install_io_fault(PersistenceBoundary::WarmParentSync, io::ErrorKind::Other);
        let error = run_prepare_direct(&fixture, &target, OperationControl::new()).unwrap_err();
        assert!(matches!(error, TerrainError::Io { .. }));
        assert_io_fault_consumed();
        assert_eq!(fs::read(&target).unwrap(), original);
        assert_eq!(
            prepare_surface(fixture.workspace.head(), &target, fixture.recipe())
                .unwrap()
                .report()
                .disposition(),
            TerrainPrepareDisposition::Opened
        );
    }

    fn assert_stale_binding(error: &TerrainError, expected_kind: &str, expected_binding: &str) {
        let TerrainError::StaleSurfaceArtifact { kind, binding, .. } = error else {
            panic!("expected stale Surface binding, got {error:?}");
        };
        assert_eq!(*kind, expected_kind);
        assert_eq!(*binding, expected_binding);
    }

    #[allow(
        clippy::needless_pass_by_value,
        reason = "the helper accepts the direct unwrap_err result and discards it after inspection"
    )]
    fn assert_corrupt_and_preserved(error: TerrainError, path: &Path) {
        assert!(
            matches!(error, TerrainError::CorruptSurfaceArtifact { .. }),
            "expected structured corruption, got {error:?}"
        );
        assert!(path.is_file(), "corrupt caller-owned file was removed");
    }

    fn swap_records(
        bytes: &mut [u8],
        section_offset: usize,
        record_bytes: usize,
        left: usize,
        right: usize,
    ) {
        let left = section_offset + left * record_bytes;
        let right = section_offset + right * record_bytes;
        for offset in 0..record_bytes {
            bytes.swap(left + offset, right + offset);
        }
    }

    fn rewrite_artifact_record_checksums(bytes: &mut [u8], layout: ArtifactLayout) {
        rewrite_record_checksums(
            bytes,
            layout.vertex_offset,
            u64::from_le_bytes(
                bytes[ARTIFACT_VERTEX_COUNT_OFFSET..ARTIFACT_VERTEX_COUNT_OFFSET + 8]
                    .try_into()
                    .unwrap(),
            ),
            VERTEX_RECORD_BYTES,
            layout.vertex_directory_offset,
            VERTEX_BLOCK_DOMAIN,
        );
        rewrite_record_checksums(
            bytes,
            layout.face_offset,
            u64::from_le_bytes(bytes[401..409].try_into().unwrap()),
            FACE_RECORD_BYTES,
            layout.face_directory_offset,
            FACE_BLOCK_DOMAIN,
        );
        rewrite_checksum(bytes, CHECKSUM_DOMAIN);
    }

    fn rewrite_artifact_semantic_hashes(
        bytes: &mut [u8],
        descriptor: &SurfaceArtifactDescriptor,
        layout: ArtifactLayout,
    ) {
        let mut geometry = domain_hasher(GEOMETRY_HASH_DOMAIN);
        let mut topology = domain_hasher(TOPOLOGY_HASH_DOMAIN);
        hash_transform(&mut geometry, descriptor.position_transform());
        geometry.update(&descriptor.vertex_count().to_le_bytes());
        topology.update(&descriptor.vertex_count().to_le_bytes());
        let vertex_offset = usize::try_from(layout.vertex_offset).unwrap();
        let vertex_record_bytes = usize::try_from(VERTEX_RECORD_BYTES).unwrap();
        for index in 0..usize::try_from(descriptor.vertex_count()).unwrap() {
            let start = vertex_offset + index * vertex_record_bytes;
            let record = &bytes[start..start + vertex_record_bytes];
            geometry.update(&u32::try_from(index + 1).unwrap().to_le_bytes());
            geometry.update(descriptor.snapshot().source().as_bytes());
            geometry.update(record);
        }
        geometry.update(&descriptor.face_count().to_le_bytes());
        topology.update(&descriptor.face_count().to_le_bytes());
        let face_offset = usize::try_from(layout.face_offset).unwrap();
        let face_record_bytes = usize::try_from(FACE_RECORD_BYTES).unwrap();
        for index in 0..usize::try_from(descriptor.face_count()).unwrap() {
            let start = face_offset + index * face_record_bytes;
            let record = &bytes[start..start + face_record_bytes];
            let face_id = u32::try_from(index + 1).unwrap().to_le_bytes();
            geometry.update(&face_id);
            geometry.update(record);
            topology.update(&face_id);
            topology.update(record);
        }
        let geometry_hash = ContentHash::new(*geometry.finalize().as_bytes());
        let topology_hash = ContentHash::new(*topology.finalize().as_bytes());
        let semantic_hash = artifact_hash(
            descriptor.snapshot(),
            descriptor.recipe_hash(),
            descriptor.position_transform(),
            descriptor.coordinate_reference(),
            descriptor.input_hash(),
            geometry_hash,
            topology_hash,
        );
        bytes[ARTIFACT_GEOMETRY_HASH_OFFSET..ARTIFACT_GEOMETRY_HASH_OFFSET + 32]
            .copy_from_slice(geometry_hash.as_bytes());
        bytes[ARTIFACT_TOPOLOGY_HASH_OFFSET..ARTIFACT_TOPOLOGY_HASH_OFFSET + 32]
            .copy_from_slice(topology_hash.as_bytes());
        bytes[ARTIFACT_HASH_OFFSET..ARTIFACT_HASH_OFFSET + 32]
            .copy_from_slice(semantic_hash.as_bytes());
        rewrite_artifact_record_checksums(bytes, layout);
    }

    fn rewrite_work_record_checksums(bytes: &mut [u8], layout: WorkLayout) {
        rewrite_record_checksums(
            bytes,
            layout.record_offset,
            u64::from_le_bytes(
                bytes[WORK_INPUT_COUNT_OFFSET..WORK_INPUT_COUNT_OFFSET + 8]
                    .try_into()
                    .unwrap(),
            ),
            VERTEX_RECORD_BYTES,
            layout.directory_offset,
            WORK_BLOCK_DOMAIN,
        );
        rewrite_checksum(bytes, WORK_CHECKSUM_DOMAIN);
    }

    fn rewrite_record_checksums(
        bytes: &mut [u8],
        record_offset: u64,
        record_count: u64,
        record_bytes: u64,
        directory_offset: u64,
        domain: &[u8],
    ) {
        for block_index in 0..block_count(record_count) {
            let first_record = block_index * RECORDS_PER_BLOCK;
            let records = (record_count - first_record).min(RECORDS_PER_BLOCK);
            let start = usize::try_from(record_offset + first_record * record_bytes).unwrap();
            let end =
                usize::try_from(record_offset + (first_record + records) * record_bytes).unwrap();
            let mut hasher = block_hasher(domain, block_index, records);
            hasher.update(&bytes[start..end]);
            let checksum = *hasher.finalize().as_bytes();
            let checksum_offset =
                usize::try_from(directory_offset + block_index * CHECKSUM_BYTES).unwrap();
            bytes[checksum_offset..checksum_offset + checksum.len()].copy_from_slice(&checksum);
        }
    }

    fn install_io_fault(boundary: PersistenceBoundary, kind: io::ErrorKind) {
        INJECTED_IO_FAULT.with(|slot| {
            let previous = slot.replace(Some((boundary, kind)));
            assert!(previous.is_none(), "test installed overlapping I/O faults");
        });
    }

    fn assert_io_fault_consumed() {
        INJECTED_IO_FAULT.with(|slot| {
            assert!(
                slot.borrow().is_none(),
                "injected I/O boundary was not reached"
            );
        });
    }

    fn install_cancellation(boundary: PersistenceBoundary) {
        INJECTED_CANCELLATION.with(|slot| {
            assert!(slot.replace(Some(boundary)).is_none());
        });
    }

    fn assert_cancellation_consumed() {
        INJECTED_CANCELLATION.with(|slot| {
            assert!(
                slot.get().is_none(),
                "injected cancellation boundary was not reached"
            );
        });
    }

    fn install_stream_mutation(boundary: StreamReadBoundary, hook: impl FnOnce() + 'static) {
        INJECTED_STREAM_MUTATION.with(|slot| {
            let previous = slot.replace(Some((boundary, Box::new(hook))));
            assert!(
                previous.is_none(),
                "test installed overlapping stream mutations"
            );
        });
    }

    fn assert_stream_mutation_consumed() {
        INJECTED_STREAM_MUTATION.with(|slot| {
            assert!(
                slot.borrow().is_none(),
                "injected stream mutation boundary was not reached"
            );
        });
    }

    fn install_open_race(hook: impl FnOnce() + 'static) {
        INJECTED_OPEN_RACE.with(|slot| {
            let previous = slot.replace(Some(Box::new(hook)));
            assert!(previous.is_none(), "test installed overlapping open races");
        });
    }

    fn assert_open_race_consumed() {
        INJECTED_OPEN_RACE.with(|slot| {
            assert!(
                slot.borrow().is_none(),
                "injected open race boundary was not reached"
            );
        });
    }

    fn install_publication_race(hook: impl FnOnce() + 'static) {
        INJECTED_PUBLICATION_RACE.with(|slot| {
            assert!(slot.replace(Some(Box::new(hook))).is_none());
        });
    }

    fn assert_publication_race_consumed() {
        INJECTED_PUBLICATION_RACE.with(|slot| {
            assert!(
                slot.borrow().is_none(),
                "injected publication race was not reached"
            );
        });
    }

    #[allow(
        clippy::needless_pass_by_value,
        reason = "tests intentionally transfer one attempt's cancellation control"
    )]
    fn run_prepare_direct(
        fixture: &Fixture,
        target: &Path,
        control: OperationControl,
    ) -> Result<PreparedTerrainSurface, TerrainError> {
        let snapshot = fixture.workspace.head();
        run_prepare(
            &snapshot,
            target,
            fixture.recipe(),
            TerrainPrepareLimits::default(),
            &control,
        )
    }

    fn prepare_surface(
        snapshot: Snapshot,
        target: &Path,
        recipe: TerrainRecipe,
    ) -> Result<PreparedTerrainSurface, TerrainError> {
        prepare(snapshot, target, recipe, TerrainPrepareLimits::default()).blocking_wait()
    }

    fn assert_corruption_reason(error: TerrainError, expected: &str) {
        let TerrainError::CorruptSurfaceArtifact { reason, .. } = error else {
            panic!("expected structured Surface corruption");
        };
        assert_eq!(reason.as_str(), expected);
    }

    fn rewrite_checksum(bytes: &mut [u8], domain: &[u8]) {
        let payload_bytes = bytes.len() - usize::try_from(CHECKSUM_BYTES).unwrap();
        let mut hasher = checksum_hasher(domain);
        hasher.update(&bytes[..payload_bytes]);
        bytes[payload_bytes..].copy_from_slice(hasher.finalize().as_bytes());
    }

    fn flip_file_byte(path: &Path, offset: u64) {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        file.seek(SeekFrom::Start(offset)).unwrap();
        let mut byte = [0];
        file.read_exact(&mut byte).unwrap();
        byte[0] ^= 0x80;
        file.seek(SeekFrom::Start(offset)).unwrap();
        file.write_all(&byte).unwrap();
        file.sync_all().unwrap();
    }

    struct Fixture {
        workspace: Workspace,
        directory: PathBuf,
        point_count: u64,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            Self::with_ticks(
                label,
                vec![
                    [0, 0, 0],
                    [10, 0, 2],
                    [10, 10, 4],
                    [0, 10, 6],
                    [5, 5, 3],
                    [3, 7, 4],
                ],
            )
        }

        fn with_ticks(label: &str, ticks: Vec<[i64; 3]>) -> Self {
            let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let directory = std::env::temp_dir().join(format!(
                "punctra-terrain-persistence-unit-{label}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&directory).unwrap();
            let point_count = u64::try_from(ticks.len()).unwrap();
            let attributes = classification_columns(vec![2; ticks.len()], ticks.len());
            let memory = MemorySource::from_columns(
                PositionTransform::new([0.0; 3], [1.0; 3]).unwrap(),
                supported_reference(),
                ticks,
                attributes,
            )
            .unwrap();
            let source = source_memory::open(memory).blocking_wait().unwrap();
            let index = prepare_index(
                source,
                directory.join("fixture.pidx"),
                PrepareLimits::default(),
            )
            .blocking_wait()
            .unwrap();
            let workspace = create(
                directory.join("fixture.pcw"),
                index,
                WorkspaceSchema::new(classification_attribute()),
                OpenLimits::default(),
            )
            .blocking_wait()
            .unwrap();
            Self {
                workspace,
                directory,
                point_count,
            }
        }

        fn path(&self, name: &str) -> PathBuf {
            self.directory.join(name)
        }

        const fn point_count(&self) -> u64 {
            self.point_count
        }

        #[allow(clippy::unused_self)]
        fn recipe(&self) -> TerrainRecipe {
            TerrainRecipe::new(2)
                .within(WorldBounds::new([-1.0, -1.0, -10.0], [11.0, 11.0, 20.0]).unwrap())
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    fn supported_reference() -> CoordinateReference {
        CoordinateReference::profile(
            SpatialReferenceProfile::new(
                32_647,
                5_703,
                SpatialAxes::EastingNorthingElevation,
                LinearUnit::Metre,
                LinearUnit::Metre,
                SpatialReferenceProvenance::CallerDeclaration,
            )
            .unwrap(),
        )
    }

    fn classification_attribute() -> AttributeId {
        AttributeId::new(301).unwrap()
    }

    fn classification_columns(values: Vec<u8>, point_count: usize) -> AttributeColumns {
        let definition = AttributeDefinition::new(
            classification_attribute(),
            "classification",
            AttributeDataType::U8,
        )
        .unwrap();
        let column = AttributeColumn::new(definition, AttributeValues::u8(values)).unwrap();
        AttributeColumns::new(vec![column], point_count).unwrap()
    }
}
