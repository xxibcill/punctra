//! Shared public-interface fixtures for Terrain Derivation evidence.

#![allow(
    dead_code,
    reason = "each integration-test binary uses a different fixture subset"
)]

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use point_contracts::{
    AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
    AttributeValues, CoordinateReference, PointId, PositionTransform,
};
use point_index::{PrepareLimits, prepare};
use point_terrain::{TerrainError, TerrainLimits, TerrainRecipe, TerrainSurface};
use point_workspace::{
    CommitOutcome, CommitReceipt, OpenLimits, OperationId, PointRowLimits, PointSet,
    PointSetLimits, Snapshot, Workspace, WorkspaceSchema, create,
};
use source_memory::MemorySource;

const ROW_PAYLOAD_BYTES: u64 = 33;
static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(1);

pub struct TerrainFixture {
    workspace: Workspace,
    ticks: Vec<[i64; 3]>,
    classifications: Vec<u8>,
    _temporary: TemporaryFixture,
}

impl TerrainFixture {
    pub fn new(label: &str, ticks: Vec<[i64; 3]>, classifications: Vec<u8>) -> Self {
        Self::with_transform(label, identity_transform(), ticks, classifications)
    }

    pub fn with_transform(
        label: &str,
        transform: PositionTransform,
        ticks: Vec<[i64; 3]>,
        classifications: Vec<u8>,
    ) -> Self {
        Self::with_transform_and_reference(
            label,
            transform,
            CoordinateReference::Unknown,
            ticks,
            classifications,
        )
    }

    pub fn with_reference(
        label: &str,
        coordinate_reference: CoordinateReference,
        ticks: Vec<[i64; 3]>,
        classifications: Vec<u8>,
    ) -> Self {
        Self::with_transform_and_reference(
            label,
            identity_transform(),
            coordinate_reference,
            ticks,
            classifications,
        )
    }

    fn with_transform_and_reference(
        label: &str,
        transform: PositionTransform,
        coordinate_reference: CoordinateReference,
        ticks: Vec<[i64; 3]>,
        classifications: Vec<u8>,
    ) -> Self {
        assert_eq!(ticks.len(), classifications.len());
        let temporary = TemporaryFixture::new(label);
        let attributes = classification_columns(classifications.clone(), ticks.len());
        let memory =
            MemorySource::from_columns(transform, coordinate_reference, ticks.clone(), attributes)
                .expect("Terrain fixture memory Source is valid");
        let source = source_memory::open(memory)
            .blocking_wait()
            .expect("Terrain fixture Source opens");
        let index = prepare(source, temporary.index_path(), PrepareLimits::default())
            .blocking_wait()
            .expect("Terrain fixture index prepares");
        let workspace = create(
            temporary.workspace_path(),
            index,
            WorkspaceSchema::new(classification_attribute()),
            OpenLimits::default(),
        )
        .blocking_wait()
        .expect("Terrain fixture Workspace creates");
        Self {
            workspace,
            ticks,
            classifications,
            _temporary: temporary,
        }
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    pub fn snapshot(&self) -> Snapshot {
        self.workspace.head()
    }

    pub fn ticks(&self) -> &[[i64; 3]] {
        &self.ticks
    }

    pub fn classifications(&self) -> &[u8] {
        &self.classifications
    }

    pub fn point(&self, ordinal: u64) -> PointId {
        PointId::new(self.workspace.source(), ordinal)
    }

    pub fn select_ordinals(&self, snapshot: &Snapshot, ordinals: &[u64]) -> PointSet {
        snapshot
            .select_point_ids(
                ordinals.iter().copied().map(|ordinal| self.point(ordinal)),
                PointSetLimits::default(),
            )
            .blocking_wait()
            .expect("Terrain fixture Edit target materializes")
    }
}

pub fn classification_attribute() -> AttributeId {
    AttributeId::new(301).expect("fixture Attribute identity is nonzero")
}

pub fn identity_transform() -> PositionTransform {
    PositionTransform::new([0.0; 3], [1.0; 3]).expect("identity transform is valid")
}

pub fn operation(byte: u8) -> OperationId {
    OperationId::from_bytes([byte; 16]).expect("fixture Operation identity is nonzero")
}

pub fn committed(outcome: CommitOutcome) -> CommitReceipt {
    match outcome {
        CommitOutcome::Committed(receipt) => receipt,
        CommitOutcome::Rejected(reason) => panic!("fixture commit was rejected: {reason:?}"),
        CommitOutcome::Indeterminate(uncertainty) => {
            panic!("fixture commit was indeterminate: {uncertainty:?}")
        }
    }
}

pub fn derive_surface(snapshot: Snapshot, ground: u8) -> TerrainSurface {
    derive_with(
        snapshot,
        TerrainRecipe::new(ground),
        TerrainLimits::default(),
    )
    .expect("Terrain fixture derives")
}

pub fn derive_with(
    snapshot: Snapshot,
    recipe: TerrainRecipe,
    limits: TerrainLimits,
) -> Result<TerrainSurface, TerrainError> {
    point_terrain::derive(snapshot, recipe, limits).blocking_wait()
}

pub fn point_row_limits(max_output_points: u64, max_batch_points: u64) -> PointRowLimits {
    let defaults = PointRowLimits::default();
    PointRowLimits::new(
        defaults.candidate_limits(),
        defaults.source_read_budget(),
        defaults.max_overlay_segments(),
        defaults.max_overlay_bytes(),
        max_output_points,
        max_batch_points,
        max_batch_points.saturating_mul(ROW_PAYLOAD_BYTES),
        defaults.max_working_bytes(),
    )
}

pub fn terrain_limits_with_row_batch(max_batch_points: u64) -> TerrainLimits {
    let defaults = TerrainLimits::default();
    TerrainLimits::new(
        point_row_limits(defaults.max_input_points(), max_batch_points),
        defaults.max_input_points(),
        defaults.max_faces(),
        defaults.max_working_bytes(),
        defaults.max_surface_bytes(),
        defaults.max_work_units(),
    )
}

fn classification_columns(classifications: Vec<u8>, point_count: usize) -> AttributeColumns {
    let definition = AttributeDefinition::new(
        classification_attribute(),
        "classification",
        AttributeDataType::U8,
    )
    .expect("fixture classification definition is valid");
    let column = AttributeColumn::new(definition, AttributeValues::u8(classifications))
        .expect("fixture classification column is valid");
    AttributeColumns::new(vec![column], point_count)
        .expect("fixture classification column is row-aligned")
}

struct TemporaryFixture {
    directory: PathBuf,
}

impl TemporaryFixture {
    fn new(label: &str) -> Self {
        let sequence = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "punctra-point-terrain-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&directory).expect("create isolated Terrain fixture directory");
        Self { directory }
    }

    fn index_path(&self) -> PathBuf {
        self.directory.join("fixture.pidx")
    }

    fn workspace_path(&self) -> PathBuf {
        self.directory.join("fixture.pcw")
    }
}

impl Drop for TemporaryFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
