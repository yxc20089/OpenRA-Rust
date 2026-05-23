//! Engine guardrail: APC transport end-to-end.
//!
//! Pins the full C# `EnterTransport` / `UnloadCargo` loop in the
//! Rust engine:
//!   1. An infantry passenger issues `EnterTransport(apc)`, walks
//!      to the APC, and boards (passenger leaves the active actor
//!      map and joins the transport's `cargo`).
//!   2. The APC drives across the map via a normal `Move` order.
//!   3. `Unload` ejects every passenger onto a passable cell next
//!      to the APC; passenger is alive and back in the actor map
//!      at the new position.
//!
//! The bench-side mirror is
//! `OpenRA-Bench/tests/test_apc_transport_end_to_end.py`.

use openra_data::oramap::{MapActor, OraMap, PlayerDef};
use openra_data::rules as data_rules;
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::gamerules::GameRules;
use openra_sim::math::{CPos, WPos};
use openra_sim::traits::TraitState;
use openra_sim::world::{
    self, all_actor_ids, insert_test_actor, set_test_unpaused, GameOrder, LobbyInfo,
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
        title: "apc-transport".into(),
        tileset: "TEMPERAT".into(),
        map_size: (64, 64),
        bounds: (0, 0, 64, 64),
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
                location: (62, 62),
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
                starting_cash: None,
            },
            SlotInfo {
                player_reference: "Multi1".into(),
                faction: "soviet".into(),
                is_bot: false,
                starting_cash: None,
            },
        ],
    };
    let mut world = world::build_world(&map, seed, &lobby, Some(rules), 0, false);
    set_test_unpaused(&mut world);
    Some(world)
}

fn make_e1(id: u32, owner: u32, at: (i32, i32)) -> Actor {
    let cell = CPos::new(at.0, at.1);
    let center = WPos::new(at.0 * 1024 + 512, at.1 * 1024 + 512, 0);
    Actor {
        id,
        kind: ActorKind::Infantry,
        owner_id: Some(owner),
        location: Some(at),
        traits: vec![
            TraitState::BodyOrientation { quantized_facings: 32 },
            TraitState::Mobile {
                facing: 512,
                from_cell: cell,
                to_cell: cell,
                center_position: center,
            },
            TraitState::Health { hp: 100_000 },
        ],
        activity: None,
        actor_type: Some("e1".into()),
        kills: 0,
        rank: 0,
    }
}

fn make_apc(id: u32, owner: u32, at: (i32, i32)) -> Actor {
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
                facing: 512,
                from_cell: cell,
                to_cell: cell,
                center_position: center,
            },
            TraitState::Health { hp: 200_000 },
        ],
        activity: None,
        actor_type: Some("apc".into()),
        kills: 0,
        rank: 0,
    }
}

fn strip_defaults(world: &mut World) {
    let strip: Vec<u32> = all_actor_ids(world)
        .into_iter()
        .filter(|&id| {
            matches!(
                world.actor_kind(id),
                Some(ActorKind::Mcv) | Some(ActorKind::Spawn),
            )
        })
        .collect();
    for id in strip {
        world::remove_test_actor(world, id);
    }
}

#[test]
fn apc_loads_drives_and_unloads_infantry_alive() {
    let mut world = match build_arena(7) {
        Some(w) => w,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };
    strip_defaults(&mut world);
    let agent = world.player_ids()[1];

    // APC at (20, 20); single rifleman one cell south.
    insert_test_actor(&mut world, make_apc(2001, agent, (20, 20)));
    insert_test_actor(&mut world, make_e1(3001, agent, (21, 20)));

    // 1. Issue EnterTransport.
    world.process_frame(&[GameOrder {
        order_string: "EnterTransport".into(),
        subject_id: Some(3001),
        target_string: None,
        extra_data: Some(2001),
    }]);

    // Step until the passenger is removed from the active actor
    // map (= boarded). Budget 100 frames is overkill (the
    // passenger is one cell away).
    let mut boarded = false;
    for _ in 0..100 {
        world.process_frame(&[]);
        if world.actor(3001).is_none() {
            boarded = true;
            break;
        }
    }
    assert!(boarded, "e1 must board the APC within step budget");
    let cargo = world.transport_cargo(2001);
    assert!(cargo.contains(&3001), "cargo manifest must list the boarded e1");

    // 2. Drive APC east ~30 cells via Move.
    let dest = (50, 20);
    world.process_frame(&[GameOrder {
        order_string: "Move".into(),
        subject_id: Some(2001),
        target_string: Some(format!("{},{}", dest.0, dest.1)),
        extra_data: None,
    }]);
    let mut arrived = false;
    for _ in 0..400 {
        world.process_frame(&[]);
        if let Some(loc) = world.actor_location(2001) {
            if (loc.0 - dest.0).abs() <= 2 && (loc.1 - dest.1).abs() <= 2 {
                arrived = true;
                break;
            }
        } else {
            break;
        }
    }
    assert!(
        arrived,
        "APC must reach within 2 cells of {:?}; final loc={:?}",
        dest,
        world.actor_location(2001)
    );

    // 3. Unload the passenger.
    world.process_frame(&[GameOrder {
        order_string: "Unload".into(),
        subject_id: Some(2001),
        target_string: None,
        extra_data: None,
    }]);
    // The engine ejects passengers synchronously inside the order.
    // Settle one frame.
    world.process_frame(&[]);

    let cargo_after = world.transport_cargo(2001);
    assert!(
        cargo_after.is_empty(),
        "cargo must be empty after Unload; still has {cargo_after:?}",
    );
    let e1_loc = world
        .actor_location(3001)
        .expect("e1 must be back in the world after Unload");
    let cheb = (e1_loc.0 - dest.0).abs().max((e1_loc.1 - dest.1).abs());
    assert!(
        cheb <= 4,
        "unloaded e1 must land within 4 cells of the APC's destination \
         ({:?}); got {:?} (Chebyshev={cheb})",
        dest,
        e1_loc,
    );
}
