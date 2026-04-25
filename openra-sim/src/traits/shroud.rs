//! Per-player visibility (shroud / fog of war) — Phase-3 typed view.
//!
//! Each `Shroud` instance is a 2D bool grid (`map.width × map.height`)
//! tracking which cells the owning player can currently see this tick.
//! The data-driven world already maintains a richer per-cell `u8`
//! shroud (0=unexplored, 1=fogged, 2=visible) — see
//! `world::World::update_shroud`. The new typed view sits alongside
//! and is what the LLM observation builder + Phase-5 PyO3 layer will
//! consume.
//!
//! Semantics — matches OpenRA's `Shroud` C# trait:
//!
//! * `is_visible(pos)`: true iff at least one own actor is currently
//!   in sight of `pos` at the end of this tick. Mirrors `Shroud.IsVisible`
//!   (which is "actively visible", excluding "explored-but-fogged").
//! * `is_explored(pos)`: true iff any own actor has *ever* seen `pos`.
//!   Once revealed, terrain stays explored — this is the gray
//!   "fogged" appearance in the C# UI. Enemy actors become invisible
//!   again the moment no own unit can see them, but the underlying
//!   terrain remains drawn (verified against
//!   `vendor/OpenRA/OpenRA.Mods.Common/Traits/World/Shroud.cs`,
//!   methods `IsVisible` vs `IsExplored`).
//!
//! Range conversion: OpenRA's `RevealsShroud.Range` is a `WDist`
//! (1024 units = 1 cell). We convert to a cell radius via integer
//! division and treat any cell within that radius as fully visible
//! ("inclusive rounding" — partial-cell overhang is rounded up
//! during conversion via `(dist + 511) / 1024`). The visibility
//! shape itself is a Chebyshev disc (square) to match the existing
//! `world::update_shroud` behaviour, which the PyO3 observation
//! layer already conforms to.

use openra_data::rules::WDist;
use std::collections::BTreeMap;

use crate::actor::ActorKind;
use crate::math::CPos;

/// Per-player shroud: which cells the player can see *right now*
/// and which they have *ever* seen.
#[derive(Debug, Clone)]
pub struct Shroud {
    width: i32,
    height: i32,
    /// `visible[y * width + x]` — actively visible this tick.
    visible: Vec<bool>,
    /// `explored[y * width + x]` — ever revealed (sticky).
    explored: Vec<bool>,
}

impl Shroud {
    /// Build a fresh shroud sized `width × height`. Everything
    /// starts unrevealed.
    pub fn new(width: i32, height: i32) -> Self {
        let n = (width.max(0) * height.max(0)) as usize;
        Shroud {
            width,
            height,
            visible: vec![false; n],
            explored: vec![false; n],
        }
    }

    pub fn width(&self) -> i32 { self.width }
    pub fn height(&self) -> i32 { self.height }

    fn idx(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            None
        } else {
            Some((y * self.width + x) as usize)
        }
    }

    /// True iff `(x, y)` is actively visible to the owning player.
    pub fn is_visible(&self, x: i32, y: i32) -> bool {
        match self.idx(x, y) {
            Some(i) => self.visible[i],
            None => false,
        }
    }

    /// True iff `(x, y)` has ever been seen by the owning player.
    pub fn is_explored(&self, x: i32, y: i32) -> bool {
        match self.idx(x, y) {
            Some(i) => self.explored[i],
            None => false,
        }
    }

    /// Convenience: `is_visible` at a `CPos`.
    pub fn is_visible_at(&self, cell: CPos) -> bool {
        self.is_visible(cell.x(), cell.y())
    }

    /// Reset the active-visibility layer (called at the start of a
    /// recompute pass). The `explored` layer is *not* cleared — it
    /// is sticky across ticks.
    pub fn clear_visible(&mut self) {
        for v in self.visible.iter_mut() {
            *v = false;
        }
    }

    /// Reveal the disc of cells centred on `(cx, cy)` with radius
    /// `cell_radius`. Marks both `visible` and `explored`.
    pub fn reveal(&mut self, cx: i32, cy: i32, cell_radius: i32) {
        if cell_radius < 0 {
            return;
        }
        let r2 = cell_radius * cell_radius;
        for dy in -cell_radius..=cell_radius {
            for dx in -cell_radius..=cell_radius {
                if dx * dx + dy * dy > r2 {
                    continue;
                }
                let x = cx + dx;
                let y = cy + dy;
                if let Some(i) = self.idx(x, y) {
                    self.visible[i] = true;
                    self.explored[i] = true;
                }
            }
        }
    }
}

/// Convert an OpenRA `WDist` into a cell radius. Partial cells are
/// rounded inclusively (so a 4c0 range = 4 cells, 4c512 = 5 cells).
pub fn wdist_to_cell_radius(d: WDist) -> i32 {
    if d.length <= 0 {
        return 0;
    }
    (d.length + 1023) / 1024
}

/// Per-player shroud table, keyed by player index in
/// `world.player_ids()`.
pub type ShroudTable = BTreeMap<u32, Shroud>;

/// Recompute the shroud for `player_idx` from every own actor's
/// `RevealsShroud.Range`. Falls back to a kind-based default sight
/// (matching `world::update_shroud`) when a unit has no rules entry.
///
/// `actors` — iterator of `(owner_player_id, kind, cell, optional reveal_range_wdist)`.
/// `player_idx` — the player whose shroud is being recomputed.
pub fn update_from_actors<I>(
    shroud: &mut Shroud,
    actors: I,
    player_idx: u32,
) where
    I: IntoIterator<Item = (u32, ActorKind, CPos, Option<WDist>)>,
{
    shroud.clear_visible();
    for (owner, kind, cell, reveal) in actors {
        if owner != player_idx {
            continue;
        }
        let radius = match reveal {
            Some(d) => wdist_to_cell_radius(d),
            None => kind_default_sight(kind),
        };
        if radius <= 0 {
            continue;
        }
        shroud.reveal(cell.x(), cell.y(), radius);
    }
}

/// Default sight radius (cells) by actor kind, matching the
/// hard-coded fallback in `world::update_shroud`.
pub fn kind_default_sight(kind: ActorKind) -> i32 {
    match kind {
        ActorKind::Building => 5,
        ActorKind::Infantry => 4,
        ActorKind::Vehicle => 6,
        ActorKind::Mcv => 5,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_shroud_has_nothing_visible_or_explored() {
        let s = Shroud::new(20, 20);
        for y in 0..20 {
            for x in 0..20 {
                assert!(!s.is_visible(x, y));
                assert!(!s.is_explored(x, y));
            }
        }
    }

    #[test]
    fn reveal_marks_visible_and_explored_within_radius() {
        let mut s = Shroud::new(20, 20);
        s.reveal(10, 10, 4);
        assert!(s.is_visible(10, 10));
        assert!(s.is_visible(14, 10));
        assert!(s.is_visible(10, 14));
        assert!(s.is_explored(14, 10));
        // Just outside the disc (Euclidean): (15, 15) is at d² = 50 > 16
        assert!(!s.is_visible(15, 15));
        assert!(!s.is_explored(15, 15));
    }

    #[test]
    fn out_of_bounds_lookup_returns_false() {
        let s = Shroud::new(5, 5);
        assert!(!s.is_visible(-1, 0));
        assert!(!s.is_visible(0, -1));
        assert!(!s.is_visible(5, 0));
        assert!(!s.is_visible(0, 5));
    }

    #[test]
    fn clear_visible_keeps_explored() {
        let mut s = Shroud::new(10, 10);
        s.reveal(5, 5, 2);
        assert!(s.is_visible(5, 5));
        s.clear_visible();
        assert!(!s.is_visible(5, 5));
        assert!(s.is_explored(5, 5));
    }

    #[test]
    fn wdist_to_cell_radius_inclusive_rounding() {
        // 4c0 → 4 cells exact
        assert_eq!(wdist_to_cell_radius(WDist::from_cells(4)), 4);
        // 4c512 → rounds up to 5
        assert_eq!(wdist_to_cell_radius(WDist::new(4 * 1024 + 512)), 5);
        // 4c1   → still rounds up
        assert_eq!(wdist_to_cell_radius(WDist::new(4 * 1024 + 1)), 5);
        // 0     → 0
        assert_eq!(wdist_to_cell_radius(WDist::ZERO), 0);
        // negative → 0
        assert_eq!(wdist_to_cell_radius(WDist::new(-100)), 0);
    }

    #[test]
    fn update_from_actors_filters_by_player() {
        let mut s = Shroud::new(20, 20);
        let actors = vec![
            // own infantry at (5,5) with reveal range 3 cells
            (1, ActorKind::Infantry, CPos::new(5, 5), Some(WDist::from_cells(3))),
            // enemy infantry at (15, 15) — should not reveal own shroud
            (2, ActorKind::Infantry, CPos::new(15, 15), Some(WDist::from_cells(3))),
        ];
        update_from_actors(&mut s, actors, 1);
        assert!(s.is_visible(5, 5));
        assert!(!s.is_visible(15, 15));
    }

    #[test]
    fn update_from_actors_falls_back_to_kind_default() {
        let mut s = Shroud::new(20, 20);
        let actors = vec![
            (1, ActorKind::Infantry, CPos::new(5, 5), None),
        ];
        update_from_actors(&mut s, actors, 1);
        // Default infantry sight is 4
        assert!(s.is_visible(5 + 4, 5));
        assert!(!s.is_visible(5 + 5, 5));
    }
}
