//! Embedded vendor RA YAML — the single source of truth for unit/weapon
//! /building stats.
//!
//! Historically these YAML files were checked out into
//! `OpenRA-Rust/vendor/OpenRA/mods/ra/` from upstream OpenRA at SHA
//! `0938a27` (bleed) and parsed at runtime. That meant CI had to clone the
//! 139 MB upstream repo, and a developer running tests outside the source
//! tree would crash with "vendor RA YAML not found".
//!
//! The vendor YAML is now BAKED INTO THE BINARY via `include_str!` (the
//! exact same byte content as the original `0938a27` snapshot). The
//! filesystem-based loaders (`load_ruleset`, `try_from_vendor`) still
//! exist as overrides for power users who set `OPENRA_VENDOR_DIR`, but
//! by default the engine reads from the in-process strings here. The
//! existing parser (`load_ruleset_from_strings`) is run once per process
//! over the embedded text — this guarantees byte-identical runtime
//! behaviour to the pre-removal filesystem path (same parser, same input
//! bytes).
//!
//! To bump the snapshot, replace the .yaml files in
//! `openra-data/src/embedded/{rules,weapons}/` (this directory is the
//! tracked sibling next to `lib.rs`). Do NOT manually edit the YAML
//! after copy-in: keep each file byte-identical to upstream so the
//! provenance stays clear.

use crate::rules::{load_ruleset_from_strings, Ruleset};

// ---------------------------------------------------------------------------
// Rule files — order matches `load_ruleset()` in `rules.rs` so the merge
// produces an identical resolved ruleset.
// ---------------------------------------------------------------------------

const RULE_DEFAULTS: &str = include_str!("embedded/rules/defaults.yaml");
const RULE_PLAYER: &str = include_str!("embedded/rules/player.yaml");
const RULE_WORLD: &str = include_str!("embedded/rules/world.yaml");
const RULE_INFANTRY: &str = include_str!("embedded/rules/infantry.yaml");
const RULE_VEHICLES: &str = include_str!("embedded/rules/vehicles.yaml");
const RULE_AIRCRAFT: &str = include_str!("embedded/rules/aircraft.yaml");
const RULE_SHIPS: &str = include_str!("embedded/rules/ships.yaml");
const RULE_STRUCTURES: &str = include_str!("embedded/rules/structures.yaml");
const RULE_DECORATION: &str = include_str!("embedded/rules/decoration.yaml");
const RULE_MISC: &str = include_str!("embedded/rules/misc.yaml");
const RULE_CIVILIAN: &str = include_str!("embedded/rules/civilian.yaml");
const RULE_FAKES: &str = include_str!("embedded/rules/fakes.yaml");
const RULE_HUSKS: &str = include_str!("embedded/rules/husks.yaml");

pub const EMBEDDED_RULE_FILES: &[&str] = &[
    RULE_DEFAULTS,
    RULE_PLAYER,
    RULE_WORLD,
    RULE_INFANTRY,
    RULE_VEHICLES,
    RULE_AIRCRAFT,
    RULE_SHIPS,
    RULE_STRUCTURES,
    RULE_DECORATION,
    RULE_MISC,
    RULE_CIVILIAN,
    RULE_FAKES,
    RULE_HUSKS,
];

// ---------------------------------------------------------------------------
// Weapon files — `load_ruleset()` sorts the on-disk weapons dir by file
// name before reading. Listed below in sorted-by-filename order so the
// embedded sequence matches the filesystem behaviour exactly.
// ---------------------------------------------------------------------------

const WEAPON_BALLISTICS: &str = include_str!("embedded/weapons/ballistics.yaml");
const WEAPON_EXPLOSIONS: &str = include_str!("embedded/weapons/explosions.yaml");
const WEAPON_MISSILES: &str = include_str!("embedded/weapons/missiles.yaml");
const WEAPON_OTHER: &str = include_str!("embedded/weapons/other.yaml");
const WEAPON_SMALLCALIBER: &str = include_str!("embedded/weapons/smallcaliber.yaml");
const WEAPON_SUPERWEAPONS: &str = include_str!("embedded/weapons/superweapons.yaml");

pub const EMBEDDED_WEAPON_FILES: &[&str] = &[
    WEAPON_BALLISTICS,
    WEAPON_EXPLOSIONS,
    WEAPON_MISSILES,
    WEAPON_OTHER,
    WEAPON_SMALLCALIBER,
    WEAPON_SUPERWEAPONS,
];

/// Build a `Ruleset` from the embedded YAML strings. Equivalent to
/// `load_ruleset(<vendor RA mod dir>)` but reads from the in-binary
/// strings — no filesystem access, can't fail.
///
/// The first call from any thread is the slow one (parses ~330 KB of
/// YAML and resolves inheritance). Cache the result in your subsystem
/// (`GameRules::vendor_cached`, etc.) if you need it more than once.
pub fn load_ruleset_embedded() -> Ruleset {
    load_ruleset_from_strings(EMBEDDED_RULE_FILES, EMBEDDED_WEAPON_FILES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_ruleset_parses_and_contains_core_actors() {
        let rs = load_ruleset_embedded();
        // Spot-check a representative slice — same pins as the
        // filesystem-loader test in `rules.rs::load_ra_ruleset`.
        assert!(rs.actors.len() > 50, "actors: {}", rs.actors.len());
        let mcv = rs.actor("MCV").expect("MCV missing");
        assert!(mcv.has_trait("Mobile"));
        assert_eq!(mcv.trait_info("Health").unwrap().get_i32("HP"), Some(60000));
        let fact = rs.actor("FACT").expect("FACT missing");
        assert!(fact.has_trait("Building"));
        assert_eq!(fact.trait_info("Health").unwrap().get_i32("HP"), Some(150000));
        assert_eq!(
            rs.actor("POWR").unwrap().trait_info("Valued").unwrap().get_i32("Cost"),
            Some(300),
        );
        assert!(rs.weapons.len() > 10, "weapons: {}", rs.weapons.len());
        assert!(rs.weapon("M1Carbine").is_some());
    }
}
