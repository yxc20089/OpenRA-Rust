//! Scripted opponent subsystem (`enemy: {bot: ...}`).
//!
//! End-to-end: the scenario YAML's `enemy.bot` is parsed, a
//! `ScriptedBot` is attached for the enemy player, and it drives the
//! pre-placed enemy actors with the chosen map-agnostic behaviour.
//! We assert the *observable consequence* (a hunting enemy pursues and
//! destroys a distant agent unit; a turtling enemy does not), which
//! proves parse → attach → per-tick behaviour all work.

use openra_train::{Command, Env};
use std::io::Write;
use std::path::PathBuf;

fn scenario(name: &str, bot: &str) -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let dir = PathBuf::from(&home).join("Projects/OpenRA-RL-Training/scenarios/discovery");
    if !dir.join("rush-hour.yaml").exists() {
        return None;
    }
    let yaml = format!(
        "name: BotTest\n\
         base_map: ../maps/rush-hour-arena.oramap\n\
         spawn_mcvs: false\n\
         agent:\n  faction: allies\n\
         enemy:\n  faction: soviet\n  bot: {bot}\n\
         actors:\n\
         - type: e1\n  owner: agent\n  position:\n  - 20\n  - 20\n\
         - type: 3tnk\n  owner: enemy\n  position:\n  - 70\n  - 20\n  stance: 3\n\
         termination:\n  max_ticks: 40000\n"
    );
    let p = dir.join(format!("_botscript_{name}.yaml"));
    std::fs::File::create(&p).ok()?.write_all(yaml.as_bytes()).ok()?;
    Some(p)
}

fn agent_alive(env: &mut Env, steps: usize) -> bool {
    // Run `steps` no-op turns; report whether the agent's lone e1
    // still exists at the end.
    let mut last = true;
    for _ in 0..steps {
        let r = env.step(&[Command::Observe]);
        last = !r.obs.unit_positions.is_empty();
        if r.done {
            break;
        }
    }
    last
}

#[test]
fn hunt_bot_pursues_and_destroys_a_distant_agent_unit() {
    let Some(path) = scenario("hunt", "hunt") else {
        eprintln!("skip: RL-Training scenarios not present");
        return;
    };
    let mut env = Env::new(path.to_str().unwrap(), 7).unwrap();
    let n0 = env.reset().unit_positions.len();
    assert_eq!(n0, 1, "agent starts with one e1");
    // The 3tnk is 50 cells away; with `hunt` it must close the gap
    // and kill the rifleman well within the budget.
    let alive = agent_alive(&mut env, 80);
    assert!(!alive, "hunt bot should have pursued and destroyed the e1");
}

#[test]
fn turtle_bot_holds_position_and_spares_the_distant_unit() {
    let Some(path) = scenario("turtle", "turtle") else {
        eprintln!("skip: RL-Training scenarios not present");
        return;
    };
    let mut env = Env::new(path.to_str().unwrap(), 7).unwrap();
    assert_eq!(env.reset().unit_positions.len(), 1);
    // Same geometry, but `turtle` holds the spawn — the far e1 lives.
    let alive = agent_alive(&mut env, 80);
    assert!(alive, "turtle bot must not leave its post to hunt the e1");
}

#[test]
fn unknown_bot_name_is_ignored_not_fatal() {
    let Some(path) = scenario("bogus", "definitely-not-a-behaviour") else {
        eprintln!("skip: RL-Training scenarios not present");
        return;
    };
    let mut env = Env::new(path.to_str().unwrap(), 7).unwrap();
    assert_eq!(env.reset().unit_positions.len(), 1);
    // No panic, episode runs; enemy falls back to stance-only.
    let _ = env.step(&[Command::Observe]);
}
