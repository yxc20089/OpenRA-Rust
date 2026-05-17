//! S1/S3/S5 end-to-end: a contributed scenario with an agent producer
//! + starting cash can build a unit (cost paid, tech prereq satisfied
//! by the owned barracks, unit spawns) — all driven through the public
//! Command/Observation surface, on the Rust engine.

use openra_train::{Command, Env};
use std::io::Write;
use std::path::PathBuf;

const SCENARIO: &str = r#"name: Econ E2E
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
  - 110
  - 35
  stance: 2
termination:
  max_ticks: 20000
"#;

fn write_scenario() -> Option<PathBuf> {
    // The loader resolves `base_map` relative to the RL-Training
    // scenarios dir; only run when that vendored tree is present.
    let home = std::env::var("HOME").ok()?;
    let rh = PathBuf::from(&home)
        .join("Projects/OpenRA-RL-Training/scenarios/discovery/rush-hour.yaml");
    if !rh.exists() {
        return None;
    }
    let dir = PathBuf::from(&home).join("Projects/OpenRA-RL-Training/scenarios/discovery");
    let p = dir.join("_econ_e2e_test.yaml");
    let mut f = std::fs::File::create(&p).ok()?;
    f.write_all(SCENARIO.as_bytes()).ok()?;
    Some(p)
}

#[test]
fn scenario_starting_cash_and_production_spawns_a_unit() {
    let Some(path) = write_scenario() else {
        eprintln!("skip: RL-Training scenarios tree not present");
        return;
    };
    let mut env = Env::new(path.to_str().unwrap(), 7).expect("Env::new");
    let o = env.reset();

    // Designed economy constraint honoured (was hardcoded 0).
    assert_eq!(o.economy.cash, 4000, "scenario starting_cash must apply");
    // Agent buildings loaded from the contributed scenario.
    let btypes: Vec<&str> =
        o.own_buildings.iter().map(|b| b.building_type.as_str()).collect();
    assert!(btypes.contains(&"barr"), "barracks not loaded: {btypes:?}");
    let units_at_reset = o.unit_positions.len();

    // Queue an E1 (prereq ~barracks satisfied by the owned barr). The
    // queue may drain within this step (E1 is cheap/fast), so the real
    // contract is the *outcome*: a new unit spawns and cash is billed.
    let r0 = env.step(&[Command::Build { item: "e1".into() }]);
    let mut spawned = r0.obs.unit_positions.len() > units_at_reset;
    let mut final_cash = r0.obs.economy.cash;
    for _ in 0..60 {
        if spawned {
            break;
        }
        let r = env.step(&[Command::Observe]);
        final_cash = r.obs.economy.cash;
        spawned = r.obs.unit_positions.len() > units_at_reset;
    }
    assert!(spawned, "production never spawned a unit");
    assert!(
        final_cash < 4000,
        "cash must be debited by production cost, still {final_cash}"
    );

    let _ = std::fs::remove_file(&path);
}
