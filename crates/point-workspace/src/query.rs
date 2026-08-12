use point_contracts::{PointBatch, WorldBounds};
use point_source::SourceSpan;

use crate::{PointQuery, WorkspaceError};

const SPAN_HASH_DOMAIN: &[u8] = b"punctra-selection-spans-v1";

pub(crate) fn matches_query(
    query: PointQuery,
    batch: &PointBatch,
    effective_classifications: &[u8],
    row: usize,
) -> Result<bool, WorkspaceError> {
    let matches_bounds = match query.bounds() {
        None => true,
        Some(bounds) => {
            let world = batch.positions().world_f64(row).ok_or_else(|| {
                WorkspaceError::incompatible(
                    "Source batch position row disappeared during exact Query evaluation",
                )
            })?;
            contains(bounds, world)
        }
    };
    if !matches_bounds {
        return Ok(false);
    }

    let matches_classification = match query.classification_eq() {
        None => true,
        Some(expected) => {
            let actual = effective_classifications.get(row).ok_or_else(|| {
                WorkspaceError::incompatible(
                    "effective classification row disappeared during exact Query evaluation",
                )
            })?;
            *actual == expected
        }
    };
    Ok(matches_classification)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SpanFacts {
    span_count: usize,
    point_count: u64,
    hash: [u8; 32],
}

impl SpanFacts {
    pub(crate) fn new(spans: &[SourceSpan]) -> Result<Self, WorkspaceError> {
        let mut point_count = 0_u64;
        let mut hasher = blake3::Hasher::new();
        hasher.update(SPAN_HASH_DOMAIN);
        for span in spans {
            point_count = point_count.checked_add(span.point_count()).ok_or(
                WorkspaceError::ResourceLimit {
                    limit: "candidate Points",
                    required: u64::MAX,
                    allowed: u64::MAX - 1,
                },
            )?;
            hasher.update(&span.first_ordinal().to_le_bytes());
            hasher.update(&span.point_count().to_le_bytes());
        }
        Ok(Self {
            span_count: spans.len(),
            point_count,
            hash: *hasher.finalize().as_bytes(),
        })
    }

    pub(crate) const fn span_count(self) -> usize {
        self.span_count
    }

    pub(crate) const fn point_count(self) -> u64 {
        self.point_count
    }
}

fn contains(bounds: WorldBounds, point: [f64; 3]) -> bool {
    let min = bounds.min();
    let max = bounds.max();
    (0..3).all(|axis| point[axis] >= min[axis] && point[axis] <= max[axis])
}

#[cfg(test)]
mod tests {
    use point_contracts::WorldBounds;

    use super::contains;

    #[test]
    fn inclusive_bounds_keep_faces_edges_and_corners() {
        let bounds = WorldBounds::new([-1.0, 2.0, 3.0], [4.0, 5.0, 6.0]).unwrap();

        assert!(contains(bounds, [-1.0, 5.0, 6.0]));
        assert!(contains(bounds, [4.0, 2.0, 3.0]));
        assert!(!contains(bounds, [-1.000_001, 5.0, 6.0]));
        assert!(!contains(bounds, [4.0, 5.000_001, 6.0]));
    }
}
