//! Task #10 — FAITHFUL HoldFire (C# AttackBase / AutoTarget parity).
//!
//! Scope asserted here (honest):
//!  * A unit on HoldFire stance (0) that has an ADJACENT enemy deals
//!    ZERO damage to that enemy over many steps — the auto-acquired
//!    Attack activity is ABANDONED every tick, not merely gated at
//!    acquisition. This is the real C# behaviour: HoldFire units do
//!    not initiate or opportunistically engage.
//!  * A CONTROL unit (default AttackAnything stance) with the same
//!    adjacent enemy DOES deal damage — proves the test can observe
//!    combat and the HoldFire assertion is not vacuous.
//!  * An EXPLICIT `attack_unit` order issued by the agent STILL
//!    attacks even while the unit is on HoldFire — player/agent intent
//!    overrides stance (only auto/opportunistic engagement is
//!    suppressed). This is the auto_acquired vs order-issued split.
//!
//! NOT yet asserted (documented gap): faithful ReturnFire(1) — a
//! HoldFire/ReturnFire unit firing back only when itself attacked — is
//! NOT exercised here. ReturnFire is implemented best-effort in the
//! engine but a dedicated retaliation integ test is future work; this
//! file deliberately does not imply coverage it lacks.

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
    std::fs::File::create(&p).ok()?.write_all(body.as_bytes()).ok()?;
    Some(p)
}

// Agent e1 at (20,20); enemy e1 directly adjacent at (21,20)
// (Chebyshev distance 1 — well inside rifle range).
fn scenario_body() -> String {
    r#"name: HoldFire
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
  - 21
  - 20
  stance: 0
termination:
  max_ticks: 40000
"#
    .to_string()
}

fn agent_uid(env: &mut Env) -> String {
    env.reset()
        .unit_positions
        .iter()
        .find(|(_, p)| (p.cell_x, p.cell_y) == (20, 20))
        .map(|(id, _)| id.clone())
        .expect("agent e1 at (20,20)")
}

// Read enemy hp from the WORLD snapshot, not the shroud-filtered
// observation: the agent unit does not scout, so the enemy leaves
// agent vision after the first tick. We want ground-truth "did the
// agent deal damage", which must be shroud-independent.
fn enemy_uid(env: &Env) -> u32 {
    env.world()
        .expect("world")
        .snapshot()
        .actors
        .iter()
        .find(|a| (a.x, a.y) == (21, 20))
        .map(|a| a.id)
        .expect("enemy e1 at (21,20)")
}

fn enemy_hp(env: &Env, id: u32) -> i32 {
    env.world()
        .expect("world")
        .snapshot()
        .actors
        .iter()
        .find(|a| a.id == id)
        .map(|a| a.hp)
        .unwrap_or(0)
}

const STEPS: usize = 200;

#[test]
fn holdfire_unit_deals_no_damage_to_adjacent_enemy() {
    let Some(path) = write_scenario("_holdfire_test.yaml", &scenario_body()) else {
        eprintln!("skip: RL-Training scenarios not present");
        return;
    };
    let mut env = Env::new(path.to_str().unwrap(), 7).unwrap();
    let uid = agent_uid(&mut env);
    let eid = enemy_uid(&env);

    // Put the agent unit on HoldFire BEFORE any combat-capable step.
    let r = env.step(&[Command::SetStance {
        unit_ids: vec![uid.clone()],
        stance: 0,
    }]);
    assert!(r.warnings.is_empty(), "set_stance warned: {:?}", r.warnings);

    let hp0 = enemy_hp(&env, eid);
    assert!(hp0 > 0, "enemy must start alive (hp0={hp0})");

    for _ in 0..STEPS {
        env.step(&[Command::Observe]);
    }
    let hp1 = enemy_hp(&env, eid);

    assert_eq!(
        hp1, hp0,
        "HoldFire unit must deal NO damage: enemy hp {hp0} -> {hp1}"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn control_without_holdfire_does_damage_adjacent_enemy() {
    let Some(path) = write_scenario("_holdfire_ctrl.yaml", &scenario_body()) else {
        eprintln!("skip: RL-Training scenarios not present");
        return;
    };
    let mut env = Env::new(path.to_str().unwrap(), 7).unwrap();
    let _uid = agent_uid(&mut env);
    let eid = enemy_uid(&env);

    // No SetStance — default AttackAnything: auto-engage the adjacent enemy.
    let hp0 = enemy_hp(&env, eid);
    assert!(hp0 > 0, "enemy must start alive");

    for _ in 0..STEPS {
        env.step(&[Command::Observe]);
        if enemy_hp(&env, eid) < hp0 {
            break;
        }
    }
    let hp1 = enemy_hp(&env, eid);
    assert!(
        hp1 < hp0,
        "control (no HoldFire) MUST damage the adjacent enemy: {hp0} -> {hp1}"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn explicit_attack_overrides_holdfire() {
    let Some(path) = write_scenario("_holdfire_explicit.yaml", &scenario_body()) else {
        eprintln!("skip: RL-Training scenarios not present");
        return;
    };
    let mut env = Env::new(path.to_str().unwrap(), 7).unwrap();
    let uid = agent_uid(&mut env);
    let eid = enemy_uid(&env);

    env.step(&[Command::SetStance {
        unit_ids: vec![uid.clone()],
        stance: 0,
    }]);
    let hp0 = enemy_hp(&env, eid);
    assert!(hp0 > 0, "enemy must start alive");

    // Explicit agent order: attack despite HoldFire (player intent wins).
    env.step(&[Command::AttackUnit {
        unit_ids: vec![uid.clone()],
        target_id: eid.to_string(),
    }]);
    for _ in 0..STEPS {
        env.step(&[Command::Observe]);
        if enemy_hp(&env, eid) < hp0 {
            break;
        }
    }
    let hp1 = enemy_hp(&env, eid);
    assert!(
        hp1 < hp0,
        "explicit attack_unit MUST override HoldFire: {hp0} -> {hp1}"
    );

    let _ = std::fs::remove_file(&path);
}
