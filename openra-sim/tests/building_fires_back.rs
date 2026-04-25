//! Phase-7 acceptance: an armed static defense auto-fires on hostile
//! actors that wander into range.
//!
//! Spawns a `gun` (Allied turret, TurretGun: damage 6000, range 6c512)
//! and a `1tnk` (Light tank, HP 23000) within range. We do **not** issue
//! any orders to the gun — the world tick's auto-target pass should
//! pick the tank as a target and start firing. After 100 outer ticks
//! the tank's HP must be strictly less than its starting HP.

use openra_data::oramap::{MapActor, OraMap, PlayerDef};
use openra_data::rules as data_rules;
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::gamerules::GameRules;
use openra_sim::math::{CPos, WAngle, WPos};
use openra_sim::traits::TraitState;
use openra_sim::world::{
    self, insert_test_actor, set_test_unpaused, LobbyInfo, SlotInfo, World,
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
        title: "phase-7-fireback".into(),
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
    let mut world = world::build_world(&map, seed, &lobby, Some(rules), 0);
    set_test_unpaused(&mut world);
    Some(world)
}

fn make_tank(id: u32, owner: u32, at: (i32, i32), hp: i32) -> Actor {
    let cell = CPos::new(at.0, at.1);
    let center = WPos::new(at.0 * 1024 + 512, at.1 * 1024 + 512, 0);
    Actor {
        id,
        kind: ActorKind::Vehicle,
        owner_id: Some(owner),
        location: Some(at),
        traits: vec![
            TraitState::BodyOrientation { quantized_facings: 32 },
            TraitState::Mobile {
                facing: WAngle::new(512).angle,
                from_cell: cell,
                to_cell: cell,
                center_position: center,
            },
            TraitState::Health { hp },
        ],
        activity: None,
        actor_type: Some("1tnk".into()),
        kills: 0,
        rank: 0,
    }
}

fn make_gun(id: u32, owner: u32, at: (i32, i32), hp: i32) -> Actor {
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
        actor_type: Some("gun".into()),
        kills: 0,
        rank: 0,
    }
}

fn read_hp(world: &World, id: u32) -> Option<i32> {
    world.actor(id)?.traits.iter().find_map(|t| {
        if let TraitState::Health { hp } = t { Some(*hp) } else { None }
    })
}

#[test]
fn gun_fires_on_tank_in_range_no_orders() {
    let mut world = match build_arena(7) {
        Some(w) => w,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };

    // Strip auto-spawned MCVs.
    let strip: Vec<u32> = world::all_actor_ids(&world)
        .into_iter()
        .filter(|&id| matches!(world.actor_kind(id), Some(ActorKind::Mcv) | Some(ActorKind::Spawn)))
        .collect();
    for id in strip {
        world::remove_test_actor(&mut world, id);
    }

    let player_ids = world.player_ids().to_vec();
    let agent_pid = player_ids[1];
    let enemy_pid = player_ids[2];

    let initial_tank_hp = 23000;
    insert_test_actor(&mut world, make_tank(101, agent_pid, (12, 20), initial_tank_hp));
    insert_test_actor(&mut world, make_gun(102, enemy_pid, (16, 20), 40000));

    // Run 100 outer ticks (300 inner) without issuing any orders. The
    // gun must auto-target the tank and shoot.
    for _ in 0..100 {
        let _ = world.tick(&[]);
    }

    let tank_hp = read_hp(&world, 101).unwrap_or(0);
    assert!(
        tank_hp < initial_tank_hp,
        "tank HP should be reduced after 100 ticks (was {initial_tank_hp}, now {tank_hp})"
    );
}

#[test]
fn gun_does_not_fire_on_friendly_actor() {
    let mut world = match build_arena(8) {
        Some(w) => w,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };

    let strip: Vec<u32> = world::all_actor_ids(&world)
        .into_iter()
        .filter(|&id| matches!(world.actor_kind(id), Some(ActorKind::Mcv) | Some(ActorKind::Spawn)))
        .collect();
    for id in strip {
        world::remove_test_actor(&mut world, id);
    }

    let player_ids = world.player_ids().to_vec();
    let enemy_pid = player_ids[2];

    let initial_tank_hp = 23000;
    // Both gun AND tank owned by the same enemy player — gun must NOT
    // target friendly tank.
    insert_test_actor(&mut world, make_tank(101, enemy_pid, (12, 20), initial_tank_hp));
    insert_test_actor(&mut world, make_gun(102, enemy_pid, (16, 20), 40000));

    for _ in 0..100 {
        let _ = world.tick(&[]);
    }

    let tank_hp = read_hp(&world, 101).unwrap_or(0);
    assert_eq!(
        tank_hp, initial_tank_hp,
        "friendly tank HP must be unchanged (no friendly fire)"
    );
}
