//! Parsing coverage for the top-level `termination:` scenario-YAML
//! block — specifically the two auto-`done` gating flags
//! `agent_units_killed` / `enemy_units_killed`.
//!
//! Both default to `true` (back-compat: engine auto-`done`s the run
//! when one side's force is wiped). A scenario opts out by declaring
//! the flag `false`. The flags are surfaced on `MapDef` as
//! `terminate_on_{agent,enemy}_units_killed` and consumed by the env
//! layer's `is_terminal` check.
//!
//! The end-to-end "wipe doesn't end the run" behaviour is tested
//! engine-side in `openra-train/tests/env_termination_flags.rs` and
//! over the Python boundary in the bench repo at
//! `tests/test_termination_flags_python.py`.

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

    load_rush_hour_map_with_spawn(&scenario_path, 0).expect("scenario should parse")
}

#[test]
fn termination_flags_default_to_true_when_block_omitted() {
    let map = load_scenario(BODY);
    assert!(
        map.terminate_on_agent_units_killed,
        "a scenario without a termination: block keeps the agent-wipe auto-done"
    );
    assert!(
        map.terminate_on_enemy_units_killed,
        "a scenario without a termination: block keeps the enemy-wipe auto-done"
    );
}

#[test]
fn termination_block_with_both_flags_false_parses() {
    let yaml = format!(
        "{BODY}termination:\n  max_ticks: 5400\n  agent_units_killed: false\n  enemy_units_killed: false\n"
    );
    let map = load_scenario(&yaml);
    assert!(
        !map.terminate_on_agent_units_killed,
        "agent_units_killed: false must disable agent-wipe auto-done"
    );
    assert!(
        !map.terminate_on_enemy_units_killed,
        "enemy_units_killed: false must disable enemy-wipe auto-done"
    );
}

#[test]
fn termination_block_with_only_agent_flag_false_parses() {
    // The canonical suicide-charge idiom: keep the run alive past the
    // strike package's death so the within_ticks fail clause can fire,
    // but still auto-`done` when the enemy fact falls.
    let yaml = format!("{BODY}termination:\n  agent_units_killed: false\n");
    let map = load_scenario(&yaml);
    assert!(!map.terminate_on_agent_units_killed);
    assert!(
        map.terminate_on_enemy_units_killed,
        "enemy flag remains true (omitted ⇒ default)"
    );
}

#[test]
fn termination_block_with_only_enemy_flag_false_parses() {
    let yaml = format!("{BODY}termination:\n  enemy_units_killed: false\n");
    let map = load_scenario(&yaml);
    assert!(map.terminate_on_agent_units_killed);
    assert!(!map.terminate_on_enemy_units_killed);
}

#[test]
fn termination_block_inline_flow_form_parses() {
    // PyYAML emits inline `{k: v, k: v}` for very short blocks; the
    // parser must accept both block and inline forms.
    let yaml = format!(
        "{BODY}termination: {{agent_units_killed: false, enemy_units_killed: false}}\n"
    );
    let map = load_scenario(&yaml);
    assert!(!map.terminate_on_agent_units_killed);
    assert!(!map.terminate_on_enemy_units_killed);
}

#[test]
fn termination_block_with_only_max_ticks_keeps_defaults() {
    // `max_ticks:` is handled bench-side (the env wrapper passes a
    // separate cap). A `termination:` block containing only `max_ticks`
    // must leave the kill-gate flags at their defaults.
    let yaml = format!("{BODY}termination:\n  max_ticks: 6000\n");
    let map = load_scenario(&yaml);
    assert!(map.terminate_on_agent_units_killed);
    assert!(map.terminate_on_enemy_units_killed);
}

#[test]
fn termination_block_explicit_true_round_trips() {
    let yaml = format!(
        "{BODY}termination:\n  agent_units_killed: true\n  enemy_units_killed: true\n"
    );
    let map = load_scenario(&yaml);
    assert!(map.terminate_on_agent_units_killed);
    assert!(map.terminate_on_enemy_units_killed);
}
