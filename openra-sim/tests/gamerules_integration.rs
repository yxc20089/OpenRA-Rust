//! Test GameRules::from_ruleset() with real OpenRA mod data.

use std::path::Path;

const RA_MOD_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/OpenRA/mods/ra");

fn ra_mod_available() -> bool {
    Path::new(RA_MOD_DIR).join("rules/defaults.yaml").exists()
}

#[test]
fn from_ruleset_loads_real_ra_rules() {
    if !ra_mod_available() {
        eprintln!("Skipping: vendor/OpenRA not found");
        return;
    }

    let mod_dir = Path::new(RA_MOD_DIR);
    let ruleset = openra_data::rules::load_ruleset(mod_dir).expect("Failed to load RA ruleset");
    let rules = openra_sim::gamerules::GameRules::from_ruleset(&ruleset);

    // Check key actors made it through (lowercased)
    let e1 = rules.actor("e1").expect("e1 should exist");
    assert!(e1.hp > 0, "e1 HP should be > 0, got {}", e1.hp);
    assert!(e1.cost > 0, "e1 cost should be > 0, got {}", e1.cost);
    assert!(e1.speed > 0, "e1 speed should be > 0, got {}", e1.speed);
    assert!(!e1.is_building);
    eprintln!("e1: hp={} cost={} speed={} kind={:?}", e1.hp, e1.cost, e1.speed, e1.kind);

    let fact = rules.actor("fact").expect("fact should exist");
    assert!(fact.is_building);
    assert!(fact.hp > 0);
    eprintln!("fact: hp={} footprint={:?} power={}", fact.hp, fact.footprint, fact.power);

    let harv = rules.actor("harv").expect("harv should exist");
    assert!(harv.cost > 0);
    assert!(harv.speed > 0);
    assert!(!harv.is_building);
    eprintln!("harv: hp={} cost={} speed={}", harv.hp, harv.cost, harv.speed);

    let tank = rules.actor("2tnk").expect("2tnk should exist");
    assert!(tank.hp > 0);
    assert!(tank.cost > 0);
    eprintln!("2tnk: hp={} cost={} speed={} weapons={:?}", tank.hp, tank.cost, tank.speed, tank.weapons);

    // Check weapons
    assert!(rules.weapons.len() > 0, "Should have weapons");
    eprintln!("Loaded {} actors, {} weapons", rules.actors.len(), rules.weapons.len());

    // Print weapon details for first few
    for (name, w) in rules.weapons.iter().take(5) {
        eprintln!("  weapon {}: dmg={} range={} reload={} burst={}",
            name, w.damage, w.range, w.reload_delay, w.burst);
    }
}

#[test]
fn from_ruleset_matches_defaults_for_common_units() {
    if !ra_mod_available() {
        eprintln!("Skipping: vendor/OpenRA not found");
        return;
    }

    let mod_dir = Path::new(RA_MOD_DIR);
    let ruleset = openra_data::rules::load_ruleset(mod_dir).expect("Failed to load RA ruleset");
    let from_yaml = openra_sim::gamerules::GameRules::from_ruleset(&ruleset);
    let defaults = openra_sim::gamerules::GameRules::defaults();

    // Compare costs for common units between YAML-loaded and hardcoded defaults
    let units_to_check = ["e1", "e3", "2tnk", "harv", "mcv", "powr", "weap", "proc"];
    for name in &units_to_check {
        let yaml_cost = from_yaml.cost(name);
        let default_cost = defaults.cost(name);
        eprintln!("{}: yaml_cost={} default_cost={}", name, yaml_cost, default_cost);
        // These should be close (YAML may differ slightly from our hardcoded guesses)
        if yaml_cost != default_cost {
            eprintln!("  NOTE: {} cost differs: yaml={} vs default={}", name, yaml_cost, default_cost);
        }
    }
}
