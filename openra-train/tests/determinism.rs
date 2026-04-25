//! Wider determinism check (per Phase 5 spec): two parallel resets +
//! the same command sequence yield byte-identical observations
//! field-by-field, not just the same hash.

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
fn parallel_envs_produce_identical_observations() {
    let path = match scenario_path() {
        Some(p) => p,
        None => {
            eprintln!("skipping — rush-hour scenario yaml not found");
            return;
        }
    };
    let p = path.to_str().unwrap();

    let mut a = Env::new(p, 999).expect("env A");
    let mut b = Env::new(p, 999).expect("env B");

    let init_a = a.reset();
    let init_b = b.reset();
    assert_eq!(init_a.unit_positions.len(), init_b.unit_positions.len());
    for (ua, ub) in init_a
        .unit_positions
        .iter()
        .zip(init_b.unit_positions.iter())
    {
        assert_eq!(ua.0, ub.0);
        assert_eq!(ua.1.cell_x, ub.1.cell_x);
        assert_eq!(ua.1.cell_y, ub.1.cell_y);
    }
    assert_eq!(init_a.deterministic_hash(), init_b.deterministic_hash());

    let unit_ids: Vec<String> = init_a
        .unit_positions
        .iter()
        .map(|(id, _)| id.clone())
        .collect();
    let cmd = Command::MoveUnits {
        unit_ids,
        target_x: 60,
        target_y: 20,
    };

    for step in 0..6 {
        let ra = a.step(&[cmd.clone()]);
        let rb = b.step(&[cmd.clone()]);
        assert_eq!(
            ra.obs.deterministic_hash(),
            rb.obs.deterministic_hash(),
            "obs hash mismatch at step {step}"
        );
        assert_eq!(ra.obs.unit_positions.len(), rb.obs.unit_positions.len());
        for ((id_a, pa), (id_b, pb)) in ra
            .obs
            .unit_positions
            .iter()
            .zip(rb.obs.unit_positions.iter())
        {
            assert_eq!(id_a, id_b, "unit id ordering at step {step}");
            assert_eq!(
                (pa.cell_x, pa.cell_y),
                (pb.cell_x, pb.cell_y),
                "unit {id_a} drift at step {step}"
            );
        }
    }
}
