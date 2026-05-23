//! Resource (ore) layer — a thin facade over `TerrainMap`'s
//! per-cell `ResourceCell` storage, focused on the scenario-author
//! workflow of "place an ore patch centered at (x,y) holding N units".
//!
//! Historical context: the ore layer has long existed on
//! `TerrainMap` (`set_resource` / `harvest_resource` /
//! `find_nearest_resource`) and the harvester `Activity::Harvest` FSM
//! already drives a full harvest→deposit cycle in `world.rs`. What
//! was missing for the bench's economy idiom was (a) a first-class
//! scenario-YAML way to declare ore patches (`ore_patches:`) — without
//! abusing the `mine` map prop — and (b) a stable, testable API for
//! seeding patches into the terrain. This module provides both.
//!
//! Ore patches mineral out at one density unit per harvest tick
//! (`TerrainMap::harvest_resource`), and `resource_value(ResourceType)`
//! controls the per-bale cash conversion (currently 25 per ore unit,
//! 50 per gem) — those constants live in `world.rs` to keep economy
//! tuning in one place.

use crate::terrain::{ResourceType, TerrainMap};

/// A scenario-declared ore patch: a roughly disk-shaped region of
/// harvestable ore centered at `(x, y)` containing roughly `amount`
/// density units total. `radius` controls how spread out the patch
/// is (default 3, ≈ 28 cells); `density` is the per-cell density
/// (1..=12, default = clamp(amount / cells, 1, 12)).
///
/// The seeder fills cells in a disk around the center, skipping the
/// center cell only if it is impassable (we WANT the center cell
/// harvestable for a tight patch). Cells with existing terrain
/// impassability (water, cliffs) are skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrePatch {
    pub x: i32,
    pub y: i32,
    /// Total density units to spread across the patch (roughly).
    /// Each unit = one harvested bale = ~25 cash on deposit.
    pub amount: i32,
    /// Patch radius in cells. Default = 3.
    pub radius: i32,
}

impl OrePatch {
    pub fn new(x: i32, y: i32, amount: i32) -> Self {
        Self { x, y, amount, radius: 3 }
    }

    /// Number of map cells the patch will attempt to fill (disk of
    /// radius `r`). Independent of how many are actually passable.
    pub fn cell_capacity(&self) -> i32 {
        let r = self.radius.max(0);
        let mut n = 0;
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy <= r * r {
                    n += 1;
                }
            }
        }
        n.max(1)
    }
}

/// Seed one ore patch into the given terrain. Returns the number of
/// cells that ACTUALLY received ore (skipped cells: out-of-bounds or
/// impassable). Idempotent in the sense that re-seeding adds density
/// on top of any pre-existing ore at the same cell, capped at the
/// per-cell max (12).
///
/// The seed pattern is a filled disk of radius `patch.radius`; per-cell
/// density is `clamp(amount / passable_cells, 1, 12)`. This keeps the
/// total density roughly equal to `amount` while never exceeding the
/// RA per-cell cap.
pub fn seed_ore_patch(terrain: &mut TerrainMap, patch: OrePatch) -> i32 {
    let r = patch.radius.max(0);
    // First pass: count passable target cells so we can pick a per-cell
    // density that approximates `amount` total without exceeding the
    // RA per-cell cap of 12.
    let mut passable_cells = 0i32;
    for dy in -r..=r {
        for dx in -r..=r {
            if dx * dx + dy * dy > r * r {
                continue;
            }
            let (x, y) = (patch.x + dx, patch.y + dy);
            if terrain.contains(x, y) && terrain.is_terrain_passable(x, y) {
                passable_cells += 1;
            }
        }
    }
    if passable_cells == 0 {
        return 0;
    }
    let per_cell = ((patch.amount.max(1) + passable_cells - 1) / passable_cells)
        .clamp(1, 12) as u8;

    let mut placed = 0i32;
    for dy in -r..=r {
        for dx in -r..=r {
            if dx * dx + dy * dy > r * r {
                continue;
            }
            let (x, y) = (patch.x + dx, patch.y + dy);
            if !terrain.contains(x, y) || !terrain.is_terrain_passable(x, y) {
                continue;
            }
            // Add to any existing density (capped). Cells previously
            // empty become Ore at `per_cell`.
            let existing = terrain.resource(x, y);
            let (new_type, new_density) = match existing.resource_type {
                ResourceType::None => (ResourceType::Ore, per_cell),
                t => (t, (existing.density.saturating_add(per_cell)).min(12)),
            };
            terrain.set_resource(x, y, new_type, new_density);
            placed += 1;
        }
    }
    placed
}

/// Total ore density currently present on the map (sum across all
/// cells). Useful for snapshot reporting and tests that verify ore
/// depletion over time.
pub fn total_ore_density(terrain: &TerrainMap) -> i32 {
    terrain.total_resources()
}

/// Enumerate every cell that currently holds ore or gems, returning
/// `(x, y, density)` triples. Used by the env layer to surface a
/// compact ore-cells list in the agent observation — the agent's
/// "where is the ore?" perception channel.
pub fn enumerate_resource_cells(terrain: &TerrainMap) -> Vec<(i32, i32, i32)> {
    let mut out = Vec::new();
    for y in 0..terrain.height {
        for x in 0..terrain.width {
            let c = terrain.resource(x, y);
            if c.resource_type != ResourceType::None && c.density > 0 {
                out.push((x, y, c.density as i32));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::TerrainMap;

    #[test]
    fn seed_patch_fills_disk() {
        let mut t = TerrainMap::new(40, 40);
        let placed = seed_ore_patch(&mut t, OrePatch::new(20, 20, 500));
        assert!(placed > 20, "expected disk to fill, got {}", placed);
        assert!(t.has_resource(20, 20));
        let total = total_ore_density(&t);
        // Total density should be in the same order of magnitude as `amount`
        // (per-cell density is clamped at 12; if amount / cells > 12 we
        // saturate, so total can be < amount but never far above).
        assert!(total > 0);
    }

    #[test]
    fn seed_patch_skips_impassable() {
        let mut t = TerrainMap::new(20, 20);
        // Make a swath impassable
        for x in 5..=10 {
            t.set_cost(x, 10, crate::terrain::COST_IMPASSABLE);
        }
        let placed = seed_ore_patch(&mut t, OrePatch { x: 7, y: 10, amount: 200, radius: 1 });
        // r=1 disk of 5 cells centered (7,10): (7,10),(6,10),(8,10),(7,9),(7,11);
        // 3 of those are impassable (x∈5..10 at y=10).
        assert!(placed <= 5);
        // The impassable cells must not have received ore.
        assert!(!t.has_resource(7, 10));
        assert!(!t.has_resource(6, 10));
        assert!(!t.has_resource(8, 10));
        // The cells above/below (passable) did.
        assert!(t.has_resource(7, 9));
        assert!(t.has_resource(7, 11));
    }

    #[test]
    fn enumerate_returns_seeded_cells() {
        let mut t = TerrainMap::new(20, 20);
        seed_ore_patch(&mut t, OrePatch { x: 10, y: 10, amount: 100, radius: 1 });
        let cells = enumerate_resource_cells(&t);
        assert_eq!(cells.len(), 5); // r=1 disk = 5 cells
        for (_, _, d) in &cells {
            assert!(*d > 0);
        }
    }

    #[test]
    fn depletion_drains_total() {
        let mut t = TerrainMap::new(20, 20);
        seed_ore_patch(&mut t, OrePatch { x: 10, y: 10, amount: 60, radius: 1 });
        let before = total_ore_density(&t);
        // Harvest until the center cell is depleted
        while t.has_resource(10, 10) {
            t.harvest_resource(10, 10);
        }
        let after = total_ore_density(&t);
        assert!(after < before, "expected total to drop after harvest");
    }
}
