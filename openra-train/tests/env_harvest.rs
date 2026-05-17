//! S0 harvest-income end-to-end: a scenario with a `mine` (ore source),
//! a `proc` refinery and a `harv` harvester must convert ore into cash —
//! and stay correct when the harvest order is re-issued every tick
//! (agents/models re-send commands; `order_harvest` is idempotent).

use openra_train::{Command, Env};
use std::io::Write;
use std::path::PathBuf;

const SCENARIO: &str = r#"name: HarvestIncome
base_map: ../maps/rush-hour-arena.oramap
spawn_mcvs: false
starting_cash: 200
agent:
  faction: allies
enemy:
  faction: soviet
actors:
- type: proc
  owner: agent
  position:
  - 12
  - 18
- type: harv
  owner: agent
  position:
  - 14
  - 18
- type: mine
  owner: neutral
  position:
  - 22
  - 18
- type: jeep
  owner: agent
  position:
  - 8
  - 16
termination:
  max_ticks: 40000
"#;

fn scenario() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let dir = PathBuf::from(&home).join("Projects/OpenRA-RL-Training/scenarios/discovery");
    if !dir.join("rush-hour.yaml").exists() {
        return None;
    }
    let p = dir.join("_harvest_income_test.yaml");
    std::fs::File::create(&p).ok()?.write_all(SCENARIO.as_bytes()).ok()?;
    Some(p)
}

fn harv_id(env: &mut Env) -> Vec<String> {
    let o = env.reset();
    o.unit_positions
        .iter()
        .filter(|(_, p)| (p.cell_x, p.cell_y) == (14, 18))
        .map(|(id, _)| id.clone())
        .collect()
}

#[test]
fn harvest_converts_ore_to_cash_and_is_idempotent() {
    let Some(path) = scenario() else {
        eprintln!("skip: RL-Training scenarios tree not present");
        return;
    };
    // Re-issue harvest EVERY tick (the realistic agent pattern). With a
    // non-idempotent order_harvest this resets carried ore and earns 0.
    let mut env = Env::new(path.to_str().unwrap(), 7).expect("Env::new");
    let ids = harv_id(&mut env);
    assert!(!ids.is_empty(), "harvester must load from YAML");
    assert_eq!(env.last_observation().economy.cash, 200);

    let mut peak = 200;
    for _ in 0..200 {
        let r = env.step(&[Command::Harvest {
            unit_ids: ids.clone(),
            target_x: 22,
            target_y: 18,
        }]);
        peak = peak.max(r.obs.economy.cash);
    }
    assert!(
        peak > 200,
        "harvester never converted ore to cash (peak {peak}) — ore not \
         seeded, or order_harvest not idempotent under re-issue"
    );

    let _ = std::fs::remove_file(&path);
}
