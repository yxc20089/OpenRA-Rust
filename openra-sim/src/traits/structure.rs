//! Structure trait component — Phase-7 typed view onto static buildings
//! that may have an Armament (gun, tsla, pbox, ftur) and always block
//! pathfinder cells via their footprint.
//!
//! Unlike `Vehicle`/`Turret`, structures do not move and do not maintain
//! a chassis facing — they are placed at construction time and either
//! stand or die. This struct is a thin annotation that tells the world
//! tick (a) how big the footprint is, (b) whether the structure can
//! shoot, (c) what weapon/range to use, and (d) whether the kill counts
//! toward the `MustBeDestroyed` win condition.
//!
//! Determinism notes
//! -----------------
//! `Structure` carries no synced state of its own — the canonical hash
//! lives in `TraitState::Building { top_left }` (already in the actor's
//! trait list) and `TraitState::Health`. Adding `Structure` does not
//! perturb the sync hash; this is intentional, mirroring C# `Building`
//! whose `[VerifySync]` field is the top-left CPos only.

use crate::traits::armament::Armament;

/// Static defense / building component.
#[derive(Debug, Clone)]
pub struct Structure {
    /// Footprint width in cells (x extent).
    pub footprint_w: i32,
    /// Footprint height in cells (y extent).
    pub footprint_h: i32,
    /// Whether this structure provides a virtual `MustBeDestroyed`
    /// objective (fact, proc — but not powr / barr / tent).
    pub must_be_destroyed: bool,
    /// Whether the structure has an active armament. We don't yet
    /// distinguish AntiAir-only weapons from ground; AA-only stubs
    /// (sam, agun) have `armament = None` until we add aircraft.
    pub armament: Option<Armament>,
}

impl Structure {
    /// Build a `Structure` from rules-derived stats. `armament` may be
    /// `None` for cosmetic buildings (powr/barr) or AA-only defenses
    /// (sam/agun) that the engine does not yet resolve targets for.
    pub fn new(footprint_w: i32, footprint_h: i32) -> Self {
        Self {
            footprint_w: footprint_w.max(1),
            footprint_h: footprint_h.max(1),
            must_be_destroyed: false,
            armament: None,
        }
    }

    /// Builder: enable the `MustBeDestroyed` flag.
    pub fn with_must_be_destroyed(mut self, v: bool) -> Self {
        self.must_be_destroyed = v;
        self
    }

    /// Builder: attach an Armament so the world tick will fire it at
    /// in-range hostile actors. Pass `None` to leave the structure
    /// inert (cosmetic / AA-only).
    pub fn with_armament(mut self, armament: Option<Armament>) -> Self {
        self.armament = armament;
        self
    }

    /// True iff the structure currently has a usable armament (i.e.
    /// can fire). AA-only defenses return false until aircraft land.
    pub fn can_fire(&self) -> bool {
        self.armament.is_some()
    }

    /// Number of cells this structure occupies.
    pub fn footprint_cells(&self) -> i32 {
        self.footprint_w * self.footprint_h
    }
}

/// Lightweight classification of an actor type into "armed defense /
/// armed building / cosmetic / AA-only".
///
/// Phase 7 keeps this as a small lookup table rather than relying on
/// `weapons.first()` because (a) we want to distinguish AA-only
/// weapons (`AAGun`, `Nike`) from ground weapons until aircraft exist,
/// and (b) RA's `^Defense` actors carry primary weapons we want to
/// load (TurretGun, TeslaZap, M60mg).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefenseKind {
    /// Standard ground-shoot static turret (gun, pbox, hbox, ftur).
    GroundTurret,
    /// Tesla coil — handled like a ground turret but documented as
    /// having a 3-charge salvo we approximate as plain reload-gated
    /// shots in Phase 7.
    Tesla,
    /// AA-only: not yet wired to fire at anything (no aircraft yet).
    AntiAirOnly,
    /// Has a primary weapon trait but it is intentionally unmapped
    /// (e.g. SAM whose weapon name we don't include).
    InertWeapon,
}

/// Classify a building actor type into a defense kind, or `None` if
/// the building is purely cosmetic (powr, barr, fact, proc, ...).
///
/// The list mirrors the `^Defense` chain in `vendor/OpenRA/mods/ra/
/// rules/structures.yaml`.
pub fn classify_defense(actor_type: &str) -> Option<DefenseKind> {
    match actor_type {
        // Standard ground turrets.
        "gun" | "pbox" | "hbox" | "ftur" => Some(DefenseKind::GroundTurret),
        // Tesla coil — special charge cycle, approximated as a turret.
        "tsla" => Some(DefenseKind::Tesla),
        // AA-only — inert until aircraft exist.
        "agun" | "sam" => Some(DefenseKind::AntiAirOnly),
        // Gap generator — has no offensive weapon.
        "gap" => Some(DefenseKind::InertWeapon),
        _ => None,
    }
}

/// True if the actor is a structure that contributes to the kill-all
/// objective (`MustBeDestroyed`).
///
/// In RA the trait is set on `^Building` actors that should count for
/// the kill-all win condition. For the Phase 7 scope we treat any
/// building as contributing — the strategy scenarios' termination
/// criterion is "all enemy buildings + units destroyed", not the more
/// nuanced C# logic. This mirrors the existing `world::winners()`
/// behaviour which already counts all `ActorKind::Building` actors.
pub fn building_must_be_destroyed(_actor_type: &str) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::armament::Armament;
    use openra_data::rules::{WDist, WeaponStats};

    fn make_test_weapon() -> WeaponStats {
        WeaponStats {
            name: "TurretGun".into(),
            range: WDist::from_cells(6),
            reload_delay: 30,
            damage: 6000,
            ..Default::default()
        }
    }

    #[test]
    fn classify_known_defenses() {
        assert_eq!(classify_defense("gun"), Some(DefenseKind::GroundTurret));
        assert_eq!(classify_defense("pbox"), Some(DefenseKind::GroundTurret));
        assert_eq!(classify_defense("ftur"), Some(DefenseKind::GroundTurret));
        assert_eq!(classify_defense("tsla"), Some(DefenseKind::Tesla));
        assert_eq!(classify_defense("agun"), Some(DefenseKind::AntiAirOnly));
        assert_eq!(classify_defense("sam"), Some(DefenseKind::AntiAirOnly));
        assert_eq!(classify_defense("gap"), Some(DefenseKind::InertWeapon));
        assert_eq!(classify_defense("powr"), None);
        assert_eq!(classify_defense("fact"), None);
    }

    #[test]
    fn structure_can_fire_only_when_armament_set() {
        let s_inert = Structure::new(1, 1);
        assert!(!s_inert.can_fire());
        let s_armed = Structure::new(1, 1)
            .with_armament(Some(Armament::new(make_test_weapon())));
        assert!(s_armed.can_fire());
    }

    #[test]
    fn structure_footprint_cells_is_w_times_h() {
        assert_eq!(Structure::new(3, 2).footprint_cells(), 6);
        assert_eq!(Structure::new(1, 1).footprint_cells(), 1);
        // Lower bound clamps to 1×1.
        assert_eq!(Structure::new(0, 0).footprint_cells(), 1);
    }

    #[test]
    fn building_must_be_destroyed_default_true() {
        // Every building counts for the win condition in Phase 7.
        assert!(building_must_be_destroyed("fact"));
        assert!(building_must_be_destroyed("powr"));
        assert!(building_must_be_destroyed("gun"));
    }
}
