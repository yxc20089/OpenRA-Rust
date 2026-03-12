//! Terrain layer — cell grid, movement costs, and occupancy.
//!
//! Provides `CellLayer<T>` for 2D grid storage and `TerrainMap`
//! for querying movement costs and building/actor occupancy.

/// A 2D grid layer indexed by cell position (x, y).
///
/// Uses rectangular grid layout matching RA mod.
/// Index = y * width + x.
#[derive(Debug, Clone)]
pub struct CellLayer<T> {
    pub width: i32,
    pub height: i32,
    data: Vec<T>,
}

impl<T: Default + Clone> CellLayer<T> {
    pub fn new(width: i32, height: i32) -> Self {
        Self {
            width,
            height,
            data: vec![T::default(); (width * height) as usize],
        }
    }

    fn index(&self, x: i32, y: i32) -> usize {
        (y * self.width + x) as usize
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= 0 && x < self.width && y >= 0 && y < self.height
    }

    pub fn get(&self, x: i32, y: i32) -> &T {
        &self.data[self.index(x, y)]
    }

    pub fn set(&mut self, x: i32, y: i32, val: T) {
        let idx = self.index(x, y);
        self.data[idx] = val;
    }
}

/// Movement cost for a cell. 0 = impassable, 100 = normal, >100 = rough.
pub const COST_IMPASSABLE: i16 = i16::MAX;
pub const COST_NORMAL: i16 = 100;

/// The world's terrain and occupancy state.
#[derive(Debug, Clone)]
pub struct TerrainMap {
    pub width: i32,
    pub height: i32,
    /// Base terrain movement cost per cell.
    costs: CellLayer<i16>,
    /// Actor ID occupying each cell (0 = empty). For buildings with footprints,
    /// all footprint cells store the building's actor ID.
    occupancy: CellLayer<u32>,
}

impl TerrainMap {
    /// Create terrain map from map dimensions.
    /// All cells default to normal passable terrain.
    pub fn new(width: i32, height: i32) -> Self {
        let mut costs = CellLayer::new(width, height);
        for y in 0..height {
            for x in 0..width {
                costs.set(x, y, COST_NORMAL);
            }
        }
        Self {
            width,
            height,
            costs,
            occupancy: CellLayer::new(width, height),
        }
    }

    /// Check if a cell is within map bounds.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.costs.contains(x, y)
    }

    /// Get the base movement cost for a cell.
    pub fn cost(&self, x: i32, y: i32) -> i16 {
        if !self.contains(x, y) {
            return COST_IMPASSABLE;
        }
        *self.costs.get(x, y)
    }

    /// Check if a cell is passable (not impassable terrain AND not occupied).
    pub fn is_passable(&self, x: i32, y: i32) -> bool {
        self.contains(x, y)
            && *self.costs.get(x, y) != COST_IMPASSABLE
            && *self.occupancy.get(x, y) == 0
    }

    /// Check if a cell is passable terrain (ignoring occupancy).
    pub fn is_terrain_passable(&self, x: i32, y: i32) -> bool {
        self.contains(x, y) && *self.costs.get(x, y) != COST_IMPASSABLE
    }

    /// Get the actor ID occupying a cell (0 = empty).
    pub fn occupant(&self, x: i32, y: i32) -> u32 {
        if !self.contains(x, y) { return 0; }
        *self.occupancy.get(x, y)
    }

    /// Mark a cell as occupied by an actor.
    pub fn set_occupant(&mut self, x: i32, y: i32, actor_id: u32) {
        if self.contains(x, y) {
            self.occupancy.set(x, y, actor_id);
        }
    }

    /// Clear occupancy for a cell.
    pub fn clear_occupant(&mut self, x: i32, y: i32) {
        if self.contains(x, y) {
            self.occupancy.set(x, y, 0);
        }
    }

    /// Occupy a rectangular footprint for a building.
    pub fn occupy_footprint(&mut self, top_left_x: i32, top_left_y: i32, w: i32, h: i32, actor_id: u32) {
        for dy in 0..h {
            for dx in 0..w {
                self.set_occupant(top_left_x + dx, top_left_y + dy, actor_id);
            }
        }
    }

    /// Clear a rectangular footprint.
    pub fn clear_footprint(&mut self, top_left_x: i32, top_left_y: i32, w: i32, h: i32) {
        for dy in 0..h {
            for dx in 0..w {
                self.clear_occupant(top_left_x + dx, top_left_y + dy);
            }
        }
    }

    /// Set terrain cost for a cell.
    pub fn set_cost(&mut self, x: i32, y: i32, cost: i16) {
        if self.contains(x, y) {
            self.costs.set(x, y, cost);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_layer_basic() {
        let mut layer: CellLayer<i32> = CellLayer::new(10, 10);
        assert_eq!(*layer.get(5, 5), 0);
        layer.set(5, 5, 42);
        assert_eq!(*layer.get(5, 5), 42);
    }

    #[test]
    fn cell_layer_bounds() {
        let layer: CellLayer<i32> = CellLayer::new(10, 10);
        assert!(layer.contains(0, 0));
        assert!(layer.contains(9, 9));
        assert!(!layer.contains(-1, 0));
        assert!(!layer.contains(10, 0));
    }

    #[test]
    fn terrain_map_passability() {
        let mut map = TerrainMap::new(10, 10);
        assert!(map.is_passable(5, 5));

        map.set_cost(5, 5, COST_IMPASSABLE);
        assert!(!map.is_passable(5, 5));
    }

    #[test]
    fn terrain_map_occupancy() {
        let mut map = TerrainMap::new(10, 10);
        assert!(map.is_passable(5, 5));

        map.set_occupant(5, 5, 100);
        assert!(!map.is_passable(5, 5));
        assert!(map.is_terrain_passable(5, 5));
        assert_eq!(map.occupant(5, 5), 100);

        map.clear_occupant(5, 5);
        assert!(map.is_passable(5, 5));
    }

    #[test]
    fn footprint_occupy() {
        let mut map = TerrainMap::new(10, 10);
        map.occupy_footprint(3, 3, 2, 2, 50);
        assert_eq!(map.occupant(3, 3), 50);
        assert_eq!(map.occupant(4, 4), 50);
        assert_eq!(map.occupant(5, 5), 0);
    }
}
