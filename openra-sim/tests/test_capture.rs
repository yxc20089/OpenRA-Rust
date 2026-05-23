//! Engine acceptance: an allied **engineer** (`e6`) walks to an enemy
//! building (`proc`) and CAPTURES it — on arrival the target's owner
//! transfers to the engineer's player and the engineer is removed
//! from the world (consumed).
//!
//! Mirrors the `EnterTransport` drive loop (one-cell-per-outer-tick
//! step, gap≤1 then board) but the on-arrival event is "transfer
//! ownership + drop the capturer" instead of "stash as cargo".
//!
//! Three test cases:
//!   1. `engineer_captures_enemy_building` — happy path: e6 walks,
//!      reaches the proc, owner flips to the agent, engineer gone.
//!   2. `engineer_does_not_capture_own_building` — same-owner target
//!      is a no-op (engineer stays alive, building untouched).
//!   3. `non_engineer_cannot_capture` — an `e1` rifleman with the
//!      same order does NOT capture (order rejected at issue time).

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
        title: "capture-arena".into(),
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

fn make_infantry(id: u32, owner: u32, at: (i32, i32), hp: i32, ty: &str) -> Actor {
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
                facing: WAngle::new(512).angle,
                from_cell: cell,
                to_cell: cell,
                center_position: center,
            },
            TraitState::Health { hp },
        ],
        activity: None,
        actor_type: Some(ty.into()),
        kills: 0,
        rank: 0,
    }
}

fn make_proc(id: u32, owner: u32, at: (i32, i32), hp: i32) -> Actor {
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
        actor_type: Some("proc".into()),
        kills: 0,
        rank: 0,
    }
}

fn strip_defaults(world: &mut World) {
    let strip: Vec<u32> = world::all_actor_ids(world)
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

fn capture_order(engineer_id: u32, target_id: u32) -> GameOrder {
    GameOrder {
        order_string: "CaptureActor".into(),
        subject_id: Some(engineer_id),
        target_string: None,
        extra_data: Some(target_id),
    }
}

#[test]
fn engineer_captures_enemy_building() {
    let mut world = match build_arena(11) {
        Some(w) => w,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };
    strip_defaults(&mut world);

    let player_ids = world.player_ids().to_vec();
    let agent_pid = player_ids[1];
    let enemy_pid = player_ids[2];

    // Engineer 4 cells west of an enemy refinery — clear ground in
    // between, no obstacles.
    let engineer_id: u32 = 301;
    let proc_id: u32 = 302;
    insert_test_actor(
        &mut world,
        make_infantry(engineer_id, agent_pid, (16, 20), 25000, "e6"),
    );
    insert_test_actor(
        &mut world,
        make_proc(proc_id, enemy_pid, (20, 20), 90000),
    );

    // Issue the capture order on tick 0.
    let _ = world.tick(&[capture_order(engineer_id, proc_id)]);

    // Drive forward up to 400 outer ticks. The engineer should walk
    // ~4 cells (1 cell per outer tick) and then the on-arrival pulse
    // flips ownership and removes the engineer.
    let mut captured = false;
    for _ in 0..400 {
        let _ = world.tick(&[]);
        let target_owner = world.actor(proc_id).and_then(|a| a.owner_id);
        let engineer_gone = world.actor(engineer_id).is_none();
        if target_owner == Some(agent_pid) && engineer_gone {
            captured = true;
            break;
        }
    }
    assert!(
        captured,
        "engineer should have captured the proc and been consumed within 400 ticks \
         (proc.owner={:?}, engineer_alive={})",
        world.actor(proc_id).and_then(|a| a.owner_id),
        world.actor(engineer_id).is_some(),
    );

    // The proc must still be a Building (no kind change).
    let proc = world.actor(proc_id).expect("proc should still exist");
    assert_eq!(proc.kind, ActorKind::Building);
    // And its actor_type unchanged.
    assert_eq!(proc.actor_type.as_deref(), Some("proc"));
}

#[test]
fn engineer_does_not_capture_own_building() {
    let mut world = match build_arena(12) {
        Some(w) => w,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };
    strip_defaults(&mut world);

    let player_ids = world.player_ids().to_vec();
    let agent_pid = player_ids[1];

    let engineer_id: u32 = 311;
    let proc_id: u32 = 312;
    insert_test_actor(
        &mut world,
        make_infantry(engineer_id, agent_pid, (16, 20), 25000, "e6"),
    );
    insert_test_actor(
        &mut world,
        make_proc(proc_id, agent_pid, (20, 20), 90000),
    );

    let _ = world.tick(&[capture_order(engineer_id, proc_id)]);
    for _ in 0..50 {
        let _ = world.tick(&[]);
    }

    // Same-owner target — order is a no-op. Engineer stays alive,
    // proc still ours.
    assert!(
        world.actor(engineer_id).is_some(),
        "engineer must NOT be consumed by a same-owner capture order"
    );
    assert_eq!(
        world.actor(proc_id).and_then(|a| a.owner_id),
        Some(agent_pid),
        "same-owner proc ownership must be unchanged"
    );
}

#[test]
fn non_engineer_cannot_capture() {
    let mut world = match build_arena(13) {
        Some(w) => w,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };
    strip_defaults(&mut world);

    let player_ids = world.player_ids().to_vec();
    let agent_pid = player_ids[1];
    let enemy_pid = player_ids[2];

    // An e1 rifleman, NOT an engineer. Same geometry as the happy
    // path. The capture order must be rejected at issue time and the
    // proc must remain enemy-owned indefinitely.
    let rifleman_id: u32 = 321;
    let proc_id: u32 = 322;
    insert_test_actor(
        &mut world,
        make_infantry(rifleman_id, agent_pid, (16, 20), 50000, "e1"),
    );
    insert_test_actor(
        &mut world,
        make_proc(proc_id, enemy_pid, (20, 20), 90000),
    );

    let _ = world.tick(&[capture_order(rifleman_id, proc_id)]);
    for _ in 0..200 {
        let _ = world.tick(&[]);
    }

    assert_eq!(
        world.actor(proc_id).and_then(|a| a.owner_id),
        Some(enemy_pid),
        "rifleman (e1) must not capture — proc must still be enemy-owned"
    );
    assert!(
        world.actor(rifleman_id).is_some(),
        "rifleman must still be alive (non-engineer capture order is a no-op)"
    );
}
