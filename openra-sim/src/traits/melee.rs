//! Melee attack — Phase-8 typed component for the dog (`DogJaw`).
//!
//! C# `OpenRA.Mods.Cnc/Traits/Attacks/AttackLeap.cs` does a leap-and-strike
//! at exactly 1c0 cells, dealing instant damage to a single infantry
//! target. We treat melee as a special-case `Armament` whose:
//!
//! - `range = 1` cell
//! - `projectile_speed = 0` (instant — no projectile spawn)
//! - `splash_radius = 0` (single-target)
//! - per-warhead `ValidTargets = Infantry` constraint applied at
//!   target-pick time (the dog won't aim at a tank, even if it could
//!   reach 1 cell away)
//!
//! Phase 8 wires this through the existing `Activity::Attack` data
//! path: when `weapon_range == 1` and the attacker is `dog`, the world
//! tick uses the standard instant-damage code-path with no projectile
//! spawn. We expose a `MeleeAttack` thin wrapper around an `Armament`
//! so future callers (a wolf, a lion, an enraged tesla trooper…) can
//! reuse the structure.

use crate::traits::armament::Armament;
use openra_data::rules::{WDist, WeaponStats};

/// One melee armament. The contained `Armament` carries the weapon
/// definition (e.g. `DogJaw`) and the per-actor cooldown — same as
/// any ranged armament.
#[derive(Debug, Clone)]
pub struct MeleeAttack {
    pub armament: Armament,
    /// Whether the melee attack only hits infantry (always true for
    /// `dog`, kept as a flag in case future units need it).
    pub infantry_only: bool,
}

impl MeleeAttack {
    /// Build a fresh melee attack from a weapon. We force the range
    /// down to `1c0` regardless of what the YAML claims (the C#
    /// `AttackLeap` ignores `Range` and uses a hard-coded 1-cell leap
    /// distance). The DogJaw YAML actually sets `Range: 3c0` (the
    /// detection radius) but the leap itself is 1 cell.
    pub fn new(weapon: WeaponStats) -> Self {
        let mut clamped = weapon;
        clamped.range = WDist::from_cells(1);
        // Defensive: a melee weapon should never spawn a projectile.
        clamped.projectile_speed = WDist::ZERO;
        // No splash for melee.
        clamped.splash_radius = WDist::ZERO;
        MeleeAttack {
            armament: Armament::new(clamped),
            infantry_only: true,
        }
    }

    /// True iff the attack is currently ready to fire.
    pub fn is_ready(&self) -> bool {
        self.armament.is_ready()
    }

    /// Mark the attack as having connected — resets the cooldown.
    pub fn mark_fired(&mut self) {
        self.armament.mark_fired();
    }

    /// Decrement the cooldown by one tick.
    pub fn tick(&mut self) {
        self.armament.tick();
    }

    /// True iff the target at `chebyshev_cells` is in melee range
    /// (always exactly 1 cell for now).
    pub fn in_range(&self, chebyshev_cells: i32) -> bool {
        // Allow 0 cells too (target on the same cell — possible mid
        // path-tick when a unit walks onto a melee attacker's tile).
        chebyshev_cells <= 1
    }
}

/// Heuristic: does this actor type fight in melee?
///
/// Returns the weapon name to look up in the ruleset. Phase 8 only
/// wires `dog → DogJaw`; future melee units should be added here.
pub fn melee_weapon_for(actor_type: &str) -> Option<&'static str> {
    match actor_type {
        "dog" => Some("DogJaw"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dogjaw() -> WeaponStats {
        WeaponStats {
            name: "DogJaw".into(),
            // YAML has Range: 3c0 but melee enforces 1c0 internally.
            range: WDist::from_cells(3),
            reload_delay: 10,
            damage: 100_000,
            ..Default::default()
        }
    }

    #[test]
    fn melee_clamps_range_to_one_cell() {
        let m = MeleeAttack::new(dogjaw());
        assert_eq!(m.armament.weapon.range.length, 1024);
    }

    #[test]
    fn melee_in_range_is_one_cell_max() {
        let m = MeleeAttack::new(dogjaw());
        assert!(m.in_range(0));
        assert!(m.in_range(1));
        assert!(!m.in_range(2));
    }

    #[test]
    fn dog_is_known_melee_unit() {
        assert_eq!(melee_weapon_for("dog"), Some("DogJaw"));
        assert_eq!(melee_weapon_for("e1"), None);
    }
}
