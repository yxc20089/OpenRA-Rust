//! Parsing coverage for the top-level `reveal_map:` scenario-YAML flag.
//!
//! `reveal_map: true` disables fog of war for the agent player — the
//! no-fog cells of the bench's perception ablation grid (vision /
//! structured × fog / no-fog). It is parsed by
//! `oramap::parse_scenario_yaml` and re-exposed on `MapDef::reveal_map`.
//!
//! These tests cover only the parsing layer; the end-to-end no-fog
//! observation behaviour is tested in the Python `tests/test_reveal_map.py`.

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

/// Write the scenario + base map into a temp dir and load it via
/// `load_rush_hour_map_with_spawn`. Returns the parsed MapDef.
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
fn reveal_map_defaults_to_false() {
    let map = load_scenario(BODY);
    assert!(
        !map.reveal_map,
        "a scenario that omits reveal_map keeps normal fog of war"
    );
}

#[test]
fn reveal_map_true_is_parsed() {
    let map = load_scenario(&format!("{BODY}reveal_map: true\n"));
    assert!(map.reveal_map, "reveal_map: true must disable fog");
}

#[test]
fn reveal_map_false_is_parsed() {
    let map = load_scenario(&format!("{BODY}reveal_map: false\n"));
    assert!(!map.reveal_map, "reveal_map: false keeps fog");
}
