//! Engine regression: a scenario that places NO enemy actor must not
//! be auto-terminated by the enemy-elimination check (it was `done` at
//! tick 0). Termination is then driven solely by max_ticks / the agent
//! being wiped / the bench-side declarative win_condition.

use openra_train::{Command, Env};
use std::io::Write;
use std::path::PathBuf;

const SCENARIO: &str = r#"name: NoEnemy
base_map: ../maps/rush-hour-arena.oramap
spawn_mcvs: false
starting_cash: 1000
agent:
  faction: allies
enemy:
  faction: soviet
actors:
- type: jeep
  owner: agent
  position:
  - 8
  - 12
- type: e1
  owner: agent
  position:
  - 9
  - 14
  count: 2
termination:
  max_ticks: 20000
"#;

fn write_scenario() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let dir = PathBuf::from(&home).join("Projects/OpenRA-RL-Training/scenarios/discovery");
    if !dir.join("rush-hour.yaml").exists() {
        return None;
    }
    let p = dir.join("_no_enemy_test.yaml");
    std::fs::File::create(&p).ok()?.write_all(SCENARIO.as_bytes()).ok()?;
    Some(p)
}

#[test]
fn no_enemy_scenario_does_not_instantly_terminate() {
    let Some(path) = write_scenario() else {
        eprintln!("skip: RL-Training scenarios tree not present");
        return;
    };
    let mut env = Env::new(path.to_str().unwrap(), 7).expect("Env::new");
    let o = env.reset();
    assert!(!o.unit_positions.is_empty(), "agent units must load from YAML");

    let mut done_at = None;
    for i in 0..20 {
        let ids: Vec<String> = env
            .last_observation()
            .unit_positions
            .iter()
            .map(|(id, _)| id.clone())
            .collect();
        let r = env.step(&[Command::MoveUnits {
            unit_ids: ids,
            target_x: 40,
            target_y: 16,
        }]);
        if r.done {
            done_at = Some(i);
            break;
        }
    }
    assert!(
        done_at.is_none(),
        "no-enemy scenario terminated at step {:?} (should run to max_ticks)",
        done_at
    );
    assert!(env.last_observation().game_tick > 50);

    let _ = std::fs::remove_file(&path);
}
