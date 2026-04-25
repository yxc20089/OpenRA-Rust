//! `step()` with a `MoveUnits` command should advance unit positions.

use openra_train::{Command, Env};
use std::path::PathBuf;

fn scenario_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("RUSH_HOUR_SCENARIO") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let p = PathBuf::from(home)
            .join("Projects/OpenRA-RL-Training/scenarios/discovery/rush-hour.yaml");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

#[test]
fn move_command_advances_unit_positions() {
    let path = match scenario_path() {
        Some(p) => p,
        None => {
            eprintln!("skipping — rush-hour scenario yaml not found");
            return;
        }
    };

    let mut env = Env::new(path.to_str().unwrap(), 7).expect("Env::new");
    let initial = env.reset();

    // Pick a single own unit and move it to a far-away cell well
    // inside the playable bounds. We deliberately pick a target far
    // from the current cluster (rush-hour spawn 0 is around (4-7,
    // 5-13)), so any movement at all will register as a position
    // change.
    let (pick_id, start_pos) = initial
        .unit_positions
        .first()
        .map(|(id, p)| (id.clone(), (p.cell_x, p.cell_y)))
        .expect("expected ≥ 1 own unit at reset");

    // Target: same row but ~30 cells east.
    let target_x = start_pos.0 + 30;
    let target_y = start_pos.1;

    let cmd = Command::MoveUnits {
        unit_ids: vec![pick_id.clone()],
        target_x,
        target_y,
    };

    // Step the env many times so the actor has had a chance to make
    // visible progress (movement is sub-cell, accumulates over many
    // ticks — each step advances 30 ticks by default).
    let mut last_pos = start_pos;
    for _ in 0..15 {
        let result = env.step(&[cmd.clone()]);
        // No invalid id warnings expected.
        assert!(
            result.warnings.is_empty(),
            "unexpected warnings: {:?}",
            result.warnings
        );
        if let Some((_, p)) = result
            .obs
            .unit_positions
            .iter()
            .find(|(id, _)| id == &pick_id)
        {
            last_pos = (p.cell_x, p.cell_y);
            if last_pos.0 != start_pos.0 || last_pos.1 != start_pos.1 {
                break;
            }
        }
    }

    assert_ne!(
        last_pos, start_pos,
        "unit {pick_id} should have moved from {start_pos:?} after a Move order"
    );

    // Should be advancing toward the target (eastward).
    assert!(
        last_pos.0 > start_pos.0,
        "expected eastward progress: started {start_pos:?}, ended {last_pos:?}"
    );
}

#[test]
fn invalid_unit_id_emits_warning() {
    let path = match scenario_path() {
        Some(p) => p,
        None => {
            eprintln!("skipping — rush-hour scenario yaml not found");
            return;
        }
    };

    let mut env = Env::new(path.to_str().unwrap(), 7).expect("Env::new");
    env.reset();

    // 99999999 is far above any auto-allocated id.
    let cmd = Command::MoveUnits {
        unit_ids: vec!["99999999".into()],
        target_x: 30,
        target_y: 30,
    };
    let result = env.step(&[cmd]);
    assert!(
        !result.warnings.is_empty(),
        "expected an ownership warning for an unknown unit id"
    );
}
