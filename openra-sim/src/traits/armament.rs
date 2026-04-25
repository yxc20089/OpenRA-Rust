//! Armament trait component — single-weapon firing state.
//!
//! Phase-3 typed component carrying a weapon's stats (range, damage,
//! reload delay) and the actor's per-tick firing cooldown. The weapon
//! definition is `openra_data::rules::WeaponStats`, the typed view
//! Phase 4 surfaces over the parsed `weapons.yaml` ruleset; the
//! `Armament` instance owns a clone so it can be carried around the
//! actor's component graph without lifetime grief.
//!
//! Single-weapon, no-Versus, no-burst, no-splash. Multi-weapon
//! armaments (e.g. tank turret + hull MG), per-armor-class damage
//! multipliers, burst patterns and splash radii are deferred to the
//! v2 combat pass — see `STATUS_PHASE_3.md`.

use openra_data::rules::WeaponStats;

/// One weapon mounted on an actor. Tracks the cooldown remaining
/// before the next shot.
#[derive(Debug, Clone)]
pub struct Armament {
    /// Weapon definition (range / damage / reload). Cloned from the
    /// ruleset so the component is `'static`.
    pub weapon: WeaponStats,
    /// Ticks remaining until the next shot. `0` means ready to fire.
    pub current_cooldown_ticks: u32,
}

impl Armament {
    /// Build a fresh armament with the cooldown at `0`.
    pub fn new(weapon: WeaponStats) -> Self {
        Armament { weapon, current_cooldown_ticks: 0 }
    }

    /// Decrement the cooldown by one tick (saturating at zero).
    /// Should be called once per game tick by the world loop.
    pub fn tick(&mut self) {
        if self.current_cooldown_ticks > 0 {
            self.current_cooldown_ticks -= 1;
        }
    }

    /// True when the armament is ready to fire this tick.
    pub fn is_ready(&self) -> bool {
        self.current_cooldown_ticks == 0
    }

    /// Mark the armament as having fired — resets the cooldown to
    /// `weapon.reload_delay` ticks. Negative reload delays clamp to
    /// zero (defensive against malformed YAML).
    pub fn mark_fired(&mut self) {
        self.current_cooldown_ticks = self.weapon.reload_delay.max(0) as u32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openra_data::rules::WDist;

    fn m1carbine() -> WeaponStats {
        WeaponStats {
            name: "M1Carbine".into(),
            range: WDist::from_cells(5),
            reload_delay: 20,
            damage: 1000,
        }
    }

    #[test]
    fn fresh_armament_is_ready() {
        let arm = Armament::new(m1carbine());
        assert!(arm.is_ready());
        assert_eq!(arm.current_cooldown_ticks, 0);
    }

    #[test]
    fn mark_fired_resets_cooldown_to_reload_delay() {
        let mut arm = Armament::new(m1carbine());
        arm.mark_fired();
        assert_eq!(arm.current_cooldown_ticks, 20);
        assert!(!arm.is_ready());
    }

    #[test]
    fn tick_decrements_cooldown() {
        let mut arm = Armament::new(m1carbine());
        arm.mark_fired();
        for _ in 0..20 {
            arm.tick();
        }
        assert!(arm.is_ready());
    }

    #[test]
    fn tick_saturates_at_zero() {
        let mut arm = Armament::new(m1carbine());
        // Tick from idle: must not underflow.
        for _ in 0..5 {
            arm.tick();
        }
        assert!(arm.is_ready());
    }

    #[test]
    fn negative_reload_delay_clamps_to_zero() {
        let mut bad = m1carbine();
        bad.reload_delay = -5;
        let mut arm = Armament::new(bad);
        arm.mark_fired();
        assert_eq!(arm.current_cooldown_ticks, 0);
    }
}
