use render_protocol::{
    BatchKey, BatchVersion, PointBatch, PointId, ProtocolError, RenderPoint, ViewGenerationKey,
};

pub(crate) const TILE_COLUMNS: u32 = 16;
pub(crate) const TOTAL_BATCHES: u64 = 256;
pub(crate) const POINTS_PER_BATCH: u64 = 4_096;
pub(crate) const TOTAL_POINTS: u64 = TOTAL_BATCHES * POINTS_PER_BATCH;
pub(crate) const SCENE_RADIUS: f64 = 700.0;
pub(crate) const SCENE_TARGET: [f64; 3] = [6_378_137.125, 13_756_432.625, 120.0];

const POINTS_PER_AXIS: u32 = 64;
const TILE_SIZE_F32: f32 = 32.0;
const TILE_SIZE_F64: f64 = 32.0;
const HEIGHT_SCALE: f32 = 18.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TileCoordinate {
    column: u32,
    row: u32,
}

#[derive(Debug)]
pub(crate) struct SyntheticScene {
    view_generation: ViewGenerationKey,
    tile_order: Vec<TileCoordinate>,
    next_tile: usize,
}

impl SyntheticScene {
    pub(crate) fn new(view_generation: ViewGenerationKey) -> Self {
        let mut tile_order = all_tiles();
        tile_order.sort_by_key(|tile| tile_sort_key(*tile));
        Self {
            view_generation,
            tile_order,
            next_tile: 0,
        }
    }

    pub(crate) fn next_batch(&mut self) -> Result<Option<PointBatch>, ProtocolError> {
        let Some(tile) = self.tile_order.get(self.next_tile).copied() else {
            return Ok(None);
        };
        self.next_tile += 1;
        make_batch(self.view_generation, tile).map(Some)
    }

    pub(crate) fn loaded_batches(&self) -> u64 {
        u64::try_from(self.next_tile).expect("the tile count fits in u64")
    }

    pub(crate) fn highlight_ids(&self) -> Vec<PointId> {
        self.tile_order.iter().copied().map(highlight_id).collect()
    }
}

fn all_tiles() -> Vec<TileCoordinate> {
    let capacity = usize::try_from(TOTAL_BATCHES).expect("the tile count fits in usize");
    let mut tiles = Vec::with_capacity(capacity);
    for row in 0..TILE_COLUMNS {
        for column in 0..TILE_COLUMNS {
            tiles.push(TileCoordinate { column, row });
        }
    }
    tiles
}

fn tile_sort_key(tile: TileCoordinate) -> (i32, u32, u32) {
    let column = centered_index(tile.column);
    let row = centered_index(tile.row);
    (column * column + row * row, tile.row, tile.column)
}

fn make_batch(
    view_generation: ViewGenerationKey,
    tile: TileCoordinate,
) -> Result<PointBatch, ProtocolError> {
    let point_capacity =
        usize::try_from(POINTS_PER_BATCH).expect("the points-per-batch count fits in usize");
    let mut points = Vec::with_capacity(point_capacity);
    for row in 0..POINTS_PER_AXIS {
        for column in 0..POINTS_PER_AXIS {
            points.push(make_point(tile, column, row)?);
        }
    }

    PointBatch::new(
        view_generation,
        batch_key(tile),
        BatchVersion::new(1),
        tile_world_origin(tile),
        points,
    )
}

fn make_point(tile: TileCoordinate, column: u32, row: u32) -> Result<RenderPoint, ProtocolError> {
    let denominator =
        f32::from(u16::try_from(POINTS_PER_AXIS - 1).expect("the point-grid extent fits in u16"));
    let column_fraction =
        f32::from(u16::try_from(column).expect("the point column fits in u16")) / denominator;
    let row_fraction =
        f32::from(u16::try_from(row).expect("the point row fits in u16")) / denominator;
    let relative_x = (column_fraction - 0.5) * TILE_SIZE_F32;
    let relative_y = (row_fraction - 0.5) * TILE_SIZE_F32;
    let scene_x = centered_index_f32(tile.column) * TILE_SIZE_F32 / 2.0 + relative_x;
    let scene_y = centered_index_f32(tile.row) * TILE_SIZE_F32 / 2.0 + relative_y;
    let noise = point_noise(tile, column, row);
    let height = terrain_height(scene_x, scene_y, noise);

    RenderPoint::new(
        [relative_x, relative_y, height],
        terrain_color(height, noise),
        point_id(tile, column, row),
    )
}

fn terrain_height(x: f32, y: f32, noise: f32) -> f32 {
    let broad_hills = (x * 0.018).sin() * (y * 0.015).cos() * HEIGHT_SCALE;
    let fine_ridges = ((x + y) * 0.055).sin() * 4.5;
    let drainage = -9.0 * (-((y - x * 0.28) * 0.035).powi(2)).exp();
    broad_hills + fine_ridges + drainage + noise
}

fn terrain_color(height: f32, noise: f32) -> [u8; 4] {
    let variation = if noise.is_sign_positive() { 10 } else { 0 };
    if height < -10.0 {
        [34, 104, 151, 255]
    } else if height < 2.0 {
        [42, 128_u8.saturating_add(variation), 92, 255]
    } else if height < 13.0 {
        [111, 151_u8.saturating_add(variation), 77, 255]
    } else if height < 22.0 {
        [177, 145, 91_u8.saturating_add(variation), 255]
    } else {
        [215, 218, 210, 255]
    }
}

fn point_noise(tile: TileCoordinate, column: u32, row: u32) -> f32 {
    let mut hash = tile.column.wrapping_mul(0x9E37_79B9)
        ^ tile.row.wrapping_mul(0x85EB_CA6B)
        ^ column.wrapping_mul(0xC2B2_AE35)
        ^ row.wrapping_mul(0x27D4_EB2F);
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x7FEB_352D);
    hash ^= hash >> 15;
    let low_bits = u16::try_from(hash & u32::from(u16::MAX)).expect("the hash was masked to u16");
    let unit = f32::from(low_bits) / f32::from(u16::MAX);
    (unit - 0.5) * 0.7
}

fn tile_world_origin(tile: TileCoordinate) -> [f64; 3] {
    [
        SCENE_TARGET[0] + f64::from(centered_index(tile.column)) * TILE_SIZE_F64 / 2.0,
        SCENE_TARGET[1] + f64::from(centered_index(tile.row)) * TILE_SIZE_F64 / 2.0,
        SCENE_TARGET[2],
    ]
}

fn centered_index(index: u32) -> i32 {
    i32::try_from(index).expect("the tile index fits in i32") * 2
        - (i32::try_from(TILE_COLUMNS).expect("the tile extent fits in i32") - 1)
}

fn centered_index_f32(index: u32) -> f32 {
    let centered =
        i16::try_from(centered_index(index)).expect("the centered tile index fits in i16");
    f32::from(centered)
}

fn batch_key(tile: TileCoordinate) -> BatchKey {
    BatchKey::new(tile_linear_index(tile) + 1)
}

fn point_id(tile: TileCoordinate, column: u32, row: u32) -> PointId {
    let local_index = u64::from(row) * u64::from(POINTS_PER_AXIS) + u64::from(column);
    PointId::new(tile_linear_index(tile) * POINTS_PER_BATCH + local_index + 1)
}

fn highlight_id(tile: TileCoordinate) -> PointId {
    point_id(tile, POINTS_PER_AXIS / 2, POINTS_PER_AXIS / 2)
}

fn tile_linear_index(tile: TileCoordinate) -> u64 {
    u64::from(tile.row) * u64::from(TILE_COLUMNS) + u64::from(tile.column)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use render_protocol::ViewId;

    use super::*;

    fn view_generation() -> ViewGenerationKey {
        ViewGenerationKey::new(ViewId::new(1), 1)
    }

    #[test]
    fn streams_every_tile_once_from_the_center_out() {
        let mut scene = SyntheticScene::new(view_generation());
        let first = scene.next_batch().unwrap().unwrap();

        assert_eq!(first.point_count(), POINTS_PER_BATCH);
        assert!(first.world_origin()[0] > 6_000_000.0);
        assert!(first.world_origin()[1] > 10_000_000.0);
        assert_eq!(
            scene.tile_order.len(),
            usize::try_from(TOTAL_BATCHES).unwrap()
        );
        assert_eq!(scene.highlight_ids().len(), scene.tile_order.len());
    }

    #[test]
    fn generated_batches_are_deterministic() {
        let tile = TileCoordinate { column: 5, row: 9 };

        assert_eq!(
            make_batch(view_generation(), tile),
            make_batch(view_generation(), tile)
        );
    }

    #[test]
    fn highlight_identifiers_are_unique() {
        let scene = SyntheticScene::new(view_generation());
        let highlights = scene.highlight_ids();
        let unique = highlights.iter().copied().collect::<BTreeSet<_>>();

        assert_eq!(unique.len(), highlights.len());
    }
}
