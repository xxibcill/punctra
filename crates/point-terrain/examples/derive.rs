//! Runs the complete public in-memory Source-to-LandXML terrain composition.

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use point_contracts::{
    AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
    AttributeValues, CoordinateReference, PositionTransform,
};
use point_index::{PrepareLimits, prepare};
use point_terrain::{
    CheckPoint, CheckPointId, CheckPointLimits, LandXmlLimits, LandXmlOptions, TerrainLimits,
    TerrainRecipe,
};
use point_workspace::{OpenLimits, WorkspaceSchema, create};
use source_memory::MemorySource;

const CLASSIFICATION_ATTRIBUTE_ID: u32 = 301;
const GROUND_CLASSIFICATION: u8 = 2;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = ExampleDirectory::new()?;
    let source = source_memory::open(memory_fixture()?).blocking_wait()?;
    let index = prepare(
        source,
        directory.path().join("example.pidx"),
        PrepareLimits::default(),
    )
    .blocking_wait()?;
    let workspace = create(
        directory.path().join("example.pcw"),
        index,
        WorkspaceSchema::new(classification_attribute()?),
        OpenLimits::default(),
    )
    .blocking_wait()?;
    let surface = point_terrain::derive(
        workspace.head(),
        TerrainRecipe::new(GROUND_CLASSIFICATION),
        TerrainLimits::default(),
    )
    .blocking_wait()?;

    let transform = surface.descriptor().position_transform();
    let first = surface
        .vertices()
        .first()
        .ok_or_else(|| io::Error::other("example Terrain contains no vertices"))?;
    let bounds = surface.descriptor().bounds();
    let check_points = [
        CheckPoint::new(CheckPointId::new(1)?, transform.world_f64(first.ticks()))?,
        CheckPoint::new(
            CheckPointId::new(2)?,
            [
                bounds.max()[0] + 1.0,
                bounds.max()[1] + 1.0,
                bounds.max()[2],
            ],
        )?,
    ];
    let qa = surface
        .check_points(check_points, CheckPointLimits::default())
        .blocking_wait()?;
    assert_eq!(qa.statistics().covered_count(), 1);
    assert_eq!(qa.statistics().gap_count(), 1);

    let target = directory.path().join("terrain.xml");
    let options =
        LandXmlOptions::metric_metres("Punctra Terrain Example", "2026-08-10", "12:34:56Z")?
            .allow_unknown_coordinate_reference_as_metric_metres();
    let receipt = surface
        .export_landxml(&target, options, LandXmlLimits::default())
        .blocking_wait()?;
    assert_eq!(fs::metadata(&target)?.len(), receipt.byte_length());

    let descriptor = surface.descriptor();
    println!(
        "Derived {} Ground Points into {} vertices and {} faces",
        descriptor.input_point_count(),
        descriptor.vertex_count(),
        descriptor.face_count()
    );
    println!(
        "geometry={} topology={} artifact={}",
        descriptor.geometry_hash(),
        descriptor.topology_hash(),
        descriptor.artifact_hash()
    );
    println!(
        "descriptor_accounted_peak_working_bytes={} descriptor_retained_surface_bytes={} descriptor_topology_steps={}",
        descriptor.accounted_peak_working_bytes(),
        descriptor.retained_surface_bytes(),
        descriptor.topology_steps()
    );
    println!(
        "QA covered={} gaps={} face_tests={} accounted_peak_working_bytes={}",
        qa.statistics().covered_count(),
        qa.statistics().gap_count(),
        qa.face_tests(),
        qa.accounted_peak_working_bytes()
    );
    println!(
        "LandXML bytes={} content_hash={} (temporary state is removed on exit)",
        receipt.byte_length(),
        receipt.content_hash()
    );
    println!("Descriptor accounting is not an observed worker-heap measurement.");
    Ok(())
}

fn memory_fixture() -> Result<MemorySource, Box<dyn std::error::Error>> {
    let side = 5_i64;
    let ticks = (0..side)
        .flat_map(|y| (0..side).map(move |x| [x, y, x * x + 3 * y * y + x * y]))
        .collect::<Vec<_>>();
    let point_count = ticks.len();
    let definition = AttributeDefinition::new(
        classification_attribute()?,
        "classification",
        AttributeDataType::U8,
    )?;
    let column = AttributeColumn::new(
        definition,
        AttributeValues::u8(vec![GROUND_CLASSIFICATION; point_count]),
    )?;
    let attributes = AttributeColumns::new(vec![column], point_count)?;
    Ok(MemorySource::from_columns(
        PositionTransform::new([0.0; 3], [1.0, 1.0, 0.01])?,
        CoordinateReference::Unknown,
        ticks,
        attributes,
    )?)
}

fn classification_attribute() -> Result<AttributeId, point_contracts::ContractError> {
    AttributeId::new(CLASSIFICATION_ATTRIBUTE_ID)
}

struct ExampleDirectory {
    path: PathBuf,
}

impl ExampleDirectory {
    fn new() -> io::Result<Self> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for attempt in 0..100_u32 {
            let path = std::env::temp_dir().join(format!(
                "punctra-point-terrain-example-{}-{timestamp}-{attempt}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not create isolated point-terrain example directory",
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ExampleDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
