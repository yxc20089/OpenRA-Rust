//! Vehicle trait component — locomotor classification + chassis state.
//!
//! Phase-6 typed component for tank/jeep/apc/harv-class actors. The
//! `Mobile` trait already carries chassis facing and interpolated
//! center position; `Vehicle` adds the bits the combat/turret code
//! needs to know about specifically:
//!
//! - `Locomotor`: which terrain-cost lookup to use. Passed through to
//!   the pathfinder. Phase 6 only ships a single `tracked`/`wheeled`/
//!   `foot` enum — actual per-cell cost differences are deferred (the
//!   pathfinder already takes a unified cost map).
//! - `has_turret`: shorthand for "this actor has a `Turret` component
//!   in its trait list", used by attack/auto-target code paths to
//!   know whether to attempt a turret aim before firing.
//!
//! Out of scope (deferred):
//! - Crush damage on ramming infantry (`Crushable` trait).
//! - Speed multipliers from upgrades / damage states.
//! - Husk / wreckage spawn on death (`SpawnActorOnDeath`).

use crate::math::WAngle;

/// Locomotor variant — names mirror the C# `Mobile.Locomotor` strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Locomotor {
    /// Foot infantry. Slowest, can cross some narrow obstacles.
    Foot,
    /// Wheeled vehicles (jeep, harv). Faster on roads, slower in rough.
    Wheeled,
    /// Tracked vehicles (1tnk, 2tnk, 3tnk, apc, mnly). Medium across all
    /// passable terrain.
    Tracked,
    /// Heavy-tracked (4tnk only in this codebase). Slow, stronger crush.
    HeavyTracked,
    /// Naval — not used by strategy scenarios but kept for completeness.
    Naval,
    /// Aircraft — flies over everything.
    Aircraft,
}

impl Locomotor {
    /// Map a YAML `Mobile.Locomotor` string into the typed enum.
    /// Unknown strings fall back to `Tracked` (the most common
    /// vehicle locomotor) and emit a warning via `eprintln!`.
    pub fn from_yaml(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "foot" | "infantry" => Locomotor::Foot,
            "wheeled" => Locomotor::Wheeled,
            "tracked" => Locomotor::Tracked,
            "heavytracked" => Locomotor::HeavyTracked,
            "naval" | "lcraft" => Locomotor::Naval,
            "fly" | "aircraft" | "helicopter" => Locomotor::Aircraft,
            other => {
                eprintln!(
                    "openra-sim: unknown locomotor '{other}', defaulting to Tracked"
                );
                Locomotor::Tracked
            }
        }
    }

    /// True if this locomotor is ground-based (tracked / wheeled / foot).
    pub fn is_ground(self) -> bool {
        matches!(
            self,
            Locomotor::Foot | Locomotor::Wheeled | Locomotor::Tracked | Locomotor::HeavyTracked
        )
    }
}

/// Vehicle component carried alongside `Mobile` for vehicular actors.
///
/// The chassis facing already lives on `Mobile` (and is synced via
/// `TraitState::Mobile`). This struct only adds non-synced runtime
/// metadata — locomotor type and the "has a turret" flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vehicle {
    pub locomotor: Locomotor,
    pub has_turret: bool,
    /// Initial chassis facing applied at spawn. Used by the env loader
    /// when spawning a unit; once spawned this is informational only.
    pub initial_facing: WAngle,
}

impl Vehicle {
    pub fn new(locomotor: Locomotor, has_turret: bool, initial_facing: WAngle) -> Self {
        Vehicle { locomotor, has_turret, initial_facing }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_locomotors() {
        assert_eq!(Locomotor::from_yaml("foot"), Locomotor::Foot);
        assert_eq!(Locomotor::from_yaml("Foot"), Locomotor::Foot);
        assert_eq!(Locomotor::from_yaml("wheeled"), Locomotor::Wheeled);
        assert_eq!(Locomotor::from_yaml("tracked"), Locomotor::Tracked);
        assert_eq!(Locomotor::from_yaml("heavytracked"), Locomotor::HeavyTracked);
        assert_eq!(Locomotor::from_yaml("naval"), Locomotor::Naval);
    }

    #[test]
    fn unknown_locomotor_falls_back_to_tracked() {
        assert_eq!(Locomotor::from_yaml("rocket-skates"), Locomotor::Tracked);
    }

    #[test]
    fn is_ground_classification() {
        assert!(Locomotor::Tracked.is_ground());
        assert!(Locomotor::Foot.is_ground());
        assert!(!Locomotor::Aircraft.is_ground());
        assert!(!Locomotor::Naval.is_ground());
    }
}
