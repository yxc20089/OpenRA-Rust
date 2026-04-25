//! Phase 4 rush-hour map loader test.
//!
//! Loads the rush-hour scenario and asserts that the actor list, after
//! `count:` expansion and spawn_point filtering, matches the spec:
//!   - 13 enemy infantry
//!   -  5 own infantry (3× e1 + 2× dog at spawn_point=0)
//!
//! The scenario YAML lives at
//!   ~/Projects/OpenRA-RL-Training/scenarios/discovery/rush-hour.yaml
//! and references the base map at `../maps/rush-hour-arena.oramap`.
//!
//! If the scenario file is not present (e.g. on CI without the training
//! repo checked out alongside), the test is skipped with a printed reason
//! rather than failing — it still serves as a runnable check on dev boxes.

use openra_data::oramap;
use std::path::PathBuf;

/// Locate the rush-hour scenario yaml. Tries, in order:
/// 1. `$RUSH_HOUR_SCENARIO` environment override
/// 2. `~/Projects/OpenRA-RL-Training/scenarios/discovery/rush-hour.yaml`
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
fn rush_hour_actor_count_matches_spec() {
    let path = match scenario_path() {
        Some(p) => p,
        None => {
            eprintln!(
                "Skipping rush_hour_actor_count_matches_spec — scenario yaml not found. \
                 Set RUSH_HOUR_SCENARIO=/path/to/rush-hour.yaml to run."
            );
            return;
        }
    };

    let map = oramap::load_rush_hour_map(&path).expect("load rush-hour map");

    // Sanity on the base map metadata (from rush-hour-arena.oramap).
    assert_eq!(map.tileset, "TEMPERAT");
    assert_eq!(map.map_size, (128, 40), "rush-hour map dimensions");
    assert_eq!(map.bounds, (2, 2, 124, 36), "rush-hour playable bounds");
    assert!(!map.tiles.is_empty(), "terrain grid should have been populated");

    // Faction split.
    assert_eq!(map.agent_faction, "allies");
    assert_eq!(map.enemy_faction, "soviet");

    // Spec assertion: 13 enemy infantry + 5 own infantry at spawn 0.
    let enemy_inf = map.enemy_actors().filter(|a| a.is_infantry()).count();
    let own_inf = map.agent_actors().filter(|a| a.is_infantry()).count();

    assert_eq!(
        enemy_inf, 13,
        "expected 13 enemy infantry, got {} (types: {:?})",
        enemy_inf,
        map.enemy_actors().map(|a| &a.actor_type).collect::<Vec<_>>()
    );
    assert_eq!(
        own_inf, 5,
        "expected 5 own infantry at spawn_point=0, got {} (types: {:?})",
        own_inf,
        map.agent_actors().map(|a| &a.actor_type).collect::<Vec<_>>()
    );

    // Of the 5 own infantry: 3 e1 + 2 dog (the dog count comes from two
    // separate entries with count=1 each at spawn 0).
    let own_e1 = map.agent_actors().filter(|a| a.actor_type == "e1").count();
    let own_dog = map.agent_actors().filter(|a| a.actor_type == "dog").count();
    assert_eq!(own_e1, 3, "spawn 0 should have 3 e1 (count=3)");
    assert_eq!(own_dog, 2, "spawn 0 should have 2 dog (two count=1 entries)");
}

#[test]
fn spawn_point_filter_changes_agent_set() {
    let path = match scenario_path() {
        Some(p) => p,
        None => {
            eprintln!("Skipping — scenario yaml not found");
            return;
        }
    };

    let m0 = oramap::load_rush_hour_map_with_spawn(&path, 0).unwrap();
    let m1 = oramap::load_rush_hour_map_with_spawn(&path, 1).unwrap();

    // Agent positions differ by spawn point, but enemy set is identical.
    let pos0: Vec<_> = m0.agent_actors().map(|a| a.position).collect();
    let pos1: Vec<_> = m1.agent_actors().map(|a| a.position).collect();
    assert_ne!(pos0, pos1, "agent positions should differ between spawns");

    let enemy0: Vec<_> = m0.enemy_actors().map(|a| (&a.actor_type, a.position)).collect();
    let enemy1: Vec<_> = m1.enemy_actors().map(|a| (&a.actor_type, a.position)).collect();
    assert_eq!(enemy0, enemy1, "enemy set must be spawn-point-invariant");

    // Both spawn variants must have 5 own infantry.
    assert_eq!(m0.agent_actors().filter(|a| a.is_infantry()).count(), 5);
    assert_eq!(m1.agent_actors().filter(|a| a.is_infantry()).count(), 5);
}
