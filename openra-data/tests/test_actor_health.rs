//! Parsing coverage for the per-actor `health:` scenario-YAML field.
//!
//! A scenario may pre-place a damaged actor by writing `health: N`
//! (an HP percentage, 1-100) on an `actors:` list item. The field is
//! parsed by `oramap::parse_scenario_yaml` and re-exposed on each
//! `MapDef::actors[*].health`. `None` means "spawn at full HP".
//!
//! These tests cover only the parsing layer. End-to-end "the spawned
//! actor's Health trait is scaled" is covered by the Python test
//! `tests/test_actor_health_field.py`.

use openra_data::oramap::load_rush_hour_map_with_spawn;
use std::fs;
use std::path::PathBuf;

/// Write the scenario + base map into a temp dir and load it.
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

/// A `proc` placed with `health: 40` carries `Some(40)`; a sibling
/// actor with no `health:` carries `None` (⇒ full HP at spawn).
#[test]
fn parses_per_actor_health_percentage() {
    let scen = r#"
base_map: rush-hour-arena.oramap
agent:
  faction: allies
enemy:
  faction: soviet
actors:
- type: proc
  owner: agent
  position:
  - 10
  - 18
  health: 40
- type: fact
  owner: agent
  position:
  - 8
  - 18
"#;
    let m = load_scenario(scen);
    let proc = m
        .actors
        .iter()
        .find(|a| a.actor_type == "proc")
        .expect("proc actor present");
    assert_eq!(proc.health, Some(40), "proc should keep health: 40");

    let fact = m
        .actors
        .iter()
        .find(|a| a.actor_type == "fact")
        .expect("fact actor present");
    assert_eq!(fact.health, None, "fact omits health: ⇒ None (full HP)");
}

/// `health:` is clamped into the 1-100 range; `count:` expansion
/// propagates the same health to every expanded actor.
#[test]
fn health_is_clamped_and_propagates_through_count() {
    let scen = r#"
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
  health: 250
  count: 3
- type: e1
  owner: enemy
  position:
  - 60
  - 20
  health: 0
"#;
    let m = load_scenario(scen);
    let agent_inf: Vec<_> = m
        .actors
        .iter()
        .filter(|a| a.owner == "agent" && a.actor_type == "e1")
        .collect();
    assert_eq!(agent_inf.len(), 3, "count: 3 expands to 3 actors");
    for a in &agent_inf {
        assert_eq!(a.health, Some(100), "health: 250 clamps to 100");
    }
    // count: N spawns N units on N DISTINCT cells — never stacked.
    let cells: std::collections::HashSet<_> =
        agent_inf.iter().map(|a| a.position).collect();
    assert_eq!(
        cells.len(), 3,
        "count: 3 must spawn 3 distinct cells, got {cells:?}"
    );

    let enemy = m
        .actors
        .iter()
        .find(|a| a.owner == "enemy")
        .expect("enemy actor present");
    assert_eq!(enemy.health, Some(1), "health: 0 clamps up to 1");
}

/// Inline flow form `position: [x, y]` with a trailing `health:` line
/// still parses the health field (the form curated packs author).
#[test]
fn health_parses_alongside_inline_position() {
    let scen = r#"
base_map: rush-hour-arena.oramap
agent:
  faction: allies
enemy:
  faction: soviet
actors:
- type: pbox
  owner: agent
  position: [16, 18]
  health: 20
  stance: 3
"#;
    let m = load_scenario(scen);
    let pbox = m
        .actors
        .iter()
        .find(|a| a.actor_type == "pbox")
        .expect("pbox actor present");
    assert_eq!(pbox.position, (16, 18));
    assert_eq!(pbox.health, Some(20));
    assert_eq!(pbox.stance, Some(3));
}
