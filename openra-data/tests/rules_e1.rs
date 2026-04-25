//! Phase 4 typed Rules test.
//!
//! Loads the real RA mod ruleset from `vendor/OpenRA/mods/ra` and asserts
//! that E1 (Rifleman) and its primary weapon M1Carbine round-trip with the
//! values we expect from the C# OpenRA source.
//!
//! Reference fields (from `vendor/OpenRA/mods/ra/rules/infantry.yaml` and
//! `weapons/smallcaliber.yaml`):
//!   E1.Health.HP                  = 5000
//!   E1.Mobile.Speed (^Infantry)   = 54
//!   E1.RevealsShroud (^Infantry)  = 4c0  → WDist(4096)
//!   E1.Armament@PRIMARY.Weapon    = M1Carbine
//!   M1Carbine.Range               = 5c0  → WDist(5120)
//!   M1Carbine.ReloadDelay         = 20
//!   M1Carbine.Warhead@1Dam.Damage = 1000 (inherited from ^LightMG)

use openra_data::rules::{self, WDist};
use std::path::PathBuf;

fn vendored_mod_dir() -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(format!("{}/../vendor/OpenRA/mods/ra", manifest))
}

#[test]
fn e1_rifleman_typed_fields() {
    let mod_dir = vendored_mod_dir();
    if !mod_dir.exists() {
        panic!(
            "vendored OpenRA mod dir missing at {} — run `git submodule update --init` first",
            mod_dir.display()
        );
    }

    let r = rules::Rules::load(&mod_dir).expect("load Rules");

    let e1 = r.unit("E1").expect("E1 not found in ruleset");
    assert_eq!(e1.hp, 5000, "E1.Health.HP should be 5000");
    assert_eq!(
        e1.speed,
        Some(54),
        "E1.Mobile.Speed should be 54 (inherited from ^Infantry)"
    );
    assert_eq!(
        e1.reveal_range,
        Some(WDist::new(4096)),
        "E1.RevealsShroud.Range should be 4c0 = WDist(4096)"
    );
    assert_eq!(
        e1.primary_weapon.as_deref(),
        Some("M1Carbine"),
        "E1.Armament@PRIMARY.Weapon should be M1Carbine"
    );
    assert!(
        e1.must_be_destroyed,
        "E1 inherits MustBeDestroyed from ^Soldier (counts toward kill-all win)"
    );
    assert!(e1.speed.unwrap() > 0, "speed must be positive");
}

#[test]
fn m1carbine_weapon_typed_fields() {
    let mod_dir = vendored_mod_dir();
    if !mod_dir.exists() {
        return;
    }

    let r = rules::Rules::load(&mod_dir).expect("load Rules");

    let m1 = r.weapon("M1Carbine").expect("M1Carbine not found");
    assert_eq!(m1.range, WDist::new(5120), "M1Carbine.Range = 5c0 = 5120");
    assert_eq!(m1.reload_delay, 20, "M1Carbine.ReloadDelay = 20");
    assert_eq!(
        m1.damage, 1000,
        "M1Carbine.Warhead@1Dam.Damage = 1000 (from ^LightMG)"
    );
}

#[test]
fn rules_load_is_deterministic() {
    let mod_dir = vendored_mod_dir();
    if !mod_dir.exists() {
        return;
    }
    // Load twice in the same process — must produce the same iteration
    // order (BTreeMap-backed) and the same field values for a sanity actor.
    let a = rules::Rules::load(&mod_dir).unwrap();
    let b = rules::Rules::load(&mod_dir).unwrap();
    let a_keys: Vec<_> = a.units.keys().collect();
    let b_keys: Vec<_> = b.units.keys().collect();
    assert_eq!(a_keys, b_keys, "BTreeMap unit iteration must be deterministic");
    let a_e1 = a.unit("E1").unwrap();
    let b_e1 = b.unit("E1").unwrap();
    assert_eq!(a_e1.hp, b_e1.hp);
    assert_eq!(a_e1.speed, b_e1.speed);
    assert_eq!(a_e1.reveal_range, b_e1.reveal_range);
    assert_eq!(a_e1.primary_weapon, b_e1.primary_weapon);
}

#[test]
fn parse_wdist_handles_real_values() {
    // From real YAML files we depend on.
    assert_eq!(rules::parse_wdist("5c0"), Some(WDist::new(5120)));
    assert_eq!(rules::parse_wdist("5c512"), Some(WDist::new(5632)));
    assert_eq!(rules::parse_wdist("4c0"), Some(WDist::new(4096)));
    assert_eq!(rules::parse_wdist("2c512"), Some(WDist::new(2560)));
    assert_eq!(rules::parse_wdist("0c1024"), Some(WDist::new(1024)));
    // Bare integer (no `c`).
    assert_eq!(rules::parse_wdist("1234"), Some(WDist::new(1234)));
    // Junk.
    assert_eq!(rules::parse_wdist("abc"), None);
}
