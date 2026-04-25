//! Phase-3 combat: out-of-range chase.
//!
//! Spawn A and B 30 cells apart (well beyond M1Carbine range = 5
//! cells), issue Attack from A → B, verify A pushes a Move child
//! activity to close, then (simulating arrival) fires once it gets
//! in range. Driving the full data-driven Move loop end-to-end is
//! exercised in `combat_one_v_one.rs` and the parity test; here we
//! verify the *structural* contract that `AttackActivity` queues a
//! chase Move and then fires post-arrival.

use openra_data::oramap::{OraMap, PlayerDef};
use openra_data::rules::{WDist, WeaponStats};
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::activities::AttackActivity;
use openra_sim::activity::{Activity, ActivityState};
use openra_sim::traits::{Armament, TraitState};
use openra_sim::world::{self, insert_test_actor, set_test_unpaused, LobbyInfo};

fn empty_world(w: i32, h: i32) -> openra_sim::world::World {
    let map = OraMap {
        title: "tiny".into(),
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
    let mut world = world::build_world(&map, 0, &LobbyInfo::default(), None, 0);
    set_test_unpaused(&mut world);
    world
}

fn make_e1(id: u32, owner: u32, at: (i32, i32), hp: i32) -> Actor {
    Actor {
        id,
        kind: ActorKind::Infantry,
        owner_id: Some(owner),
        location: Some(at),
        traits: vec![TraitState::Health { hp }],
        activity: None,
        actor_type: Some("e1".into()),
        kills: 0,
        rank: 0,
    }
}

fn m1carbine() -> WeaponStats {
    WeaponStats {
        name: "M1Carbine".into(),
        range: WDist::from_cells(5),
        reload_delay: 20,
        damage: 1000,
        ..Default::default()
    }
}

fn target_hp(world: &openra_sim::world::World, id: u32) -> Option<i32> {
    world.actor(id)?.traits.iter().find_map(|t| {
        if let TraitState::Health { hp } = t { Some(*hp) } else { None }
    })
}

#[test]
fn out_of_range_pushes_move_child_then_fires_after_arrival() {
    let mut world = empty_world(60, 20);
    insert_test_actor(&mut world, make_e1(101, 1, (5, 10), 5000));
    insert_test_actor(&mut world, make_e1(102, 2, (35, 10), 5000));
    // Chebyshev distance = 30 cells, well beyond M1Carbine range = 5.

    let mut atk = AttackActivity::new(102, Armament::new(m1carbine()));

    // Tick 1: out of range — Attack should push a Move child and not fire.
    let mut a = world.actor(101).unwrap().clone();
    let s = atk.tick(&mut a, &mut world);
    assert_eq!(s, ActivityState::Continue, "Attack should continue while chasing");
    assert!(!atk.fired_this_tick(), "should not fire while out of range");
    let child = atk.take_child().expect("expected a Move child to be queued");
    assert_eq!(child.name(), "Move", "child should be a MoveActivity");
    assert_eq!(target_hp(&world, 102), Some(5000), "target HP unchanged");

    // Simulate arrival: teleport attacker to within range (3 cells away).
    if let Some(actor) = world.actor_mut(101) {
        actor.location = Some((32, 10));
        actor.activity = None;
    }
    a.location = Some((32, 10));
    a.activity = None;

    // Drain the cooldown that the Attack ticked on the previous frame
    // (cooldown started at 0, ticked to 0 still, so we're ready). On
    // this next tick we should fire.
    let s = atk.tick(&mut a, &mut world);
    assert_eq!(s, ActivityState::Continue);
    assert!(atk.fired_this_tick(), "should fire now that target is in range");
    assert!(
        atk.take_child().is_none(),
        "no chase child should be queued when in range"
    );
    let hp = target_hp(&world, 102).unwrap_or(0);
    assert!(hp < 5000, "target HP should drop after firing, got {hp}");
}
