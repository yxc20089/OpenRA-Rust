//! Phase-3 combat smoke test: 1v1 e1 vs e1, verify B dies on schedule.
//!
//! Two riflemen 5 cells apart (within M1Carbine range = 5c0). A
//! attacks B for 200 ticks. We expect B to be dead and for the kill
//! tick to fall within ±5% of the analytical estimate
//! `health / (damage * fire_rate)` where
//! `fire_rate = 1 / reload_delay`.

use openra_data::oramap::{OraMap, PlayerDef};
use openra_data::rules::{WDist, WeaponStats};
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::activities::AttackActivity;
use openra_sim::activity::{ActivityStack, ActivityState};
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
fn one_v_one_kill_within_5pct_of_expected_tick() {
    let mut world = empty_world(40, 40);
    // Use distinct owner ids to avoid friendly-fire confusion.
    insert_test_actor(&mut world, make_e1(101, 1, (5, 10), 5000));
    insert_test_actor(&mut world, make_e1(102, 2, (10, 10), 5000));

    let mut stack = ActivityStack::new();
    let weapon = m1carbine();
    let expected_shots = 5; // 5000 / 1000
    // First shot fires on tick 1 (cooldown starts at 0). Subsequent
    // shots wait `reload_delay` ticks. So total ticks for kill =
    // 1 + (expected_shots - 1) * reload_delay = 1 + 4 * 20 = 81.
    let expected_kill_tick = 1 + (expected_shots - 1) * weapon.reload_delay;

    stack.push(Box::new(AttackActivity::new(102, Armament::new(weapon))));

    let mut a = world.actor(101).unwrap().clone();
    let mut kill_tick = None;
    for tick in 1..=200 {
        let s = stack.run_top(&mut a, &mut world);
        // If the activity returned Done, the target is gone.
        if matches!(s, ActivityState::Done) {
            kill_tick = Some(tick);
            break;
        }
        if let Some(hp) = target_hp(&world, 102)
            && hp <= 0
            && kill_tick.is_none()
        {
            kill_tick = Some(tick);
            break;
        }
    }

    let kill_tick = kill_tick.expect("expected target to be killed within 200 ticks");

    // Target should be dead.
    let hp = target_hp(&world, 102).unwrap_or(0);
    assert!(hp <= 0, "target HP = {hp}, expected ≤ 0");

    // Kill timing within ±5% of analytical estimate.
    let lower = (expected_kill_tick * 95) / 100;
    let upper = (expected_kill_tick * 105) / 100 + 1; // round up
    assert!(
        (lower..=upper).contains(&kill_tick),
        "kill_tick = {kill_tick}, expected in [{lower}..={upper}] (predicted {expected_kill_tick})"
    );
}
