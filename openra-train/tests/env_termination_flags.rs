//! End-to-end coverage for `termination.{agent,enemy}_units_killed`.
//!
//! Default (both true): the engine auto-`done`s the run the moment
//! either side has no surviving combat units / MustBeDestroyed
//! buildings. This is the legacy behaviour.
//!
//! Opt-out (`termination.agent_units_killed: false` /
//! `termination.enemy_units_killed: false`): the engine no longer
//! ends the run on that side's wipe; the episode keeps advancing
//! until the tick deadline (or the OTHER side's wipe, if its flag is
//! still true). This is what packs like
//! `combat-suicide-charge-mission` rely on so a within_ticks fail
//! clause can fire after the strike package dies.
//!
//! Each test builds its scenario YAML in a process-local tempdir and
//! copies the `rush-hour-arena.oramap` base map alongside it, so the
//! test is self-contained and exercises the engine even on a fresh
//! checkout. The base map is located by walking the same fallback
//! chain `oramap::load_rush_hour_map_with_spawn` itself walks, so a
//! checkout that has the engine wheel working will also run these
//! tests. If no candidate base map is found we panic loudly (rather
//! than silently skipping, which was the pre-fix degeneracy).

use openra_train::{Command, Env};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Locate `rush-hour-arena.oramap` somewhere on disk. Mirrors the
/// fallback chain inside `openra-data/src/oramap.rs` so that any
/// machine that can run the engine wheel can also run these tests.
fn locate_base_map() -> PathBuf {
    let mut tried: Vec<PathBuf> = Vec::new();
    // 1. In-repo fixture (preferred if a future patch checks it in).
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
        "rush-hour-arena.oramap not found — looked at {:?}. Either \
         check a fixture into openra-data/tests/fixtures/, or restore \
         the OpenRA-RL-Training scenarios tree under $HOME/Projects/.",
        tried
    );
}

/// Build a scenario tempdir containing the YAML body + a copy of the
/// base map alongside it (so `base_map: rush-hour-arena.oramap`
/// resolves at scenario_dir-relative path 1, no $HOME lookup needed).
/// Returns `(tmpdir, scenario_yaml_path)` — keep `tmpdir` alive for
/// the duration of the test so the on-disk files stick around.
fn write_scenario(name: &str, body: &str) -> (TempDir, PathBuf) {
    let tmpdir = tempfile::tempdir().expect("tempdir");
    let scenario_path = tmpdir.path().join(name);
    fs::write(&scenario_path, body).expect("write scenario yaml");

    let src = locate_base_map();
    let dest = tmpdir.path().join("rush-hour-arena.oramap");
    fs::copy(&src, &dest).expect("copy rush-hour-arena.oramap into tempdir");

    (tmpdir, scenario_path)
}

/// A single fragile agent infantry adjacent to a much stronger enemy
/// tank — the agent gets wiped within a handful of ticks. Used for
/// the "agent_units_killed" branch.
fn agent_wipe_body(agent_units_killed: &str) -> String {
    format!(
        r#"name: TerminationAgentWipe
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
- type: 4tnk
  owner: enemy
  position:
  - 21
  - 20
termination:
  max_ticks: 40000
  agent_units_killed: {agent_units_killed}
"#
    )
}

/// One e1 vs one e1 mirror, but the agent's stance is forced via an
/// explicit attack — used for the "enemy_units_killed" branch (the
/// agent kills the enemy and we check whether the engine ends the
/// run on enemy wipe).
fn enemy_wipe_body(enemy_units_killed: &str) -> String {
    format!(
        r#"name: TerminationEnemyWipe
base_map: rush-hour-arena.oramap
spawn_mcvs: false
starting_cash: 0
agent:
  faction: allies
enemy:
  faction: soviet
actors:
- type: 4tnk
  owner: agent
  position:
  - 20
  - 20
- type: e1
  owner: enemy
  position:
  - 21
  - 20
  stance: 0
termination:
  max_ticks: 40000
  enemy_units_killed: {enemy_units_killed}
"#
    )
}

/// MBD-building variant: the enemy roster is a single `fact`
/// (MustBeDestroyed) plus a stance:0 e1 used only to bootstrap
/// `enemy_started_with_buildings == true` ⇒ enemy aliveness is checked
/// via `has_must_be_destroyed_buildings`, not `has_combat_units`. The
/// agent (4× 4tnk swarm + adjacent to the fact) razes the fact in a
/// handful of ticks. This is the path the qwen-9B-sweep DRAW witnesses
/// asked about — confirms `enemy_units_killed: false` ALSO gates the
/// MBD-removal auto-`done`, not only the units-killed auto-`done`.
fn enemy_mbd_wipe_body(enemy_units_killed: &str) -> String {
    format!(
        r#"name: TerminationEnemyMbdWipe
base_map: rush-hour-arena.oramap
spawn_mcvs: false
starting_cash: 0
agent:
  faction: allies
enemy:
  faction: soviet
actors:
- type: 4tnk
  owner: agent
  position:
  - 18
  - 20
- type: 4tnk
  owner: agent
  position:
  - 18
  - 21
- type: 4tnk
  owner: agent
  position:
  - 18
  - 22
- type: 4tnk
  owner: agent
  position:
  - 18
  - 23
- type: fact
  owner: enemy
  position:
  - 22
  - 20
- type: e1
  owner: enemy
  position:
  - 60
  - 20
  stance: 0
termination:
  max_ticks: 40000
  enemy_units_killed: {enemy_units_killed}
"#
    )
}

/// Helper: run up to `max_steps` and return the (step_idx_when_done,
/// final_observation_tick). `step_idx_when_done` is `None` if the run
/// never terminated.
fn run_until_done(env: &mut Env, max_steps: usize) -> (Option<usize>, i32) {
    for i in 0..max_steps {
        let r = env.step(&[Command::Observe]);
        if r.done {
            return (Some(i), r.obs.game_tick);
        }
    }
    (None, env.last_observation().game_tick)
}

fn open_env(scenario_path: &Path) -> Env {
    Env::new(scenario_path.to_str().unwrap(), 7)
        .expect("Env::new")
        .with_max_ticks(40000)
}

#[test]
fn default_agent_wipe_ends_run() {
    let (_tmp, path) = write_scenario("_term_agent_default.yaml", &agent_wipe_body("true"));
    let mut env = open_env(&path);
    let _ = env.reset();
    let (done_at, _tick) = run_until_done(&mut env, 200);
    assert!(
        done_at.is_some(),
        "default agent_units_killed:true must auto-`done` on agent wipe"
    );
}

#[test]
fn agent_units_killed_false_keeps_run_alive_past_agent_wipe() {
    let (_tmp, path) = write_scenario("_term_agent_false.yaml", &agent_wipe_body("false"));
    let mut env = open_env(&path);
    let _ = env.reset();
    // Run far past the point at which the agent e1 would be dead vs.
    // an adjacent 4tnk (a few volleys, well under 50 steps × 30 t/step).
    let mut still_running_at = 0;
    for i in 0..120 {
        let r = env.step(&[Command::Observe]);
        if r.done {
            panic!(
                "agent_units_killed:false must NOT auto-done on agent wipe \
                 (terminated at step {i}, tick {})",
                r.obs.game_tick
            );
        }
        still_running_at = i;
    }
    assert!(
        still_running_at >= 100,
        "loop must have advanced at least 100 steps"
    );
    // The agent infantry should be dead by now — that's the whole
    // point of "wipe doesn't end the run".
    let agent_pid = env.agent_player_id();
    let world = env.world().expect("world");
    let snap = world.snapshot();
    let agent_combat_alive = snap
        .actors
        .iter()
        .any(|a| a.owner == agent_pid && a.hp > 0);
    assert!(
        !agent_combat_alive,
        "the agent unit should have been wiped by tick {} \
         (otherwise the test isn't actually exercising the opt-out)",
        snap.tick
    );
}

#[test]
fn default_enemy_wipe_ends_run() {
    let (_tmp, path) = write_scenario("_term_enemy_default.yaml", &enemy_wipe_body("true"));
    let mut env = open_env(&path);
    let _ = env.reset();
    let (done_at, _tick) = run_until_done(&mut env, 300);
    assert!(
        done_at.is_some(),
        "default enemy_units_killed:true must auto-`done` on enemy wipe"
    );
}

#[test]
fn enemy_units_killed_false_keeps_run_alive_past_enemy_wipe() {
    let (_tmp, path) = write_scenario("_term_enemy_false.yaml", &enemy_wipe_body("false"));
    let mut env = open_env(&path);
    let _ = env.reset();
    // Many steps — the 4tnk obliterates the stance:0 e1 quickly.
    for i in 0..150 {
        let r = env.step(&[Command::Observe]);
        if r.done {
            panic!(
                "enemy_units_killed:false must NOT auto-done on enemy wipe \
                 (terminated at step {i}, tick {})",
                r.obs.game_tick
            );
        }
    }
    let enemy_pid = env.enemy_player_id();
    let world = env.world().expect("world");
    let snap = world.snapshot();
    let enemy_combat_alive = snap
        .actors
        .iter()
        .any(|a| a.owner == enemy_pid && a.hp > 0);
    assert!(
        !enemy_combat_alive,
        "the enemy unit should have been wiped (otherwise the test isn't \
         actually exercising the opt-out)"
    );
}

// ---- MBD-building wipe coverage --------------------------------------------
// The two tests above use an enemy `e1` (combat unit), exercising
// `has_combat_units(enemy)` — the `enemy_started_with_buildings == false`
// branch. The qwen-9B-sweep DRAW investigation (2026-05-25) asked
// whether the `enemy_units_killed` flag ALSO governs the
// `enemy_started_with_buildings == true` branch, where enemy aliveness
// is checked via `has_must_be_destroyed_buildings`. The two tests below
// PIN that yes — it does. Without the gate, a pack that places a
// killable enemy `fact` marker (to defeat the auto-done) re-introduces
// the very DRAW race the marker was meant to prevent.

#[test]
fn default_enemy_mbd_wipe_ends_run() {
    let (_tmp, path) = write_scenario(
        "_term_enemy_mbd_default.yaml",
        &enemy_mbd_wipe_body("true"),
    );
    let mut env = open_env(&path);
    let _ = env.reset();
    let (done_at, _tick) = run_until_done(&mut env, 600);
    assert!(
        done_at.is_some(),
        "default enemy_units_killed:true must auto-`done` on enemy MBD-building wipe"
    );
}

#[test]
fn enemy_units_killed_false_keeps_run_alive_past_mbd_wipe() {
    let (_tmp, path) = write_scenario(
        "_term_enemy_mbd_false.yaml",
        &enemy_mbd_wipe_body("false"),
    );
    let mut env = open_env(&path);
    let _ = env.reset();
    // 4× 4tnk pummel an adjacent `fact` — it's razed in ~a few hundred
    // ticks. Run well past that and assert the run is still live.
    for i in 0..400 {
        let r = env.step(&[Command::Observe]);
        if r.done {
            panic!(
                "enemy_units_killed:false must NOT auto-done on enemy MBD-building wipe \
                 (terminated at step {i}, tick {})",
                r.obs.game_tick
            );
        }
    }
    // Confirm the fact actually died — otherwise the test isn't
    // exercising the opt-out.
    let enemy_pid = env.enemy_player_id();
    let world = env.world().expect("world");
    let snap = world.snapshot();
    let enemy_fact_alive = snap
        .actors
        .iter()
        .any(|a| a.owner == enemy_pid && a.actor_type == "fact" && a.hp > 0);
    assert!(
        !enemy_fact_alive,
        "the enemy fact should have been razed by 400 steps \
         (otherwise the test isn't actually exercising the MBD opt-out)"
    );
}
