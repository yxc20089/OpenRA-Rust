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

use openra_train::{Command, Env};
use std::io::Write;
use std::path::PathBuf;

fn write_scenario(name: &str, body: &str) -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let dir = PathBuf::from(&home).join("Projects/OpenRA-RL-Training/scenarios/discovery");
    if !dir.join("rush-hour.yaml").exists() {
        return None;
    }
    let p = dir.join(name);
    std::fs::File::create(&p)
        .ok()?
        .write_all(body.as_bytes())
        .ok()?;
    Some(p)
}

/// A single fragile agent infantry adjacent to a much stronger enemy
/// tank — the agent gets wiped within a handful of ticks. Used for
/// the "agent_units_killed" branch.
fn agent_wipe_body(agent_units_killed: &str) -> String {
    format!(
        r#"name: TerminationAgentWipe
base_map: ../maps/rush-hour-arena.oramap
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
base_map: ../maps/rush-hour-arena.oramap
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

#[test]
fn default_agent_wipe_ends_run() {
    let Some(path) = write_scenario("_term_agent_default.yaml", &agent_wipe_body("true")) else {
        eprintln!("skip: RL-Training scenarios not present");
        return;
    };
    let mut env = Env::new(path.to_str().unwrap(), 7)
        .unwrap()
        .with_max_ticks(40000);
    let _ = env.reset();
    let (done_at, _tick) = run_until_done(&mut env, 200);
    assert!(
        done_at.is_some(),
        "default agent_units_killed:true must auto-`done` on agent wipe"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn agent_units_killed_false_keeps_run_alive_past_agent_wipe() {
    let Some(path) = write_scenario("_term_agent_false.yaml", &agent_wipe_body("false")) else {
        eprintln!("skip: RL-Training scenarios not present");
        return;
    };
    let mut env = Env::new(path.to_str().unwrap(), 7)
        .unwrap()
        .with_max_ticks(40000);
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
    let _ = std::fs::remove_file(&path);
}

#[test]
fn default_enemy_wipe_ends_run() {
    let Some(path) = write_scenario("_term_enemy_default.yaml", &enemy_wipe_body("true")) else {
        eprintln!("skip: RL-Training scenarios not present");
        return;
    };
    let mut env = Env::new(path.to_str().unwrap(), 7)
        .unwrap()
        .with_max_ticks(40000);
    let _ = env.reset();
    let (done_at, _tick) = run_until_done(&mut env, 300);
    assert!(
        done_at.is_some(),
        "default enemy_units_killed:true must auto-`done` on enemy wipe"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn enemy_units_killed_false_keeps_run_alive_past_enemy_wipe() {
    let Some(path) = write_scenario("_term_enemy_false.yaml", &enemy_wipe_body("false")) else {
        eprintln!("skip: RL-Training scenarios not present");
        return;
    };
    let mut env = Env::new(path.to_str().unwrap(), 7)
        .unwrap()
        .with_max_ticks(40000);
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
    let _ = std::fs::remove_file(&path);
}
