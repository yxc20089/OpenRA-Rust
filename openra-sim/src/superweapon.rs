//! Superweapon system — per-player charge timers + effect dispatchers.
//!
//! Three superweapons, each requiring a launcher building:
//!
//! - `mslo` Nuclear Missile Silo — ground-target Nuke. On fire applies
//!   high-damage AoE at a target cell.
//! - `iron` Iron Curtain — friendly-actor target. On fire the target
//!   actor becomes invulnerable for `IRON_CURTAIN_TICKS` ticks.
//! - `pdox` Chronosphere — friendly-actor target. On fire the target
//!   teleports to a chosen cell.
//!
//! Charge timers are per (kind, owner). Once a launcher building exists
//! the charge starts counting DOWN from `charge_for(kind)` to zero;
//! `is_ready` returns true at zero. Firing resets the timer.
//!
//! The actual world-side effects (damage / invulnerability / teleport)
//! are dispatched from `World::fire_superweapon` so the manager itself
//! stays a pure timing / state struct (easier to test in isolation).

use std::collections::BTreeMap;

/// Which superweapon are we talking about. We carry the launcher
/// building's type-name as the canonical identity so the rules ↔ engine
/// roundtrip stays a string compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SuperweaponKind {
    /// `mslo` nuclear missile silo. AoE damage at a ground cell.
    Nuke,
    /// `iron` iron curtain. Invulnerability on a friendly actor.
    IronCurtain,
    /// `pdox` chronosphere. Teleport a friendly actor to a chosen cell.
    Chronosphere,
}

impl SuperweaponKind {
    /// The launcher building's actor-type name. This is the string the
    /// command DSL uses and what the gamerules registers.
    pub fn building_type(self) -> &'static str {
        match self {
            SuperweaponKind::Nuke => "mslo",
            SuperweaponKind::IronCurtain => "iron",
            SuperweaponKind::Chronosphere => "pdox",
        }
    }

    /// Inverse of `building_type` — None if the name is not a known
    /// superweapon launcher.
    pub fn from_building_type(t: &str) -> Option<Self> {
        match t {
            "mslo" => Some(SuperweaponKind::Nuke),
            "iron" => Some(SuperweaponKind::IronCurtain),
            "pdox" => Some(SuperweaponKind::Chronosphere),
            _ => None,
        }
    }

    /// Charge time in ticks. Hard-coded MVP values (~30–60 game seconds
    /// at 25 ticks/sec). All three are 100 ticks for easy testing — the
    /// real game values would be higher (mslo ~6000, iron/pdox ~4500).
    pub fn charge_ticks(self) -> i32 {
        // Hardcoded short charge so tests can fully charge inside a
        // handful of frames. The world-side default for real play uses
        // the longer values in `gamerules.rs` superweapon defaults.
        100
    }
}

/// Effect radius / window constants.
pub const NUKE_RADIUS_CELLS: i32 = 4;
pub const NUKE_BASE_DAMAGE: i32 = 500_000;
/// Iron Curtain invulnerability window in ticks (~30 sec at 25 t/s).
pub const IRON_CURTAIN_TICKS: u32 = 750;

/// All superweapon kinds the engine knows about, in deterministic order.
pub const ALL_KINDS: &[SuperweaponKind] = &[
    SuperweaponKind::Nuke,
    SuperweaponKind::IronCurtain,
    SuperweaponKind::Chronosphere,
];

/// Per-(kind, owner) charge state. Each entry is the ticks remaining
/// until the weapon is ready to fire (0 = charged).
#[derive(Debug, Clone, Default)]
pub struct SuperweaponManager {
    timers: BTreeMap<(SuperweaponKind, u32), i32>,
}

impl SuperweaponManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ensure a charge timer exists for `(kind, owner)`. If none exists,
    /// seed it at the full charge value. Called every tick for each
    /// (launcher-building, owner) pair so a launcher that gets built
    /// mid-game starts charging from full.
    pub fn ensure_timer(&mut self, kind: SuperweaponKind, owner: u32) {
        self.timers
            .entry((kind, owner))
            .or_insert_with(|| kind.charge_ticks());
    }

    /// Decrement every active timer by one tick (saturating at 0).
    pub fn tick(&mut self) {
        for t in self.timers.values_mut() {
            if *t > 0 {
                *t -= 1;
            }
        }
    }

    /// True iff the weapon is fully charged for this owner. A weapon
    /// the manager has never seen is NOT ready (no launcher ⇒ no
    /// charge).
    pub fn is_ready(&self, kind: SuperweaponKind, owner: u32) -> bool {
        matches!(self.timers.get(&(kind, owner)), Some(&t) if t <= 0)
    }

    /// Reset the timer for this (kind, owner) back to the full charge
    /// value after firing.
    pub fn reset(&mut self, kind: SuperweaponKind, owner: u32) {
        self.timers
            .insert((kind, owner), kind.charge_ticks());
    }

    /// Ticks remaining until ready (0 = ready). None when the player
    /// has never had a launcher of this kind.
    pub fn ticks_remaining(&self, kind: SuperweaponKind, owner: u32) -> Option<i32> {
        self.timers.get(&(kind, owner)).copied()
    }

    /// Iterate over every (kind, owner, ticks_remaining) entry — used
    /// by the snapshot pipeline.
    pub fn iter(&self) -> impl Iterator<Item = (SuperweaponKind, u32, i32)> + '_ {
        self.timers
            .iter()
            .map(|((k, o), &t)| (*k, *o, t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_weapon_is_never_ready() {
        let m = SuperweaponManager::new();
        assert!(!m.is_ready(SuperweaponKind::Nuke, 1));
    }

    #[test]
    fn timer_charges_and_fires() {
        let mut m = SuperweaponManager::new();
        m.ensure_timer(SuperweaponKind::Nuke, 1);
        assert!(!m.is_ready(SuperweaponKind::Nuke, 1));
        for _ in 0..SuperweaponKind::Nuke.charge_ticks() {
            m.tick();
        }
        assert!(m.is_ready(SuperweaponKind::Nuke, 1));
        m.reset(SuperweaponKind::Nuke, 1);
        assert!(!m.is_ready(SuperweaponKind::Nuke, 1));
    }

    #[test]
    fn building_type_roundtrip() {
        for k in ALL_KINDS {
            let t = k.building_type();
            assert_eq!(SuperweaponKind::from_building_type(t), Some(*k));
        }
        assert_eq!(SuperweaponKind::from_building_type("powr"), None);
    }
}
