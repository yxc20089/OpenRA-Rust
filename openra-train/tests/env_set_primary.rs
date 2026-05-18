//! S7 — SET_PRIMARY command (C# `PrimaryBuilding` trait parity,
//! pragmatic subset).
//!
//! Scope asserted here (honest):
//!  * With two same-type production buildings owned by the agent,
//!    SET_PRIMARY on one makes newly produced units spawn next to
//!    THAT building (not the first one found / the other one).
//!  * The `is_primary` flag is surfaced on `own_buildings` and tracks
//!    the SET_PRIMARY command; exactly one building of a given type is
//!    primary at a time (setting a new one clears the old, like C#
//!    `PrimaryBuilding.SetPrimaryProducer`).
//!  * SET_PRIMARY validates ownership (non-owned id warns) and never
//!    terminates the episode.
//!
//! NOT asserted (documented gap): C# also routes the production EXIT
//! cell / rally inheritance through the primary; we assert the
//! spawn-building preference and the observation flag only.

use openra_train::{Command, Env};
use std::io::Write;
use std::path::PathBuf;

const SCENARIO: &str = r#"name: SetPrimary
base_map: ../maps/rush-hour-arena.oramap
spawn_mcvs: false
starting_cash: 6000
agent:
  faction: allies
enemy:
  faction: soviet
actors:
- type: barr
  owner: agent
  position:
  - 8
  - 8
- type: barr
  owner: agent
  position:
  - 50
  - 30
- type: powr
  owner: agent
  position:
  - 12
  - 8
- type: e1
  owner: enemy
  position:
  - 60
  - 33
  stance: 0
termination:
  max_ticks: 20000
"#;

fn write_scenario(name: &str) -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let dir = PathBuf::from(&home).join("Projects/OpenRA-RL-Training/scenarios/discovery");
    if !dir.join("rush-hour.yaml").exists() {
        return None;
    }
    let p = dir.join(name);
    std::fs::File::create(&p).ok()?.write_all(SCENARIO.as_bytes()).ok()?;
    Some(p)
}

fn building_id_at(env: &Env, cell: (i32, i32)) -> u32 {
    env.world()
        .expect("world")
        .snapshot()
        .actors
        .iter()
        .find(|a| (a.x, a.y) == cell)
        .map(|a| a.id)
        .unwrap_or_else(|| panic!("no building at {cell:?}"))
}

fn cheb(a: (i32, i32), b: (i32, i32)) -> i32 {
    (a.0 - b.0).abs().max((a.1 - b.1).abs())
}

#[test]
fn set_primary_routes_production_spawn_and_sets_flag() {
    let Some(path) = write_scenario("_set_primary.yaml") else {
        eprintln!("skip: RL-Training scenarios not present");
        return;
    };
    let mut env = Env::new(path.to_str().unwrap(), 7).unwrap();
    let o = env.reset();

    // Both barracks present in own_buildings, none primary yet.
    let barr_count = o
        .own_buildings
        .iter()
        .filter(|b| b.building_type == "barr")
        .count();
    assert_eq!(barr_count, 2, "expected two barracks: {:?}",
        o.own_buildings.iter().map(|b| &b.building_type).collect::<Vec<_>>());
    assert!(
        o.own_buildings.iter().all(|b| !b.is_primary),
        "no building should be primary before SET_PRIMARY"
    );

    let far_barr = building_id_at(&env, (50, 30));

    // Designate the far barracks primary.
    let rp = env.step(&[Command::SetPrimary {
        unit_ids: vec![far_barr.to_string()],
    }]);
    assert!(rp.warnings.is_empty(), "set_primary warned: {:?}", rp.warnings);
    assert!(!rp.done, "set_primary must not terminate the episode");

    // The flag is reflected on exactly the far barracks.
    let prim: Vec<u32> = rp
        .obs
        .own_buildings
        .iter()
        .filter(|b| b.is_primary)
        .map(|b| b.id.parse().unwrap())
        .collect();
    assert_eq!(prim, vec![far_barr], "exactly the far barracks is primary");

    // Build an E1 and let it spawn; it must appear next to the FAR
    // (primary) barracks, not the near one at (8,8).
    let units_before = rp.obs.unit_positions.len();
    env.step(&[Command::Build { item: "e1".into() }]);
    let mut spawn_pos = None;
    for _ in 0..80 {
        let r = env.step(&[Command::Observe]);
        if r.obs.unit_positions.len() > units_before {
            // newest unit = the one not at an original cell; just take
            // the unit closest to either barracks for the assertion.
            spawn_pos = r
                .obs
                .unit_positions
                .iter()
                .map(|(_, p)| (p.cell_x, p.cell_y))
                .min_by_key(|&c| cheb(c, (50, 30)).min(cheb(c, (8, 8))));
            break;
        }
    }
    let sp = spawn_pos.expect("production never spawned a unit");
    let d_far = cheb(sp, (50, 30));
    let d_near = cheb(sp, (8, 8));
    assert!(
        d_far < d_near,
        "unit must spawn at the PRIMARY (far) barracks: spawn {sp:?}, \
         d_far={d_far}, d_near={d_near}"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn set_primary_switches_and_validates() {
    let Some(path) = write_scenario("_set_primary_sw.yaml") else {
        eprintln!("skip: RL-Training scenarios not present");
        return;
    };
    let mut env = Env::new(path.to_str().unwrap(), 7).unwrap();
    env.reset();
    let near = building_id_at(&env, (8, 8));
    let far = building_id_at(&env, (50, 30));

    env.step(&[Command::SetPrimary { unit_ids: vec![near.to_string()] }]);
    let r = env.step(&[Command::SetPrimary { unit_ids: vec![far.to_string()] }]);
    // Switching primary clears the previous one (one primary per type).
    let prim: Vec<u32> = r
        .obs
        .own_buildings
        .iter()
        .filter(|b| b.is_primary)
        .map(|b| b.id.parse().unwrap())
        .collect();
    assert_eq!(prim, vec![far], "primary must switch, not accumulate");

    // Non-owned id warns.
    let rb = env.step(&[Command::SetPrimary { unit_ids: vec!["999999".into()] }]);
    assert!(
        rb.warnings.iter().any(|w| w.contains("not owned")),
        "ownership validation must apply: {:?}",
        rb.warnings
    );

    let _ = std::fs::remove_file(&path);
}
