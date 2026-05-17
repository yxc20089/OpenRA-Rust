//! F3: every new Command variant is plumbed through `build_orders`
//! into the sim. Asserts observable behaviour where the rush-hour
//! scenario allows it (Stop/AttackMove) and clean acceptance
//! (no spurious warnings, no panic) for the economy/structure
//! commands, plus that ownership validation still rejects bad ids.

use openra_train::{Command, Env};
use std::path::PathBuf;

fn scenario_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("RUSH_HOUR_SCENARIO") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    let home = std::env::var("HOME").ok()?;
    let p = PathBuf::from(home)
        .join("Projects/OpenRA-RL-Training/scenarios/discovery/rush-hour.yaml");
    p.exists().then_some(p)
}

fn first_unit(env: &mut Env) -> (String, (i32, i32)) {
    let o = env.reset();
    let (id, p) = o.unit_positions.first().expect("≥1 own unit");
    (id.clone(), (p.cell_x, p.cell_y))
}

/// A reachable in-bounds target ~25 cells toward map interior from
/// `start` (rush-hour is ~128×40; edge units must move inward).
fn inward_target(start: (i32, i32)) -> (i32, i32) {
    let tx = if start.0 > 64 { start.0 - 25 } else { start.0 + 25 };
    (tx, start.1.clamp(2, 37))
}

#[test]
fn attack_move_advances_like_move() {
    let Some(path) = scenario_path() else {
        eprintln!("skip: rush-hour yaml not found");
        return;
    };
    let mut env = Env::new(path.to_str().unwrap(), 7).unwrap();
    let (id, start) = first_unit(&mut env);
    let tgt = inward_target(start);
    let cmd = Command::AttackMove {
        unit_ids: vec![id.clone()],
        target_x: tgt.0,
        target_y: tgt.1,
    };
    let mut last = start;
    for _ in 0..15 {
        let r = env.step(&[cmd.clone()]);
        assert!(r.warnings.is_empty(), "warnings: {:?}", r.warnings);
        if let Some((_, p)) = r.obs.unit_positions.iter().find(|(u, _)| u == &id) {
            last = (p.cell_x, p.cell_y);
        }
    }
    let d0 = (tgt.0 - start.0).abs() + (tgt.1 - start.1).abs();
    let d1 = (tgt.0 - last.0).abs() + (tgt.1 - last.1).abs();
    assert!(d1 < d0, "attack_move did not advance {start:?}->{last:?} (tgt {tgt:?})");
}

#[test]
fn stop_halts_a_moving_unit() {
    let Some(path) = scenario_path() else { return };
    let mut env = Env::new(path.to_str().unwrap(), 7).unwrap();
    let (id, start) = first_unit(&mut env);
    // Send it moving, let it travel.
    let tgt = inward_target(start);
    let mv = Command::MoveUnits { unit_ids: vec![id.clone()], target_x: tgt.0, target_y: tgt.1 };
    for _ in 0..4 {
        env.step(&[mv.clone()]);
    }
    let pos_after_move = {
        let o = env.last_observation();
        let (_, p) = o.unit_positions.iter().find(|(u, _)| u == &id).unwrap();
        (p.cell_x, p.cell_y)
    };
    // Stop, then idle several steps — position must not keep advancing.
    env.step(&[Command::Stop { unit_ids: vec![id.clone()] }]);
    let mut prev = pos_after_move;
    for _ in 0..6 {
        let r = env.step(&[Command::Observe]);
        let (_, p) = r.obs.unit_positions.iter().find(|(u, _)| u == &id).unwrap();
        let now = (p.cell_x, p.cell_y);
        prev = now;
    }
    let drift = (prev.0 - pos_after_move.0).abs() + (prev.1 - pos_after_move.1).abs();
    assert!(drift <= 1, "unit kept moving after Stop: {pos_after_move:?}->{prev:?}");
}

#[test]
fn economy_and_structure_commands_are_accepted_cleanly() {
    let Some(path) = scenario_path() else { return };
    let mut env = Env::new(path.to_str().unwrap(), 7).unwrap();
    let (id, _) = first_unit(&mut env);
    let cmds = vec![
        Command::Build { item: "E1".into() },
        Command::CancelProduction { item: "E1".into() },
        Command::PlaceBuilding { item: "POWR".into(), target_x: 10, target_y: 10 },
        Command::Harvest { unit_ids: vec![id.clone()], target_x: 12, target_y: 12 },
        Command::Deploy { unit_ids: vec![id.clone()] },
        Command::Sell { unit_ids: vec![id.clone()] },
        Command::Repair { unit_ids: vec![id.clone()] },
        Command::PowerDown { unit_ids: vec![id.clone()] },
        Command::SetRallyPoint { unit_ids: vec![id.clone()], target_x: 9, target_y: 9 },
    ];
    // Must not panic; valid agent-owned ids must not raise ownership
    // warnings (Build/Cancel/Place carry no unit id at all).
    let r = env.step(&cmds);
    assert!(
        !r.warnings.iter().any(|w| w.contains("not owned") || w.contains("invalid unit_id")),
        "unexpected warnings: {:?}",
        r.warnings
    );
    assert!(r.obs.game_tick > 0);
}

#[test]
fn economy_observation_is_surfaced_and_deterministic() {
    let Some(path) = scenario_path() else { return };
    let mut a = Env::new(path.to_str().unwrap(), 7).unwrap();
    let mut b = Env::new(path.to_str().unwrap(), 7).unwrap();
    let oa = a.reset();
    // S9 fields exist and are well-formed.
    assert!(oa.economy.cash >= 0, "cash {}", oa.economy.cash);
    assert!(oa.economy.harvesters >= 0);
    assert!(oa.economy.power_provided >= 0 && oa.economy.power_drained >= 0);
    // production is a list (possibly empty in rush-hour).
    let _ = oa.production.len();
    let _ = oa.own_buildings.len();

    // Same seed ⇒ identical economy after identical stepping.
    b.reset();
    for _ in 0..5 {
        a.step(&[Command::Observe]);
        b.step(&[Command::Observe]);
    }
    let ea = a.last_observation();
    let eb = b.last_observation();
    assert_eq!(ea.economy, eb.economy, "economy must be deterministic per seed");
}

#[test]
fn ownership_validation_still_rejects_bad_ids() {
    let Some(path) = scenario_path() else { return };
    let mut env = Env::new(path.to_str().unwrap(), 7).unwrap();
    env.reset();
    let r = env.step(&[Command::Stop { unit_ids: vec!["999999".into()] }]);
    assert!(
        r.warnings.iter().any(|w| w.contains("not owned")),
        "expected ownership warning, got {:?}",
        r.warnings
    );
}
