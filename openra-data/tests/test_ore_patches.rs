//! Parsing coverage for the top-level `ore_patches:` scenario-YAML
//! block (resource-wave feature).
//!
//! Each entry materialises (in `openra-train::env`) into a disk of
//! ore cells on the live terrain via `openra_sim::resource::seed_ore_patch`.
//! These tests cover only the parsing layer; the end-to-end harvest
//! loop is tested in `openra-sim/tests/test_resource_layer.rs`.

use openra_data::oramap::load_rush_hour_map_with_spawn;
use std::fs;
use std::path::PathBuf;

const BODY: &str = r#"
base_map: rush-hour-arena.oramap
agent:
  faction: allies
enemy:
  faction: soviet
actors:
- type: e1
  owner: agent
  position:
  - 5
  - 5
"#;

fn load_scenario(text: &str) -> openra_data::oramap::MapDef {
    let tmpdir = tempfile::tempdir().expect("tempdir");
    let scenario_path: PathBuf = tmpdir.path().join("scen.yaml");
    fs::write(&scenario_path, text).unwrap();

    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("rush-hour-arena.oramap");
    let dest = tmpdir.path().join("rush-hour-arena.oramap");
    if fixture.exists() {
        fs::copy(&fixture, &dest).unwrap();
    } else if let Ok(home) = std::env::var("HOME") {
        let candidate = PathBuf::from(&home)
            .join("Projects/OpenRA-RL-Training/scenarios/maps/rush-hour-arena.oramap");
        if candidate.exists() {
            fs::copy(&candidate, &dest).unwrap();
        }
    }

    load_rush_hour_map_with_spawn(&scenario_path, 0)
        .expect("scenario should parse")
}

#[test]
fn ore_patches_default_empty() {
    let map = load_scenario(BODY);
    assert!(
        map.ore_patches.is_empty(),
        "a scenario that omits ore_patches has no patches"
    );
}

#[test]
fn ore_patches_block_form_parses() {
    let yaml = format!(
        "{BODY}ore_patches:\n  - x: 50\n    y: 20\n    amount: 5000\n    radius: 3\n  - x: 90\n    y: 60\n    amount: 1000\n"
    );
    let map = load_scenario(&yaml);
    assert_eq!(map.ore_patches.len(), 2, "two patches must parse");

    let p0 = map.ore_patches[0];
    assert_eq!(p0.x, 50);
    assert_eq!(p0.y, 20);
    assert_eq!(p0.amount, 5000);
    assert_eq!(p0.radius, 3);

    let p1 = map.ore_patches[1];
    assert_eq!(p1.x, 90);
    assert_eq!(p1.y, 60);
    assert_eq!(p1.amount, 1000);
    // Omitted radius defaults to 3.
    assert_eq!(p1.radius, 3);
}

#[test]
fn ore_patches_compact_form_parses() {
    // PyYAML emits list items at column 0 (compact form). Mirror the
    // shape `scheduled_events:` parsing handles.
    let yaml = format!(
        "{BODY}ore_patches:\n- x: 40\n  y: 40\n  amount: 750\n  radius: 1\n"
    );
    let map = load_scenario(&yaml);
    assert_eq!(map.ore_patches.len(), 1);
    let p = map.ore_patches[0];
    assert_eq!((p.x, p.y, p.amount, p.radius), (40, 40, 750, 1));
}
