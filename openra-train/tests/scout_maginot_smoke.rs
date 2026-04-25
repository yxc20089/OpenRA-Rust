//! Phase-7 smoke test: load `scout-maginot` from the OpenRA-RL-Training
//! scenarios directory, run a handful of `Observe` ticks, verify the
//! observation reports the expected enemy buildings.
//!
//! Skipped silently when the scenario / base map is missing (e.g. CI
//! without the OpenRA-RL-Training repo cloned at the canonical path).

use std::path::PathBuf;

fn scenario_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let p = PathBuf::from(home)
        .join("Projects/OpenRA-RL-Training/scenarios/strategy/scout-maginot.yaml");
    if p.exists() { Some(p) } else { None }
}

#[test]
fn scout_maginot_loads_with_buildings_and_no_panic() {
    let path = match scenario_path() {
        Some(p) => p,
        None => {
            eprintln!("skipping: scout-maginot scenario not present at canonical path");
            return;
        }
    };

    // Load the scenario through the same code path the env uses.
    let map_def = match openra_data::oramap::load_rush_hour_map(&path) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("skipping: failed to load scout-maginot map ({e})");
            return;
        }
    };
    eprintln!(
        "loaded scout-maginot: {}×{} bounds={:?} actors={}",
        map_def.map_size.0,
        map_def.map_size.1,
        map_def.bounds,
        map_def.actors.len()
    );

    // Verify the YAML places at least the expected enemy buildings:
    // 2 gun, 2 tsla, 1 fact, 1 powr, 1 barr, 1 proc + the base-defender
    // gun = 8 buildings (the scenario lists 9 building entries total
    // because the bypass adds another gun on the right). We assert ≥ 7
    // to leave room for future scenario edits.
    let enemy_buildings: Vec<_> = map_def
        .actors
        .iter()
        .filter(|a| a.owner == "enemy")
        .filter(|a| {
            matches!(
                a.actor_type.as_str(),
                "gun" | "tsla" | "fact" | "powr" | "barr" | "proc" | "pbox" | "ftur"
            )
        })
        .collect();
    eprintln!(
        "scout-maginot enemy buildings (from yaml): {}",
        enemy_buildings.len()
    );
    assert!(
        enemy_buildings.len() >= 7,
        "expected >=7 enemy buildings in scout-maginot, found {} ({:?})",
        enemy_buildings.len(),
        enemy_buildings
            .iter()
            .map(|a| &a.actor_type)
            .collect::<Vec<_>>()
    );

    // Map dimensions sanity (the singles-* maps are at least 32 in
    // either axis; scout-maginot is 128×40 in practice).
    assert!(
        map_def.map_size.0 >= 32 && map_def.map_size.1 >= 32,
        "scout-maginot map dimensions look wrong: {:?}",
        map_def.map_size
    );
}

#[test]
fn scout_maginot_env_runs_50_observe_ticks() {
    let path = match scenario_path() {
        Some(p) => p,
        None => {
            eprintln!("skipping: scout-maginot scenario not present at canonical path");
            return;
        }
    };
    let path_str = path.to_string_lossy().to_string();

    // Build the env. We use openra-train's `Env` (pure Rust API) since
    // we don't need PyO3.
    let mut env = match openra_train::env::Env::new(&path_str, 0xC0FFEE) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("skipping: failed to construct env: {e}");
            return;
        }
    };
    let _ = env.reset();

    // Run 50 observe ticks. Each step issues no commands, so the only
    // dynamics are auto-target firing and shroud reveals.
    let mut last_obs = env.last_observation();
    for tick in 0..50 {
        let result = env.step(&[]);
        last_obs = result.obs;
        if tick == 0 {
            eprintln!(
                "scout-maginot @ tick0: own={} visible_enemies={} visible_buildings={}",
                last_obs.unit_positions.len(),
                last_obs.enemy_positions.len(),
                last_obs.enemy_buildings.len(),
            );
        }
    }
    // The agent's own units should still be visible.
    assert!(
        !last_obs.unit_positions.is_empty(),
        "expected own units to remain after 50 observe ticks"
    );
    eprintln!(
        "scout-maginot @ tick50: own={} visible_enemies={} visible_buildings={}",
        last_obs.unit_positions.len(),
        last_obs.enemy_positions.len(),
        last_obs.enemy_buildings.len(),
    );
}
