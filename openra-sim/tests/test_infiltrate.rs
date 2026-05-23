//! Phase: spy / thief Infiltrate engine order.
//!
//! Two sub-tests:
//!
//!   * `spy_infiltration_reveals_enemy_buildings` — a spy walks into
//!     an enemy `proc`; afterwards every building owned by that proc's
//!     owner is in the agent's `infiltration_revealed_buildings`
//!     reveal set (the one-shot scan), and the spy is consumed.
//!
//!   * `thief_infiltration_steals_enemy_cash` — a thief walks into an
//!     enemy `silo`; afterwards a chunk of cash has transferred from
//!     the silo owner to the thief's owner, and the thief is consumed.

use openra_data::oramap::{MapActor, OraMap, PlayerDef};
use openra_data::rules as data_rules;
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::gamerules::GameRules;
use openra_sim::math::{CPos, WAngle, WPos};
use openra_sim::traits::TraitState;
use openra_sim::world::{
    self, insert_test_actor, set_test_cash, set_test_unpaused, GameOrder, LobbyInfo,
    SlotInfo, World,
};
use std::path::PathBuf;

fn vendor_mod_dir() -> Option<PathBuf> {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let p = PathBuf::from(format!("{manifest}/../vendor/OpenRA/mods/ra"));
    if p.exists() { Some(p) } else { None }
}

fn build_arena(seed: i32) -> Option<World> {
    let mod_dir = vendor_mod_dir()?;
    let ruleset = data_rules::load_ruleset(&mod_dir).ok()?;
    let rules = GameRules::from_ruleset(&ruleset);

    let map = OraMap {
        title: "infiltrate-arena".into(),
        tileset: "TEMPERAT".into(),
        map_size: (40, 40),
        bounds: (0, 0, 40, 40),
        tiles: Vec::new(),
        actors: vec![
            MapActor {
                id: "mpspawn1".into(),
                actor_type: "mpspawn".into(),
                owner: "Neutral".into(),
                location: (1, 1),
            },
            MapActor {
                id: "mpspawn2".into(),
                actor_type: "mpspawn".into(),
                owner: "Neutral".into(),
                location: (38, 38),
            },
        ],
        players: vec![
            PlayerDef {
                name: "Neutral".into(),
                playable: false,
                owns_world: true,
                non_combatant: true,
                faction: "allies".into(),
                enemies: Vec::new(),
            },
            PlayerDef {
                name: "Multi0".into(),
                playable: true,
                owns_world: false,
                non_combatant: false,
                faction: "allies".into(),
                enemies: Vec::new(),
            },
            PlayerDef {
                name: "Multi1".into(),
                playable: true,
                owns_world: false,
                non_combatant: false,
                faction: "soviet".into(),
                enemies: Vec::new(),
            },
        ],
    };
    let lobby = LobbyInfo {
        starting_cash: 0,
        allow_spectators: false,
        occupied_slots: vec![
            SlotInfo {
                player_reference: "Multi0".into(),
                faction: "allies".into(),
                is_bot: false,
            },
            SlotInfo {
                player_reference: "Multi1".into(),
                faction: "soviet".into(),
                is_bot: false,
            },
        ],
    };
    let mut world = world::build_world(&map, seed, &lobby, Some(rules), 0, false);
    set_test_unpaused(&mut world);
    Some(world)
}

fn make_infantry(id: u32, owner: u32, actor_type: &str, at: (i32, i32), hp: i32) -> Actor {
    let cell = CPos::new(at.0, at.1);
    let center = WPos::new(at.0 * 1024 + 512, at.1 * 1024 + 512, 0);
    Actor {
        id,
        kind: ActorKind::Infantry,
        owner_id: Some(owner),
        location: Some(at),
        traits: vec![
            TraitState::BodyOrientation { quantized_facings: 8 },
            TraitState::Mobile {
                facing: WAngle::new(0).angle,
                from_cell: cell,
                to_cell: cell,
                center_position: center,
            },
            TraitState::Health { hp },
        ],
        activity: None,
        actor_type: Some(actor_type.into()),
        kills: 0,
        rank: 0,
    }
}

fn make_building(
    id: u32,
    owner: u32,
    actor_type: &str,
    at: (i32, i32),
    hp: i32,
) -> Actor {
    let cell = CPos::new(at.0, at.1);
    let center = WPos::new(at.0 * 1024 + 512, at.1 * 1024 + 512, 0);
    Actor {
        id,
        kind: ActorKind::Building,
        owner_id: Some(owner),
        location: Some(at),
        traits: vec![
            TraitState::BodyOrientation { quantized_facings: 1 },
            TraitState::Building { top_left: cell },
            TraitState::Immobile { top_left: cell, center_position: center },
            TraitState::Health { hp },
        ],
        activity: None,
        actor_type: Some(actor_type.into()),
        kills: 0,
        rank: 0,
    }
}

/// Drive the world long enough for an adjacent-cell infiltrator to
/// reach the target. The infiltrator starts one cell away so a single
/// tick is enough — but allow a buffer to avoid flake under any future
/// path-cost change.
fn run_until_consumed(world: &mut World, infiltrator_id: u32, max_ticks: u32) -> bool {
    for _ in 0..max_ticks {
        let _ = world.tick(&[]);
        if world.actor(infiltrator_id).is_none() {
            return true;
        }
    }
    false
}

#[test]
fn spy_infiltration_reveals_enemy_buildings() {
    let mut world = match build_arena(11) {
        Some(w) => w,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };

    // Strip auto-spawned MCVs / spawn markers — keep the arena minimal.
    let strip: Vec<u32> = world::all_actor_ids(&world)
        .into_iter()
        .filter(|&id| {
            matches!(
                world.actor_kind(id),
                Some(ActorKind::Mcv) | Some(ActorKind::Spawn)
            )
        })
        .collect();
    for id in strip {
        world::remove_test_actor(&mut world, id);
    }

    let player_ids = world.player_ids().to_vec();
    let agent_pid = player_ids[1];
    let enemy_pid = player_ids[2];

    // Spy adjacent to the target proc; two other enemy buildings
    // placed elsewhere on the map (out of the spy's natural sight) to
    // verify the one-shot scan finds ALL of the target-owner's
    // structures, not just the one being entered.
    insert_test_actor(&mut world, make_infantry(1001, agent_pid, "spy", (10, 20), 25000));
    insert_test_actor(&mut world, make_building(2001, enemy_pid, "proc", (11, 20), 90000));
    insert_test_actor(&mut world, make_building(2002, enemy_pid, "powr", (32, 32), 40000));
    insert_test_actor(&mut world, make_building(2003, enemy_pid, "barr", (35, 35), 50000));

    // Issue Infiltrate.
    let order = GameOrder {
        order_string: "Infiltrate".into(),
        subject_id: Some(1001),
        target_string: None,
        extra_data: Some(2001),
    };
    let _ = world.tick(&[order]);

    // The spy is adjacent (gap=1) at order issue, so the next world
    // tick should consume it; allow a small buffer.
    let consumed = run_until_consumed(&mut world, 1001, 8);
    assert!(consumed, "spy should be consumed by infiltration");

    // Every enemy building must be in the agent's reveal set.
    for bid in [2001u32, 2002, 2003] {
        assert!(
            world.was_infiltration_revealed(agent_pid, bid),
            "enemy building {bid} should be revealed after spy infiltration"
        );
    }
    // Sanity: a building belonging to the agent is NOT in the set.
    insert_test_actor(&mut world, make_building(3001, agent_pid, "fact", (5, 5), 100000));
    assert!(
        !world.was_infiltration_revealed(agent_pid, 3001),
        "agent's own building must not appear in the reveal set"
    );
}

#[test]
fn thief_infiltration_steals_enemy_cash() {
    let mut world = match build_arena(12) {
        Some(w) => w,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };

    let strip: Vec<u32> = world::all_actor_ids(&world)
        .into_iter()
        .filter(|&id| {
            matches!(
                world.actor_kind(id),
                Some(ActorKind::Mcv) | Some(ActorKind::Spawn)
            )
        })
        .collect();
    for id in strip {
        world::remove_test_actor(&mut world, id);
    }

    let player_ids = world.player_ids().to_vec();
    let agent_pid = player_ids[1];
    let enemy_pid = player_ids[2];

    // Seed the wallets: enemy has cash to steal, agent starts at zero.
    set_test_cash(&mut world, enemy_pid, 2000);
    set_test_cash(&mut world, agent_pid, 0);

    // Thief adjacent to a silo (silos are stealable in our parity set).
    insert_test_actor(&mut world, make_infantry(1101, agent_pid, "thf", (10, 20), 50000));
    insert_test_actor(&mut world, make_building(2101, enemy_pid, "silo", (11, 20), 30000));

    let enemy_before = world.player_cash(enemy_pid);
    let agent_before = world.player_cash(agent_pid);

    let order = GameOrder {
        order_string: "Infiltrate".into(),
        subject_id: Some(1101),
        target_string: None,
        extra_data: Some(2101),
    };
    let _ = world.tick(&[order]);
    let consumed = run_until_consumed(&mut world, 1101, 8);
    assert!(consumed, "thief should be consumed by infiltration");

    let enemy_after = world.player_cash(enemy_pid);
    let agent_after = world.player_cash(agent_pid);
    let stolen = enemy_before - enemy_after;
    let gained = agent_after - agent_before;
    assert!(
        stolen > 0,
        "thief should drain some enemy cash (before {enemy_before}, after {enemy_after})"
    );
    assert_eq!(
        stolen, gained,
        "every cash unit removed from the enemy must be credited to the agent"
    );
}
