//! Opens and reads a directly constructed in-memory point Source.

use std::error::Error;

use point_contracts::{
    AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
    AttributeValues, CoordinateReference, PositionTransform,
};
use source_memory::MemorySource;

fn main() -> Result<(), Box<dyn Error>> {
    let ticks = vec![[0, 0, 0], [4, -2, 1], [9, 3, 2]];
    let intensity_id = AttributeId::new(1)?;
    let intensity = AttributeColumn::new(
        AttributeDefinition::new(intensity_id, "intensity", AttributeDataType::U16)?,
        AttributeValues::u16(vec![120, 340, 510]),
    )?;
    let attributes = AttributeColumns::new(vec![intensity], ticks.len())?;
    let transform = PositionTransform::new([500_000.0, 2_000_000.0, 10.0], [0.01; 3])?;
    let input =
        MemorySource::from_columns(transform, CoordinateReference::Unknown, ticks, attributes)?;

    let source = source_memory::open(input).blocking_wait()?;
    println!(
        "opened {} with {} Points",
        source.identity(),
        source.metadata().point_count()
    );

    let mut batches = source.points()?;
    while let Some(batch) = batches.next()? {
        let intensities = batch
            .attributes()
            .get(intensity_id)
            .expect("requested Attribute is present")
            .values()
            .as_u16()
            .expect("verified Attribute has its declared type");
        for (row, point_id) in batch.point_ids().enumerate() {
            println!(
                "{point_id:?}: ticks={:?}, intensity={}",
                batch.positions().ticks()[row],
                intensities[row]
            );
        }
    }

    let summary = batches
        .summary()
        .expect("successful exhaustion publishes a summary");
    println!("read {} Points", summary.exact_count());
    Ok(())
}
