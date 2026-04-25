//! `reset()` integration test for `OpenRAEnv`.
//!
//! Asserts that loading the rush-hour scenario at spawn 0 yields:
//!   * the scenario's own units (≥ 5: 3 e1 + 2 dog at minimum, plus
//!     vehicles depending on map spec)
//!   * 0 visible enemies (the map is large enough that the closest
//!     enemy squad is well outside the agents' sight radii at spawn)
//!
//! Skipped with a printed reason if the rush-hour scenario YAML is
//! not present on the dev box.

use openra_train::Env;
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
fn reset_yields_own_units_and_no_visible_enemies() {
    let path = match scenario_path() {
        Some(p) => p,
        None => {
            eprintln!("skipping — rush-hour scenario yaml not found on this box");
            return;
        }
    };

    let mut env = Env::new(path.to_str().unwrap(), 42).expect("Env::new");
    let obs = env.reset();

    // 5 infantry minimum (3 e1 + 2 dog at spawn 0). Vehicles also
    // count as own units; rush-hour's spawn 0 actor list has 8
    // distinct entries with count expansion → 12 own units total.
    assert!(
        obs.unit_positions.len() >= 5,
        "expected ≥ 5 own units, got {} (entries={:?})",
        obs.unit_positions.len(),
        obs.unit_positions
    );

    // unit_hp parallels unit_positions.
    assert_eq!(
        obs.unit_positions.len(),
        obs.unit_hp.len(),
        "unit_positions / unit_hp length mismatch"
    );

    // All HP fractions should be at full health (1.0) on reset.
    for (id, hp) in &obs.unit_hp {
        assert!(
            (*hp - 1.0).abs() < 0.01,
            "expected fresh unit {id} at full HP, got {hp}"
        );
    }

    // Enemies are spawned far across the map; agent shroud at sight
    // range 4-6 should not see them on tick 0.
    assert_eq!(
        obs.enemy_positions.len(),
        0,
        "enemies should be hidden by fog on reset; got {:?}",
        obs.enemy_positions
    );
    assert_eq!(obs.enemy_hp.len(), 0);

    // Tick + counters initialised.
    assert_eq!(obs.units_killed, 0, "no kills before any step");
    assert!(obs.game_tick >= 0, "game tick non-negative");

    // Some explored area (non-zero, but well under 100% on a 124x36
    // playable region).
    assert!(
        obs.explored_percent > 0.0 && obs.explored_percent < 100.0,
        "explored_percent should be > 0 and < 100, got {}",
        obs.explored_percent
    );
}

#[test]
fn unit_positions_have_cell_x_cell_y_schema() {
    // Schema validation: every unit_positions entry must have an int
    // cell_x and cell_y in playable bounds. This mirrors what
    // `agent_rollout.py::_fork_start_snapshot` reads.
    let path = match scenario_path() {
        Some(p) => p,
        None => {
            eprintln!("skipping — rush-hour scenario yaml not found");
            return;
        }
    };

    let mut env = Env::new(path.to_str().unwrap(), 1234).expect("Env::new");
    let obs = env.reset();

    for (id, pos) in &obs.unit_positions {
        assert!(
            pos.cell_x >= 0 && pos.cell_x < 200,
            "unit {id} cell_x {} out of range",
            pos.cell_x
        );
        assert!(
            pos.cell_y >= 0 && pos.cell_y < 200,
            "unit {id} cell_y {} out of range",
            pos.cell_y
        );
        // ID must round-trip through u32.
        let _: u32 = id.parse().unwrap_or_else(|_| {
            panic!("unit id {id:?} must be a base-10 u32 for Python compatibility")
        });
    }
}
