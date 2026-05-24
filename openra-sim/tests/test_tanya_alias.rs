//! Tanya alias regression: scenarios that use `type: tanya` must
//! resolve to the same combat stats as the C# RA `E7` actor when the
//! ruleset path is used (i.e. in production / vendored-YAML runs, NOT
//! just the `defaults()` fallback).
//!
//! Root cause this pins:
//!   * The C# Red Alert YAML registers Tanya as `E7` (HP 10000, weapon
//!     Colt45 damage 10000 / reload 7 / range 7c0). The bench scenario
//!     YAML uses the canonical name `tanya`, which existed in
//!     `GameRules::defaults()` but NOT in `GameRules::from_ruleset` —
//!     so production lookups (`world.rules.actor("tanya")`) returned
//!     `None`, the scenario actor was spawned with the fallback
//!     (max_hp=50000, weapons=[]), and Tanya fired the 100-dps
//!     "default" weapon. Cumulative effect: `combat-tanya-vs-rush`
//!     was unsolvable by any policy (the doctrine LOST instead of
//!     winning).
//!
//! Fix: `from_ruleset` now inserts a `tanya` alias cloning the `e7`
//! ActorStats so a scenario `type: tanya` actor gets the real
//! Allied-commando stats from the vendored YAML.

use openra_data::rules::load_ruleset;
use openra_sim::gamerules::GameRules;
use std::path::PathBuf;

fn vendor_dir() -> Option<PathBuf> {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let p = crate_dir.join("../vendor/OpenRA/mods/ra");
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

#[test]
fn from_ruleset_registers_tanya_alias() {
    let Some(p) = vendor_dir() else {
        eprintln!("vendor dir absent — skipping (defaults() path is tested elsewhere)");
        return;
    };
    let rs = load_ruleset(&p).expect("ruleset load");
    let rules = GameRules::from_ruleset(&rs);

    let tanya = rules
        .actor("tanya")
        .expect("tanya must be registered as an alias of e7 in from_ruleset");
    let e7 = rules.actor("e7").expect("e7 in ruleset");

    // Alias must be a faithful clone of E7's combat-relevant stats so
    // a scenario `type: tanya` actor gets the real Allied-commando
    // numbers (HP, speed, sight, weapon binding) instead of the
    // fallback (max_hp=50000, weapons=[], default weapon at 100 dps).
    assert_eq!(tanya.hp, e7.hp, "tanya HP must match e7");
    assert_eq!(tanya.speed, e7.speed, "tanya speed must match e7");
    assert_eq!(tanya.sight_range, e7.sight_range, "tanya sight must match e7");
    assert_eq!(
        tanya.weapons, e7.weapons,
        "tanya weapons must match e7 (Colt45)"
    );
    assert!(
        !tanya.weapons.is_empty(),
        "tanya must have ≥1 weapon (Colt45); empty ⇒ falls back to 'default' 100-dps weapon"
    );
}

#[test]
fn tanya_alias_has_colt45_combat_stats() {
    let Some(p) = vendor_dir() else {
        eprintln!("vendor dir absent — skipping");
        return;
    };
    let rs = load_ruleset(&p).expect("ruleset load");
    let rules = GameRules::from_ruleset(&rs);

    let tanya = rules.actor("tanya").expect("tanya alias");
    // Validate the actual ratings match C# RA E7 (HP 10000) — pin so
    // a future YAML drift breaks loud.
    assert_eq!(tanya.hp, 10000, "E7 HP from C# YAML");

    // Tanya must wield Colt45 as her PRIMARY armament (the actor stats
    // weapons list is built from Armament traits; Armament@PRIMARY +
    // Armament@GARRISONED both fire Colt45).
    let wname = tanya.weapons.first().expect("tanya has ≥1 weapon");
    assert_eq!(wname, "Colt45", "tanya primary weapon must be Colt45");

    let w = rules.weapon("Colt45").expect("Colt45 weapon");
    // Per C# YAML: damage 10000, reload 7, range 7c0.
    assert_eq!(w.damage, 10000, "Colt45 damage");
    assert_eq!(w.reload_delay, 7, "Colt45 reload");
    assert_eq!(w.range, 7 * 1024, "Colt45 range (7c0)");
}
