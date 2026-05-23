//! Tanya C4 commando ability — instant-destroy an enemy building on
//! adjacency.
//!
//! C4Detonate is Tanya's signature in RA: she walks to a target
//! building, plants C4, the building explodes, Tanya runs away
//! unharmed. MVP simplification: no escape animation — Tanya simply
//! survives the detonation in place.
//!
//! Pinned contracts:
//!  1. Issued with subject = tanya, target = enemy proc, the proc is
//!     destroyed within a few dozen ticks and Tanya stays alive.
//!  2. Validation: a non-tanya subject (e1) gets its C4Detonate order
//!     dropped — the building is NOT destroyed.
//!  3. Validation: a friendly building target is rejected — same
//!     building stays alive.
//!
//! Runs on `GameRules::defaults()` (no vendored RA YAML required) —
//! both `tanya` (added in the prior commit on this branch) and `proc`
//! are in the defaults actor table.

use openra_data::oramap::{OraMap, PlayerDef};
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::traits::TraitState;
use openra_sim::world::{
    self, insert_test_actor, set_test_unpaused, GameOrder, LobbyInfo, World,
};

fn arena(w: i32, h: i32) -> World {
    let map = OraMap {
        title: "tanya-c4-arena".into(),
        tileset: "TEMPERAT".into(),
        map_size: (w, h),
        bounds: (0, 0, w, h),
        tiles: Vec::new(),
        actors: Vec::new(),
        players: vec![PlayerDef {
            name: "Neutral".into(),
            playable: false,
            owns_world: true,
            non_combatant: true,
            faction: "allies".into(),
            enemies: Vec::new(),
        }],
    };
    let mut world = world::build_world(&map, 0, &LobbyInfo::default(), None, 0, true);
    set_test_unpaused(&mut world);
    world
}

fn make_infantry(id: u32, owner: u32, actor_type: &str, at: (i32, i32), hp: i32) -> Actor {
    Actor {
        id,
        kind: ActorKind::Infantry,
        owner_id: Some(owner),
        location: Some(at),
        traits: vec![TraitState::Health { hp }],
        activity: None,
        actor_type: Some(actor_type.into()),
        kills: 0,
        rank: 0,
    }
}

fn make_building(id: u32, owner: u32, actor_type: &str, at: (i32, i32), hp: i32) -> Actor {
    Actor {
        id,
        kind: ActorKind::Building,
        owner_id: Some(owner),
        location: Some(at),
        traits: vec![TraitState::Health { hp }],
        activity: None,
        actor_type: Some(actor_type.into()),
        kills: 0,
        rank: 0,
    }
}

fn c4_order(subject: u32, target: u32) -> GameOrder {
    GameOrder {
        order_string: "C4Detonate".into(),
        subject_id: Some(subject),
        target_string: None,
        extra_data: Some(target),
    }
}

#[test]
fn tanya_c4_destroys_enemy_proc_and_survives() {
    let mut world = arena(40, 40);

    let tanya_id: u32 = 100;
    let proc_id: u32 = 200;
    let tanya_hp = 150_000;
    let proc_hp = 90_000;

    // Tanya at (10,10), owner 1 (enemy of owner 2).
    insert_test_actor(
        &mut world,
        make_infantry(tanya_id, 1, "tanya", (10, 10), tanya_hp),
    );
    // Enemy proc footprint top-left (15, 10), 3x2 — close enough that
    // Tanya can walk over in a reasonable tick budget on the open
    // arena (no terrain blocking).
    insert_test_actor(
        &mut world,
        make_building(proc_id, 2, "proc", (15, 10), proc_hp),
    );

    world.process_frame(&[c4_order(tanya_id, proc_id)]);

    // Activity should be set to C4Plant immediately after the order
    // is processed.
    let act = world.actor(tanya_id).and_then(|a| a.activity.clone());
    assert!(
        matches!(act, Some(openra_sim::actor::Activity::C4Plant { .. })),
        "tanya should have a C4Plant activity, got {:?}",
        act
    );

    // Walk + detonate. Tanya is 5 cells from the proc footprint, so
    // ≤4 step-tick approaches + 1 detonate tick. 50 frames is ample.
    let mut destroyed_at = None;
    for tick in 1..=200 {
        world.process_frame(&[]);
        if world.actor(proc_id).is_none() {
            destroyed_at = Some(tick);
            break;
        }
    }

    assert!(
        destroyed_at.is_some(),
        "proc should have been destroyed within 200 ticks"
    );
    eprintln!("proc destroyed at tick {}", destroyed_at.unwrap());

    // Tanya survives, fully healthy.
    let tanya = world.actor(tanya_id).expect("tanya must still exist");
    let tanya_hp_final = tanya
        .traits
        .iter()
        .find_map(|t| match t {
            TraitState::Health { hp } => Some(*hp),
            _ => None,
        })
        .unwrap_or(0);
    assert_eq!(
        tanya_hp_final, tanya_hp,
        "tanya should be unharmed after planting C4 (HP {}/{})",
        tanya_hp_final, tanya_hp
    );

    // Tanya is no longer in a C4Plant activity (went idle after the boom).
    assert!(
        !matches!(
            tanya.activity,
            Some(openra_sim::actor::Activity::C4Plant { .. })
        ),
        "tanya should leave the C4Plant activity after detonation"
    );

    // Kill was credited.
    assert!(tanya.kills >= 1, "tanya should be credited with the proc kill");
}

#[test]
fn non_tanya_c4_order_is_rejected() {
    let mut world = arena(40, 40);

    let e1_id: u32 = 101;
    let proc_id: u32 = 201;
    let proc_hp = 90_000;

    // An ordinary e1 rifleman tries to "C4" an enemy proc.
    insert_test_actor(&mut world, make_infantry(e1_id, 1, "e1", (10, 10), 50_000));
    insert_test_actor(
        &mut world,
        make_building(proc_id, 2, "proc", (15, 10), proc_hp),
    );

    world.process_frame(&[c4_order(e1_id, proc_id)]);

    // The e1 must NOT have received a C4Plant activity — the engine
    // dropped the order silently.
    let act = world.actor(e1_id).and_then(|a| a.activity.clone());
    assert!(
        !matches!(act, Some(openra_sim::actor::Activity::C4Plant { .. })),
        "non-tanya subject should not get a C4Plant activity; got {:?}",
        act
    );

    // Run a generous tick budget — nothing should detonate.
    for _ in 0..200 {
        world.process_frame(&[]);
    }

    let proc_alive = world.actor(proc_id).is_some();
    assert!(
        proc_alive,
        "proc must NOT be destroyed by a non-tanya C4Detonate order"
    );
}

#[test]
fn c4_on_friendly_building_is_rejected() {
    let mut world = arena(40, 40);

    let tanya_id: u32 = 102;
    let friendly_proc_id: u32 = 202;

    // Tanya AND target proc are BOTH owner 1 — friendly. Engine must
    // reject the order so a misclick can't suicide our own refinery.
    insert_test_actor(
        &mut world,
        make_infantry(tanya_id, 1, "tanya", (10, 10), 150_000),
    );
    insert_test_actor(
        &mut world,
        make_building(friendly_proc_id, 1, "proc", (15, 10), 90_000),
    );

    world.process_frame(&[c4_order(tanya_id, friendly_proc_id)]);

    let act = world.actor(tanya_id).and_then(|a| a.activity.clone());
    assert!(
        !matches!(act, Some(openra_sim::actor::Activity::C4Plant { .. })),
        "C4 on a friendly target should be dropped; got activity {:?}",
        act
    );

    for _ in 0..50 {
        world.process_frame(&[]);
    }
    assert!(
        world.actor(friendly_proc_id).is_some(),
        "friendly proc must survive a (rejected) friendly-fire C4Detonate"
    );
}
