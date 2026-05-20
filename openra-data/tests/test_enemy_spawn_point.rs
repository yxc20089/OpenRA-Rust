//! Wave-9 engine-fix test: `expand_scenario_actors` must honour
//! `spawn_point:` on ENEMY actors too (per-owner activation), not just
//! on agent actors.
//!
//! The pre-fix behaviour was: enemy actors with `spawn_point` set were
//! kept regardless of the requested spawn_point (the filter applied
//! only to agent actors). That made per-seed enemy-composition
//! variation impossible — the only seed axis was the agent's spawn
//! corner.
//!
//! Post-fix: when ANY enemy actor declares `spawn_point`, the filter
//! activates for enemies too — only enemies whose `spawn_point`
//! matches the requested one pass through. The agent-side semantics
//! are mirrored exactly: enemy actors WITHOUT `spawn_point` are
//! filtered out (the established "duplicate the persistent marker
//! across every spawn group" idiom is required for enemy markers too).
//!
//! The agent-side filter is independent: the two owners activate
//! separately, so a scenario can fix the agent base across all seeds
//! (no agent declares spawn_point → agent filter inactive → all agent
//! actors pass) while rotating the enemy archetype (enemies declare
//! spawn_point → enemy filter active → only matching archetype places).
//!
//! This file also exercises the new `distinct_enemy_spawn_points`
//! helper, which parallels `distinct_agent_spawn_points`.

use openra_data::oramap::{
    distinct_agent_spawn_points, distinct_enemy_spawn_points, load_rush_hour_map_with_spawn,
};
use std::path::{Path, PathBuf};

/// Locate the shared rush-hour base map. Without it, the loader can't
/// resolve a base — skip the test rather than failing on CI without
/// the training repo checked out.
fn base_map_path() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("HOME") {
        for candidate in [
            "Projects/OpenRA-RL-Training/scenarios/maps/rush-hour-arena.oramap",
            "Projects/openra-rl/maps/rush-hour-arena.oramap",
        ] {
            let p = PathBuf::from(&home).join(candidate);
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

/// Write a scenario YAML to `dir` that references `base_map_abs` and
/// has the given actor block. Returns the scenario yaml path.
fn write_scenario(dir: &Path, name: &str, base_map_abs: &Path, actors_yaml: &str) -> PathBuf {
    let scen_path = dir.join(name);
    let body = format!(
        "name: Test\n\
         description: enemy spawn_point test\n\
         base_map: {}\n\
         spawn_mcvs: false\n\
         agent:\n  faction: allies\n\
         enemy:\n  faction: soviet\n\
         actors:\n{}\n",
        base_map_abs.display(),
        actors_yaml
    );
    std::fs::write(&scen_path, body).expect("write scenario yaml");
    scen_path
}

#[test]
fn enemy_spawn_point_filters_per_owner() {
    let base = match base_map_path() {
        Some(p) => p,
        None => {
            eprintln!(
                "Skipping enemy_spawn_point_filters_per_owner — rush-hour base map not found"
            );
            return;
        }
    };
    let tmp = std::env::temp_dir().join(format!(
        "openra-enemy-spawn-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&tmp).expect("mkdir tmpdir");

    // Scenario A: agent declares spawn_point=0/1; enemy declares
    // spawn_point=0/1 on the composition actors AND duplicates the
    // persistent fact marker across every spawn group (mirroring the
    // agent-side "duplicate across both spawn groups" idiom — actors
    // of an owner whose filter is active and that DON'T declare
    // spawn_point are filtered out).
    let actors = "\
- type: e1
  owner: agent
  position:
  - 10
  - 20
  spawn_point: 0
- type: e1
  owner: agent
  position:
  - 12
  - 22
  spawn_point: 1
- type: e1
  owner: enemy
  position:
  - 70
  - 18
  spawn_point: 0
- type: e1
  owner: enemy
  position:
  - 70
  - 20
  spawn_point: 0
- type: 3tnk
  owner: enemy
  position:
  - 70
  - 19
  spawn_point: 1
- type: 3tnk
  owner: enemy
  position:
  - 70
  - 21
  spawn_point: 1
- type: fact
  owner: enemy
  position:
  - 124
  - 20
  spawn_point: 0
- type: fact
  owner: enemy
  position:
  - 124
  - 20
  spawn_point: 1
- type: fact
  owner: enemy
  position:
  - 124
  - 22
";
    let scen = write_scenario(&tmp, "enemy-spawn.yaml", &base, actors);

    // Spawn 0: the e1 enemies place; 3tnk does NOT; sp=0 fact marker
    // places; the no-spawn_point marker is FILTERED OUT (must be
    // duplicated across spawn groups, mirroring agent semantics).
    let m0 = load_rush_hour_map_with_spawn(&scen, 0).expect("load m0");
    let enemy_e1_0 = m0
        .actors
        .iter()
        .filter(|a| a.owner == "enemy" && a.actor_type == "e1")
        .count();
    let enemy_3tnk_0 = m0
        .actors
        .iter()
        .filter(|a| a.owner == "enemy" && a.actor_type == "3tnk")
        .count();
    let enemy_fact_0 = m0
        .actors
        .iter()
        .filter(|a| a.owner == "enemy" && a.actor_type == "fact")
        .count();
    assert_eq!(enemy_e1_0, 2, "spawn 0 must place both e1 enemies");
    assert_eq!(
        enemy_3tnk_0, 0,
        "spawn 0 must FILTER OUT the spawn_point=1 3tnk enemies (got {enemy_3tnk_0})"
    );
    assert_eq!(
        enemy_fact_0, 1,
        "spawn 0 must keep ONLY the spawn_point=0 fact marker; the \
         no-spawn_point fact (mirroring agent semantics) is filtered out"
    );

    // Spawn 1: the 3tnk enemies place; e1 does NOT; sp=1 fact marker
    // places.
    let m1 = load_rush_hour_map_with_spawn(&scen, 1).expect("load m1");
    let enemy_e1_1 = m1
        .actors
        .iter()
        .filter(|a| a.owner == "enemy" && a.actor_type == "e1")
        .count();
    let enemy_3tnk_1 = m1
        .actors
        .iter()
        .filter(|a| a.owner == "enemy" && a.actor_type == "3tnk")
        .count();
    let enemy_fact_1 = m1
        .actors
        .iter()
        .filter(|a| a.owner == "enemy" && a.actor_type == "fact")
        .count();
    assert_eq!(
        enemy_e1_1, 0,
        "spawn 1 must FILTER OUT the spawn_point=0 e1 enemies (got {enemy_e1_1})"
    );
    assert_eq!(enemy_3tnk_1, 2, "spawn 1 must place both 3tnk enemies");
    assert_eq!(
        enemy_fact_1, 1,
        "spawn 1 must keep ONLY the spawn_point=1 fact marker"
    );

    // Agent filter activates independently:
    let agent_pos_0: Vec<_> = m0
        .actors
        .iter()
        .filter(|a| a.owner == "agent")
        .map(|a| a.position)
        .collect();
    let agent_pos_1: Vec<_> = m1
        .actors
        .iter()
        .filter(|a| a.owner == "agent")
        .map(|a| a.position)
        .collect();
    assert_eq!(agent_pos_0, vec![(10, 20)]);
    assert_eq!(agent_pos_1, vec![(12, 22)]);

    // distinct_enemy_spawn_points must return [0, 1] sorted.
    let enemy_sps = distinct_enemy_spawn_points(&scen).expect("distinct_enemy_spawn_points");
    assert_eq!(enemy_sps, vec![0, 1]);
    let agent_sps = distinct_agent_spawn_points(&scen).expect("distinct_agent_spawn_points");
    assert_eq!(agent_sps, vec![0, 1]);
}

#[test]
fn enemy_no_spawn_point_passes_through_backcompat() {
    let base = match base_map_path() {
        Some(p) => p,
        None => {
            eprintln!(
                "Skipping enemy_no_spawn_point_passes_through_backcompat — base map not found"
            );
            return;
        }
    };
    let tmp = std::env::temp_dir().join(format!(
        "openra-enemy-spawn-back-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&tmp).expect("mkdir");

    // Scenario B: NO enemy declares spawn_point — all enemy actors
    // must pass through on every requested spawn (pre-Wave-9 contract).
    let actors = "\
- type: e1
  owner: agent
  position:
  - 10
  - 20
  spawn_point: 0
- type: e1
  owner: agent
  position:
  - 12
  - 22
  spawn_point: 1
- type: e1
  owner: enemy
  position:
  - 70
  - 18
- type: e1
  owner: enemy
  position:
  - 70
  - 20
- type: 3tnk
  owner: enemy
  position:
  - 70
  - 19
- type: fact
  owner: enemy
  position:
  - 124
  - 20
";
    let scen = write_scenario(&tmp, "enemy-no-sp.yaml", &base, actors);

    for sp in [0_i32, 1] {
        let m = load_rush_hour_map_with_spawn(&scen, sp).expect("load");
        let enemies: Vec<_> = m
            .actors
            .iter()
            .filter(|a| a.owner == "enemy")
            .map(|a| (a.actor_type.as_str(), a.position))
            .collect();
        assert_eq!(
            enemies.len(),
            4,
            "back-compat: all 4 enemies (no spawn_point set on any) place on spawn {sp}, got {enemies:?}"
        );
    }

    // distinct_enemy_spawn_points must be empty when no enemy uses sp.
    let enemy_sps = distinct_enemy_spawn_points(&scen).expect("distinct_enemy_spawn_points");
    assert!(
        enemy_sps.is_empty(),
        "no enemy declares spawn_point → distinct_enemy_spawn_points must be empty, got {enemy_sps:?}"
    );
}
