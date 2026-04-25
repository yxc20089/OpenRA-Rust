//! Turret trait component — independent turret facing on top of a chassis.
//!
//! Phase-6 typed component mirroring C# `Turreted` (
//! `vendor/OpenRA/OpenRA.Mods.Common/Traits/Turreted.cs`).
//!
//! Each tank-style vehicle (and several static defenses) has a chassis
//! facing (the `Mobile.facing`) plus a turret that rotates independently
//! to track its target. The turret's local facing is stored relative to
//! the chassis. World-space facing is `chassis_facing + local_facing`.
//!
//! The facing is **not** synced — C# `Turreted.cs` only `[VerifySync]`'s
//! `QuantizedFacings`. Animation-only state. We keep it off the
//! `TraitState` enum to avoid bloating the sync hash.
//!
//! Out of scope (deferred):
//! - Pitch / Roll axes (we only carry yaw).
//! - Quantized facings (used only for sprite rendering).
//! - `RealignDelay` re-centering when not aiming. Phase 7+.
//! - Multi-turret actors (4tnk hull-MG style). Phase 8+.

use crate::math::WAngle;

/// One turret mounted on an actor. Carries an independent facing
/// in world-space (we collapse "local + body" to a single absolute
/// yaw — for ground-only units with no pitch/roll this is exact).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Turret {
    /// World-space yaw of the turret in `WAngle` units (0..1024).
    pub facing: WAngle,
    /// Turn rate in `WAngle` units per tick. Sourced from
    /// `Turreted.TurnSpeed` in the YAML rules. Default `28` matches
    /// the 1tnk reference value.
    pub turn_speed: i32,
    /// Tolerance for "facing target" — once the desired and current
    /// facings are within this many units the turret is considered
    /// locked on. C# uses an exact `==` comparison on `LocalOrientation.Yaw`
    /// vs `desired`. Setting this to `0` reproduces that behaviour;
    /// we keep a small non-zero default to soak rounding error in
    /// the integer chase loop. `attack_turret_tolerance_default()` in
    /// the world config can override it.
    pub aim_tolerance: i32,
}

impl Turret {
    /// New turret pointing at `initial` with the given turn speed.
    /// `aim_tolerance` defaults to `0` (exact match) which mirrors C#.
    pub fn new(initial: WAngle, turn_speed: i32) -> Self {
        Turret {
            facing: initial,
            turn_speed: turn_speed.max(0),
            aim_tolerance: 0,
        }
    }

    /// Construct a turret with a custom aim tolerance (used by the
    /// activity to avoid rounding-related deadlocks).
    pub fn with_tolerance(initial: WAngle, turn_speed: i32, aim_tolerance: i32) -> Self {
        Turret {
            facing: initial,
            turn_speed: turn_speed.max(0),
            aim_tolerance: aim_tolerance.max(0),
        }
    }

    /// Step the turret one tick toward `desired`. Mirrors
    /// `Util.TickFacing` semantics: take the shorter rotation
    /// direction, advance by `turn_speed` (snap to target if within
    /// `turn_speed`).
    ///
    /// Returns `true` if the turret has arrived at `desired` within
    /// `aim_tolerance`.
    pub fn tick(&mut self, desired: WAngle) -> bool {
        let current = self.facing.angle;
        let target = desired.angle;

        // Equal: nothing to do.
        if angle_delta(current, target).abs() <= self.aim_tolerance {
            self.facing = WAngle::new(target);
            return true;
        }

        // Snap if within turn_speed of target.
        let step = self.turn_speed;
        let left_turn = ((current - target).rem_euclid(1024)).abs();
        let right_turn = ((target - current).rem_euclid(1024)).abs();
        let close_enough_left = left_turn <= step;
        let close_enough_right = right_turn <= step;

        if close_enough_left || close_enough_right {
            self.facing = WAngle::new(target);
            return true;
        }

        // Take the shorter direction.
        let new_angle = if right_turn < left_turn {
            current + step
        } else {
            current - step
        };
        self.facing = WAngle::new(new_angle);
        false
    }

    /// True iff `self.facing` is within `aim_tolerance` of `desired`.
    pub fn has_achieved(&self, desired: WAngle) -> bool {
        angle_delta(self.facing.angle, desired.angle).abs() <= self.aim_tolerance
    }
}

/// Compute the smallest signed delta `to - from` in `[-512, 512]` so
/// the test `delta.abs() <= tol` is well-defined even across the
/// wrap-around at 0/1023.
fn angle_delta(from: i32, to: i32) -> i32 {
    let mut d = (to - from).rem_euclid(1024);
    if d > 512 {
        d -= 1024;
    }
    d
}

/// Compute the world-space yaw a turret should adopt to point from
/// `from` to `to`. Both are world positions; only horizontal components
/// matter (we ignore z). Returns `None` if the points coincide.
///
/// OpenRA's `WVec.Yaw` returns `WAngle.ArcTan(-y, x)` (note the
/// negation of y because screen-y is south-positive). We reproduce
/// the same convention so a target south of the attacker yields
/// a downward facing.
pub fn yaw_between(from: crate::math::WPos, to: crate::math::WPos) -> Option<WAngle> {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    if dx == 0 && dy == 0 {
        return None;
    }
    Some(arc_tan(-dy as i64, dx as i64))
}

/// Integer ArcTan returning a `WAngle` (0..1024). Mirrors
/// `WAngle.ArcTan(y, x)` semantics: 0 = east, 256 = north, 512 = west,
/// 768 = south. We use `f64::atan2` because the C# implementation also
/// uses a floating-point approximation internally for non-cardinal
/// vectors and rounds to the nearest WAngle unit.
///
/// NOTE: the C# `WAngle.ArcTan` is integer-only (uses a polynomial
/// approximation seeded from a lookup). For determinism across all
/// platforms we'd ideally port the same fixed-point routine, but for
/// Phase 6 — used only for visual turret aim, not for damage / sync —
/// the `f64` rounding suffices and remains deterministic on x86_64
/// and aarch64 (IEEE-754).
fn arc_tan(y: i64, x: i64) -> WAngle {
    let radians = (y as f64).atan2(x as f64);
    let units = (radians * 1024.0 / (2.0 * std::f64::consts::PI)).round() as i32;
    WAngle::new(units)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::WPos;

    #[test]
    fn fresh_turret_at_target() {
        let mut t = Turret::new(WAngle::new(0), 28);
        assert!(t.tick(WAngle::new(0)));
        assert_eq!(t.facing.angle, 0);
    }

    #[test]
    fn turret_takes_shorter_direction_clockwise() {
        // 0 -> 100 should rotate clockwise (positive direction)
        let mut t = Turret::new(WAngle::new(0), 10);
        let arrived = t.tick(WAngle::new(100));
        assert!(!arrived);
        assert_eq!(t.facing.angle, 10);
    }

    #[test]
    fn turret_takes_shorter_direction_ccw() {
        // 0 -> 900 should rotate counter-clockwise (124 vs 900)
        let mut t = Turret::new(WAngle::new(0), 10);
        t.tick(WAngle::new(900));
        // Going CCW: 0 - 10 = -10 → 1014
        assert_eq!(t.facing.angle, 1014);
    }

    #[test]
    fn turret_snaps_when_within_step() {
        let mut t = Turret::new(WAngle::new(95), 10);
        let arrived = t.tick(WAngle::new(100));
        assert!(arrived);
        assert_eq!(t.facing.angle, 100);
    }

    #[test]
    fn turret_handles_wraparound() {
        let mut t = Turret::new(WAngle::new(1020), 10);
        t.tick(WAngle::new(5));
        // Wraps clockwise: 1020 + 10 = 1030 → 6 (closer to target than going CCW)
        // Should be 5 (snapped) since 1020 -> 5 is only 9 units.
        assert_eq!(t.facing.angle, 5);
    }

    #[test]
    fn yaw_east_is_zero() {
        // Target one cell east → yaw = 0 (WAngle "east")
        let from = WPos::new(0, 0, 0);
        let to = WPos::new(1024, 0, 0);
        assert_eq!(yaw_between(from, to).unwrap().angle, 0);
    }

    #[test]
    fn yaw_north_is_quarter() {
        // Target one cell north (-y in world coords) → yaw = 256
        let from = WPos::new(0, 0, 0);
        let to = WPos::new(0, -1024, 0);
        assert_eq!(yaw_between(from, to).unwrap().angle, 256);
    }

    #[test]
    fn yaw_south_is_three_quarter() {
        let from = WPos::new(0, 0, 0);
        let to = WPos::new(0, 1024, 0);
        assert_eq!(yaw_between(from, to).unwrap().angle, 768);
    }

    #[test]
    fn turret_arrives_with_tolerance() {
        let mut t = Turret::with_tolerance(WAngle::new(100), 10, 4);
        // Already within tolerance.
        assert!(t.tick(WAngle::new(102)));
        assert_eq!(t.facing.angle, 102);
    }
}
