//! Parsing coverage for the per-player starting-cash overrides on the
//! `agent:` / `enemy:` scenario-YAML blocks.
//!
//! Before the fix, `oramap::parse_scenario_yaml` only honoured the
//! top-level `starting_cash:` value and silently dropped a `cash:`
//! field tucked inside the `agent: { ... }` / `enemy: { ... }`
//! blocks. The engine (`build_world` ⇒ `build_player_traits`) then
//! gave every player the same cash, which broke scenarios like
//! `spec-thief-steal-cash` (the enemy has nothing to steal).
//!
//! These tests cover only the parsing layer; the engine-side
//! behaviour is pinned in
//! `openra-sim/tests/test_per_player_starting_cash.rs`.

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
fn agent_and_enemy_cash_default_to_none() {
    // A scenario that doesn't declare per-player cash leaves both
    // overrides None ⇒ the engine falls back to the lobby-wide
    // `starting_cash`. This is the back-compat path.
    let map = load_scenario(BODY);
    assert_eq!(map.agent_starting_cash, None);
    assert_eq!(map.enemy_starting_cash, None);
}

#[test]
fn agent_cash_is_parsed() {
    let scen = r#"
base_map: rush-hour-arena.oramap
agent:
  faction: allies
  cash: 500
enemy:
  faction: soviet
actors:
- type: e1
  owner: agent
  position:
  - 5
  - 5
"#;
    let map = load_scenario(scen);
    assert_eq!(map.agent_starting_cash, Some(500));
    assert_eq!(map.enemy_starting_cash, None);
}

#[test]
fn enemy_cash_is_parsed() {
    let scen = r#"
base_map: rush-hour-arena.oramap
agent:
  faction: allies
enemy:
  faction: soviet
  cash: 1500
actors:
- type: e1
  owner: agent
  position:
  - 5
  - 5
"#;
    let map = load_scenario(scen);
    assert_eq!(map.agent_starting_cash, None);
    assert_eq!(map.enemy_starting_cash, Some(1500));
}

#[test]
fn both_overrides_coexist_with_bot_type() {
    // Per-player cash must not collide with the `bot_type:` field on
    // the same enemy block — both must surface.
    let scen = r#"
base_map: rush-hour-arena.oramap
agent:
  faction: allies
  cash: 0
enemy:
  faction: soviet
  cash: 2000
  bot_type: hunt
actors:
- type: e1
  owner: agent
  position:
  - 5
  - 5
"#;
    let map = load_scenario(scen);
    assert_eq!(map.agent_starting_cash, Some(0));
    assert_eq!(map.enemy_starting_cash, Some(2000));
    assert_eq!(map.enemy_bot.as_deref(), Some("hunt"));
}
