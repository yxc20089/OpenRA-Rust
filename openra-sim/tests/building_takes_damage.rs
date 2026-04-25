//! Phase-7 acceptance: a static defense building dies when shot at.
//!
//! Spawns a `pbox` and a `1tnk` 5 cells apart, issues an Attack order
//! from the tank, and verifies the pbox HP drops to zero (the actor is
//! removed from the world) within a reasonable tick budget.
//!
//! Uses the vendored RA ruleset so weapon stats (25mm tank cannon vs
//! pillbox HP) come from `weapons.yaml`.

use openra_data::oramap::{MapActor, OraMap, PlayerDef};
use openra_data::rules as data_rules;
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::gamerules::GameRules;
use openra_sim::math::{CPos, WAngle, WPos};
use openra_sim::traits::TraitState;
use openra_sim::world::{
    self, insert_test_actor, set_test_unpaused, GameOrder, LobbyInfo, SlotInfo, World,
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
        title: "phase-7".into(),
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

fn make_pbox(id: u32, owner: u32, at: (i32, i32), hp: i32) -> Actor {
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
        actor_type: Some("pbox".into()),
        kills: 0,
        rank: 0,
    }
}

fn target_hp(world: &World, id: u32) -> Option<i32> {
    world.actor(id)?.traits.iter().find_map(|t| {
        if let TraitState::Health { hp } = t { Some(*hp) } else { None }
    })
}

#[test]
fn pillbox_dies_when_attacked_by_tank() {
    let mut world = match build_arena(42) {
        Some(w) => w,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };

    // Strip auto-spawned MCVs.
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

    // Place attacker and target 5 cells apart. 1tnk 25mm range is
    // 4c768 ≈ 4.75 cells so we put them at distance 4 (in range).
    insert_test_actor(&mut world, make_tank(101, agent_pid, (10, 20), 23000));
    insert_test_actor(&mut world, make_pbox(102, enemy_pid, (14, 20), 40000));

    let attack = GameOrder {
        order_string: "Attack".into(),
        subject_id: Some(101),
        target_string: None,
        extra_data: Some(102),
    };

    // 1tnk damages 2500/shot, pbox HP=40000, reload 21 inner ticks, 3
    // inner ticks per outer tick → ~ceil(40000/2500)=16 shots, ~107
    // outer ticks. Allow 600 to absorb building counter-fire (which
    // damages the tank but shouldn't kill it before the pbox dies)
    // and to give the typed-shroud refresh enough headroom.
    let attack_orders = vec![attack];
    let mut kill_tick = None;
    for tick in 1..=2000 {
        let no = Vec::new();
        let orders = if tick == 1 { &attack_orders } else { &no };
        let _ = world.tick(orders);
        if target_hp(&world, 102).is_none() {
            kill_tick = Some(tick);
            break;
        }
        if let Some(hp) = target_hp(&world, 102) {
            if hp <= 0 {
                kill_tick = Some(tick);
                break;
            }
        }
    }
    let kill_tick = kill_tick.expect("expected pbox to die within 2000 outer ticks");
    eprintln!("pbox died at outer tick {kill_tick}");
    assert!(
        world.actor(102).is_none(),
        "pbox actor 102 should be removed after death"
    );
}
