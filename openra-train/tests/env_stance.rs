//! S7: PATROL (C# parity: defined-but-unimplemented → accepted as a
//! clean no-op) and SET_STANCE command plumbing.
//!
//! NOTE on set_stance: the auto-engage gate (HoldFire excludes a unit
//! from the idle auto-engage scan) is implemented and verified via the
//! engine-internal trace, but full C# HoldFire fidelity (no weapon
//! fire at all) also requires gating the separate weapon-fire / bot-
//! issued attack path — tracked in task #10. We therefore assert only
//! what is faithfully true today: both commands are accepted, plumbed,
//! and never produce a spurious warning or terminate the episode.

use openra_train::{Command, Env};
use std::io::Write;
use std::path::PathBuf;

const SCENARIO: &str = r#"name: Stance
base_map: ../maps/rush-hour-arena.oramap
spawn_mcvs: false
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
- type: e1
  owner: enemy
  position:
  - 60
  - 30
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
    let p = dir.join("_stance_test.yaml");
    std::fs::File::create(&p).ok()?.write_all(SCENARIO.as_bytes()).ok()?;
    Some(p)
}

fn agent_uid(env: &mut Env) -> String {
    env.reset()
        .unit_positions
        .iter()
        .find(|(_, p)| (p.cell_x, p.cell_y) == (20, 20))
        .map(|(id, _)| id.clone())
        .expect("agent e1 at (20,20)")
}

#[test]
fn set_stance_and_patrol_are_accepted_and_plumbed() {
    let Some(path) = scenario() else {
        eprintln!("skip: RL-Training scenarios not present");
        return;
    };
    let mut env = Env::new(path.to_str().unwrap(), 7).unwrap();
    let uid = agent_uid(&mut env);

    // SET_STANCE — accepted for an owned unit, no warning, not terminal.
    for s in [0, 1, 2, 3] {
        let r = env.step(&[Command::SetStance {
            unit_ids: vec![uid.clone()],
            stance: s,
        }]);
        assert!(r.warnings.is_empty(), "set_stance({s}) warned: {:?}", r.warnings);
        assert!(!r.done, "set_stance must not terminate the episode");
    }

    // PATROL — C# parity no-op: accepted, no warning, no termination,
    // and does NOT divert the unit (position unchanged after issuing
    // patrol then idling a few steps).
    let before = {
        let o = env.last_observation();
        o.unit_positions
            .iter()
            .find(|(i, _)| i == &uid)
            .map(|(_, p)| (p.cell_x, p.cell_y))
            .unwrap()
    };
    let rp = env.step(&[Command::Patrol { unit_ids: vec![uid.clone()] }]);
    assert!(rp.warnings.is_empty() && !rp.done, "patrol must be a clean no-op");
    for _ in 0..3 {
        env.step(&[Command::Observe]);
    }
    let after = {
        let o = env.last_observation();
        o.unit_positions
            .iter()
            .find(|(i, _)| i == &uid)
            .map(|(_, p)| (p.cell_x, p.cell_y))
            .unwrap()
    };
    assert_eq!(before, after, "PATROL must not move the unit (no-op)");

    // Bad unit id still warns (validation intact for the new commands).
    let rb = env.step(&[Command::SetStance {
        unit_ids: vec!["999999".into()],
        stance: 0,
    }]);
    assert!(
        rb.warnings.iter().any(|w| w.contains("not owned")),
        "ownership validation must still apply: {:?}",
        rb.warnings
    );

    let _ = std::fs::remove_file(&path);
}
