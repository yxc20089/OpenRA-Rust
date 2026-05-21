//! Parsing coverage for the `scheduled_events:` scenario-YAML block.
//!
//! Wave-9 lets a scenario script mid-episode reinforcement waves,
//! base-teardowns, and deadline-shortening hooks at absolute world
//! ticks. The block is parsed by `oramap::parse_scenario_yaml` and
//! re-exposed on `MapDef::scheduled_events`.
//!
//! These tests cover only the parsing layer; the firing pathway is
//! tested in `openra-train/tests/test_scheduled_events.rs` and the
//! end-to-end behaviour in the Python `tests/test_scheduled_events.py`.

use openra_data::oramap::{
    load_rush_hour_map_with_spawn, ScheduledEventKind,
};
use std::fs;
use std::path::PathBuf;

/// A tiny scenario that pre-places 1 agent unit (required by the
/// validator) and declares three scheduled events: a spawn at tick
/// 1500, a region-destroy at tick 3000, and a deadline shortener at
/// tick 4000.
const SCENARIO: &str = r#"
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
scheduled_events:
- tick: 1500
  type: spawn_actors
  actors:
  - type: 3tnk
    owner: enemy
    position:
    - 80
    - 20
    stance: 3
    count: 2
- tick: 3000
  type: destroy_actors
  filter:
    owner: enemy
    region:
      x: 50
      y: 20
      radius: 8
- tick: 4000
  type: shorten_deadline
  new_max_ticks: 4500
"#;

/// Write the scenario + base map into a temp dir and load it via
/// `load_rush_hour_map_with_spawn`. Returns the parsed MapDef.
fn load_scenario(text: &str) -> openra_data::oramap::MapDef {
    let tmpdir = tempfile::tempdir().expect("tempdir");
    let scenario_path: PathBuf = tmpdir.path().join("scen.yaml");
    fs::write(&scenario_path, text).unwrap();

    // Copy the rush-hour arena .oramap next to the scenario file so the
    // loader's base-map fallback chain finds it without any HOME/env
    // wiring. The fixture is shipped with the openra-data crate.
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("rush-hour-arena.oramap");
    let dest = tmpdir.path().join("rush-hour-arena.oramap");
    if fixture.exists() {
        fs::copy(&fixture, &dest).unwrap();
    } else if let Ok(home) = std::env::var("HOME") {
        // Fall back to the canonical OpenRA-RL-Training fixture location
        // (the loader walks it on its own, but for hermetic tests we
        // copy it into the scenario dir).
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
fn parses_three_event_kinds() {
    let m = load_scenario(SCENARIO);
    assert_eq!(
        m.scheduled_events.len(),
        3,
        "expected 3 events, got {:?}",
        m.scheduled_events.len()
    );

    // Event 0 — spawn_actors at tick 1500, expanded `count: 2` into
    // two `3tnk` actors on DISTINCT cells (count: N no longer stacks
    // units on the anchor — copy 0 keeps the declared position, copies
    // 1.. spread to the nearest free cells in outward rings).
    let ev = &m.scheduled_events[0];
    assert_eq!(ev.tick, 1500);
    match &ev.kind {
        ScheduledEventKind::SpawnActors { actors } => {
            assert_eq!(actors.len(), 2);
            for a in actors {
                assert_eq!(a.actor_type, "3tnk");
                assert_eq!(a.owner, "enemy");
                assert_eq!(a.stance, Some(3));
            }
            assert_eq!(
                actors[0].position, (80, 20),
                "copy 0 keeps the declared anchor"
            );
            assert_ne!(
                actors[0].position, actors[1].position,
                "count: 2 must spawn on two distinct cells, not stacked"
            );
        }
        other => panic!("expected SpawnActors, got {other:?}"),
    }

    // Event 1 — destroy_actors with owner + region filter.
    let ev = &m.scheduled_events[1];
    assert_eq!(ev.tick, 3000);
    match &ev.kind {
        ScheduledEventKind::DestroyActors { filter } => {
            assert_eq!(filter.owner.as_deref(), Some("enemy"));
            let r = filter.region.expect("region filter required");
            assert_eq!(r.x, 50);
            assert_eq!(r.y, 20);
            assert_eq!(r.radius, 8);
        }
        other => panic!("expected DestroyActors, got {other:?}"),
    }

    // Event 2 — shorten_deadline.
    let ev = &m.scheduled_events[2];
    assert_eq!(ev.tick, 4000);
    match &ev.kind {
        ScheduledEventKind::ShortenDeadline { new_max_ticks } => {
            assert_eq!(*new_max_ticks, 4500);
        }
        other => panic!("expected ShortenDeadline, got {other:?}"),
    }
}

#[test]
fn scenario_without_scheduled_events_is_empty() {
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
"#;
    let m = load_scenario(scen);
    assert!(m.scheduled_events.is_empty());
}

#[test]
fn unknown_event_type_is_skipped() {
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
scheduled_events:
- tick: 100
  type: not_a_real_event
- tick: 200
  type: shorten_deadline
  new_max_ticks: 1000
"#;
    let m = load_scenario(scen);
    // Only the recognised second entry survives — the unknown one is a
    // tolerant skip (forward-compat for future event kinds).
    assert_eq!(m.scheduled_events.len(), 1);
    assert_eq!(m.scheduled_events[0].tick, 200);
}
