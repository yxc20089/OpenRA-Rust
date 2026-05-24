//! End-to-end coverage for the scenario-declared `termination.max_ticks:`
//! hard deadline. Historically the engine ignored the YAML field and
//! always used `DEFAULT_MAX_TICKS = 10000`, capping every scenario at
//! 10000 ticks (≈ `max_turns ≤ 110` for the bench's 90-ticks/turn
//! non-interrupt cadence). The F11 long-horizon packs need more —
//! `max_turns 140-180` ⇒ reachable max tick `93 + 90·179 = 16203` —
//! so the engine now reads `termination.max_ticks` from the scenario
//! YAML and honours it EXACTLY (no clamp). Any positive `u32` budget
//! is accepted; values that overflow are clamped to `u32::MAX`.
//!
//! This test pins the new behaviour: a scenario declaring
//! `termination.max_ticks: 16500` produces an `Env` whose `max_ticks()`
//! reports 16500, and the run does NOT auto-`done` solely because the
//! engine hit its old 10000 ceiling.
//!
//! See `openra-data/tests/test_scenario_termination_parse.rs` for the
//! parser-level coverage (the YAML field surfaces on `MapDef`).

use openra_train::{Command, Env};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Locate `rush-hour-arena.oramap` somewhere on disk. Mirrors the
/// fallback chain inside `env_termination_flags.rs`.
fn locate_base_map() -> PathBuf {
    let mut tried: Vec<PathBuf> = Vec::new();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("openra-data")
        .join("tests")
        .join("fixtures")
        .join("rush-hour-arena.oramap");
    if fixture.exists() {
        return fixture;
    }
    tried.push(fixture);

    if let Ok(home) = std::env::var("HOME") {
        for candidate in [
            "Projects/openra-rl/maps/rush-hour-arena.oramap",
            "Projects/OpenRA-RL-Training/scenarios/maps/rush-hour-arena.oramap",
        ] {
            let p = PathBuf::from(&home).join(candidate);
            if p.exists() {
                return p;
            }
            tried.push(p);
        }
    }

    panic!(
        "rush-hour-arena.oramap not found — looked at {:?}.",
        tried
    );
}

fn write_scenario(name: &str, body: &str) -> (TempDir, PathBuf) {
    let tmpdir = tempfile::tempdir().expect("tempdir");
    let scenario_path = tmpdir.path().join(name);
    fs::write(&scenario_path, body).expect("write scenario yaml");

    let src = locate_base_map();
    let dest = tmpdir.path().join("rush-hour-arena.oramap");
    fs::copy(&src, &dest).expect("copy rush-hour-arena.oramap into tempdir");

    (tmpdir, scenario_path)
}

/// A minimal scenario with a single agent infantry on an empty map.
/// `termination.enemy_units_killed: false` keeps the engine from
/// auto-`done`ing the moment the enemy slot has no MustBeDestroyed
/// buildings (the scenario declares none).
fn long_horizon_body(max_ticks: u32) -> String {
    format!(
        r#"name: LongHorizonMaxTicks
base_map: rush-hour-arena.oramap
spawn_mcvs: false
starting_cash: 0
agent:
  faction: allies
enemy:
  faction: soviet
actors:
- type: e1
  owner: agent
  position:
  - 20
  - 20
termination:
  max_ticks: {max_ticks}
  enemy_units_killed: false
"#
    )
}

fn open_env_no_override(scenario_path: &Path) -> Env {
    // NB: NO `.with_max_ticks(...)` builder override here — the whole
    // point of this test is that the SCENARIO YAML value is honoured.
    Env::new(scenario_path.to_str().unwrap(), 7).expect("Env::new")
}

#[test]
fn scenario_max_ticks_is_honoured_exactly_no_clamp() {
    let (_tmp, path) = write_scenario("_max_ticks_16500.yaml", &long_horizon_body(16500));
    let env = open_env_no_override(&path);
    assert_eq!(
        env.max_ticks(),
        16500,
        "scenario-declared termination.max_ticks must be honoured \
         exactly (no clamp to 10000)"
    );
}

#[test]
fn scenario_max_ticks_default_is_default_max_ticks() {
    // No termination.max_ticks → fall back to DEFAULT_MAX_TICKS.
    let body = r#"name: DefaultMaxTicks
base_map: rush-hour-arena.oramap
spawn_mcvs: false
starting_cash: 0
agent:
  faction: allies
enemy:
  faction: soviet
actors:
- type: e1
  owner: agent
  position:
  - 20
  - 20
"#;
    let (_tmp, path) = write_scenario("_max_ticks_default.yaml", body);
    let env = open_env_no_override(&path);
    assert_eq!(
        env.max_ticks(),
        openra_train::env::DEFAULT_MAX_TICKS,
        "without termination.max_ticks, fall back to DEFAULT_MAX_TICKS"
    );
}

#[test]
fn engine_runs_past_old_10000_cap_under_long_max_ticks() {
    // The capability check: a passive agent (just `observe`) must NOT
    // auto-`done` at tick 10000 when the scenario declared a higher
    // budget. Each step advances ~90 ticks (3 frames × NetFrameInterval
    // ≈ DEFAULT_TICKS_PER_STEP=30, processed once per frame). 130 steps
    // ≈ 11700 ticks, comfortably past the old 10000 cap but well under
    // the scenario's 16500 budget — the run must still be alive.
    let (_tmp, path) = write_scenario("_runs_past_10k.yaml", &long_horizon_body(16500));
    let mut env = open_env_no_override(&path);
    let _ = env.reset();

    let mut last_tick: i32 = 0;
    let mut done_at: Option<usize> = None;
    for i in 0..130 {
        let r = env.step(&[Command::Observe]);
        last_tick = r.obs.game_tick;
        if r.done {
            done_at = Some(i);
            break;
        }
    }
    assert!(
        done_at.is_none(),
        "engine must not auto-`done` before the scenario's \
         max_ticks (16500); saw done at step {:?}, tick {}",
        done_at,
        last_tick
    );
    assert!(
        last_tick > 10000,
        "expected the run to advance past the old 10000 cap; \
         only reached tick {}",
        last_tick
    );
    assert_eq!(env.max_ticks(), 16500);
}
