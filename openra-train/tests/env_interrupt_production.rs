//! Step 4: `production_complete` interrupt — step_until_event returns
//! early when a queued unit finishes (construction complete), and
//! starting actors do NOT false-fire it (baseline on first check).

use openra_train::{Command, Env};
use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;

const SCENARIO: &str = r#"name: ProdInterrupt
base_map: ../maps/rush-hour-arena.oramap
spawn_mcvs: false
starting_cash: 4000
agent:
  faction: allies
enemy:
  faction: soviet
actors:
- type: barr
  owner: agent
  position:
  - 10
  - 20
- type: powr
  owner: agent
  position:
  - 14
  - 20
- type: jeep
  owner: agent
  position:
  - 8
  - 18
- type: e1
  owner: enemy
  position:
  - 114
  - 34
  stance: 2
termination:
  max_ticks: 40000
"#;

fn scenario() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let dir = PathBuf::from(&home).join("Projects/OpenRA-RL-Training/scenarios/discovery");
    if !dir.join("rush-hour.yaml").exists() {
        return None;
    }
    let p = dir.join("_prod_interrupt_test.yaml");
    std::fs::File::create(&p).ok()?.write_all(SCENARIO.as_bytes()).ok()?;
    Some(p)
}

#[test]
fn production_complete_fires_on_finished_unit_not_on_start() {
    let Some(path) = scenario() else {
        eprintln!("skip: RL-Training scenarios not present");
        return;
    };
    let mut env = Env::new(path.to_str().unwrap(), 7).expect("Env::new");
    env.reset();
    let sig: HashSet<String> = ["production_complete".to_string()].into_iter().collect();

    // First advance with NO production queued: starting actors
    // (barr/powr/jeep) must NOT false-fire production_complete.
    let r0 = env.step_until_event(&[Command::Observe], 60, 5, Some(sig.clone()));
    assert!(
        !(r0.interrupted
            && r0.interrupt_reason.as_deref().unwrap_or("").contains("production_complete")),
        "starting actors must not fire production_complete (got {:?})",
        r0.interrupt_reason
    );

    // Queue an E1 (barracks satisfies the tech prereq), then advance:
    // the moment the unit completes, step_until_event returns early
    // with production_complete.
    env.step(&[Command::Build { item: "e1".into() }]);
    let mut fired = false;
    for _ in 0..40 {
        let r = env.step_until_event(&[Command::Observe], 200, 5, Some(sig.clone()));
        if r.interrupted
            && r.interrupt_reason
                .as_deref()
                .unwrap_or("")
                .contains("production_complete")
        {
            assert!(r.ticks_advanced <= 200);
            fired = true;
            break;
        }
        if r.done {
            break;
        }
    }
    assert!(fired, "production_complete never fired after a unit was built");

    let _ = std::fs::remove_file(&path);
}
