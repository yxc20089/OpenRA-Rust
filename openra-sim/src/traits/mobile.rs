//! Mobile trait component — facing, occupying cell(s), interpolated position.
//!
//! Phase-1 typed view onto `TraitState::Mobile`. This module owns the
//! fixed-point interpolation helpers and conversion to/from the synced
//! `TraitState` representation. All math goes through `WPos`, `CPos`, and
//! `WAngle` (no f32/f64 in game state).

use super::TraitState;
use crate::math::{CPos, WAngle, WPos};

/// Movement-related actor component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mobile {
    /// Current facing in OpenRA "WAngle" units (0..1024).
    pub facing: WAngle,
    /// Cell the actor logically occupies (origin of the current segment).
    pub from_cell: CPos,
    /// Cell the actor is moving toward (equal to `from_cell` when stationary).
    pub to_cell: CPos,
    /// Sub-cell world-position interpolated between `from_cell` and `to_cell`.
    pub center_position: WPos,
    /// World units per game-tick along the current segment.
    pub speed: i32,
}

impl Mobile {
    /// Stationary actor centred on `cell`.
    pub fn at(cell: CPos, facing: WAngle, speed: i32) -> Self {
        let center = center_of_cell(cell.x(), cell.y());
        Mobile {
            facing,
            from_cell: cell,
            to_cell: cell,
            center_position: center,
            speed,
        }
    }

    /// True when `from_cell == to_cell` and `center_position` matches.
    pub fn is_stationary(&self) -> bool {
        self.from_cell == self.to_cell
            && self.center_position == center_of_cell(self.from_cell.x(), self.from_cell.y())
    }

    /// Convert into the synced `TraitState::Mobile` representation
    /// (drops `speed`, which is not synced).
    pub fn to_state(self) -> TraitState {
        TraitState::Mobile {
            facing: self.facing.angle,
            from_cell: self.from_cell,
            to_cell: self.to_cell,
            center_position: self.center_position,
        }
    }
}

/// Convert a cell position to its sub-cell-centred world position.
/// Mirrors `world::center_of_cell`; duplicated here so trait modules are
/// self-contained.
pub fn center_of_cell(x: i32, y: i32) -> WPos {
    WPos::new(1024 * x + 512, 1024 * y + 512, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_is_stationary() {
        let m = Mobile::at(CPos::new(3, 4), WAngle::new(0), 56);
        assert!(m.is_stationary());
        assert_eq!(m.center_position, WPos::new(3 * 1024 + 512, 4 * 1024 + 512, 0));
    }

    #[test]
    fn to_state_preserves_fields() {
        let m = Mobile::at(CPos::new(1, 2), WAngle::new(256), 80);
        match m.to_state() {
            TraitState::Mobile { facing, from_cell, to_cell, center_position } => {
                assert_eq!(facing, 256);
                assert_eq!(from_cell, CPos::new(1, 2));
                assert_eq!(to_cell, CPos::new(1, 2));
                assert_eq!(center_position, WPos::new(1024 + 512, 2 * 1024 + 512, 0));
            }
            _ => panic!("expected Mobile variant"),
        }
    }

    #[test]
    fn moving_between_cells_is_not_stationary() {
        let mut m = Mobile::at(CPos::new(0, 0), WAngle::new(0), 56);
        m.to_cell = CPos::new(1, 0);
        m.center_position = WPos::new(800, 512, 0); // partway between cells
        assert!(!m.is_stationary());
    }
}
