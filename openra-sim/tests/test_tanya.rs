//! Tanya combat acceptance: a single Tanya commando must defeat a
//! cluster of four `e1` riflemen in a stand-up fight.
//!
//! Tanya is the Allied hero infantry — high HP, fast-moving, with a
//! strong personal sidearm. The actor entry lives in
//! `gamerules::GameRules::defaults()` (alongside e1/e3/dog/etc.); the
//! sidearm weapon `TanyaPistol` lives in the same table. This test
//! does NOT need the vendored OpenRA YAML — it runs on the defaults
//! ruleset using activity-stack-driven combat (same idiom as
//! `combat_one_v_one.rs`), so it works in CI without submodules.
//!
//! Setup: 1 tanya at (10,10) vs 4 e1 enemies at (12..15, 10), all in
//! M1Carbine range (5 cells). Every actor attacks tanya, and tanya
//! attacks the lowest-id alive enemy each tick. Expectation: tanya
//! outpaces the e1's burst output (5x M1Carbine damage, 2x reload
//! rate) and is still alive when the last e1 dies.

use openra_data::oramap::{OraMap, PlayerDef};
use openra_data::rules::{WDist, WeaponStats};
use openra_sim::activities::AttackActivity;
use openra_sim::activity::ActivityStack;
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::gamerules::GameRules;
use openra_sim::traits::{Armament, TraitState};
use openra_sim::world::{self, insert_test_actor, set_test_unpaused, LobbyInfo};

fn empty_world(w: i32, h: i32) -> openra_sim::world::World {
    let map = OraMap {
        title: "tanya-arena".into(),
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

fn tanya_pistol() -> WeaponStats {
    WeaponStats {
        name: "TanyaPistol".into(),
        range: WDist::from_cells(5),
        reload_delay: 10,
        damage: 5000,
        ..Default::default()
    }
}

fn m1_carbine() -> WeaponStats {
    WeaponStats {
        name: "M1Carbine".into(),
        range: WDist::from_cells(5),
        reload_delay: 20,
        damage: 1000,
        ..Default::default()
    }
}

fn hp_of(world: &openra_sim::world::World, id: u32) -> Option<i32> {
    world.actor(id)?.traits.iter().find_map(|t| {
        if let TraitState::Health { hp } = t { Some(*hp) } else { None }
    })
}

fn is_dead(world: &openra_sim::world::World, id: u32) -> bool {
    match hp_of(world, id) {
        Some(hp) => hp <= 0,
        None => true,
    }
}

/// Acceptance: 1 tanya beats 4 e1 at close range, surviving the
/// exchange. Confirms Tanya's actor-table stats produce the intended
/// "hero" combat outcome.
#[test]
fn tanya_beats_four_e1_riflemen() {
    let mut world = empty_world(40, 40);

    // Tanya at (10,10), owner 1. HP from defaults (150000).
    let tanya_id: u32 = 100;
    let tanya_hp = 150_000;
    insert_test_actor(
        &mut world,
        make_infantry(tanya_id, 1, "tanya", (10, 10), tanya_hp),
    );

    // Four e1 riflemen at (12,10), (13,10), (14,10), (15,10), owner 2.
    // Within M1Carbine range (5 cells from (10,10) → cells 11..=15 all
    // reach). e1 HP from defaults (50000).
    let e1_ids: Vec<u32> = (0..4).map(|i| 200 + i as u32).collect();
    for (i, &id) in e1_ids.iter().enumerate() {
        let cell = (12 + i as i32, 10);
        insert_test_actor(&mut world, make_infantry(id, 2, "e1", cell, 50_000));
    }

    // Each combatant gets its own AttackActivity targeting the foe.
    // Tanya re-acquires the lowest-id alive e1 if her current target
    // dies (a tanya reload of 10 ticks means she'd otherwise idle out
    // the back half of the fight after killing #1 at ~tick 100).
    let mut tanya_target = e1_ids[0];
    let mut tanya_stack = ActivityStack::new();
    tanya_stack.push(Box::new(AttackActivity::new(
        tanya_target,
        Armament::new(tanya_pistol()),
    )));

    let mut e1_stacks: Vec<ActivityStack> = e1_ids
        .iter()
        .map(|_| {
            let mut s = ActivityStack::new();
            s.push(Box::new(AttackActivity::new(
                tanya_id,
                Armament::new(m1_carbine()),
            )));
            s
        })
        .collect();

    let max_ticks = 1000;
    let mut tanya_clone = world.actor(tanya_id).unwrap().clone();
    let mut e1_clones: Vec<Actor> = e1_ids
        .iter()
        .map(|&id| world.actor(id).unwrap().clone())
        .collect();

    let mut final_tick = max_ticks;
    for tick in 1..=max_ticks {
        // Tanya fires (only if she's still alive).
        if !is_dead(&world, tanya_id) {
            // Re-target if current target dead.
            if is_dead(&world, tanya_target) {
                if let Some(&next) = e1_ids.iter().find(|&&id| !is_dead(&world, id)) {
                    tanya_target = next;
                    tanya_stack = ActivityStack::new();
                    tanya_stack.push(Box::new(AttackActivity::new(
                        tanya_target,
                        Armament::new(tanya_pistol()),
                    )));
                }
            }
            let _ = tanya_stack.run_top(&mut tanya_clone, &mut world);
        }

        // Each living e1 fires.
        for (i, &id) in e1_ids.iter().enumerate() {
            if is_dead(&world, id) {
                continue;
            }
            let _ = e1_stacks[i].run_top(&mut e1_clones[i], &mut world);
        }

        let tanya_dead = is_dead(&world, tanya_id);
        let all_e1_dead = e1_ids.iter().all(|&id| is_dead(&world, id));
        if tanya_dead || all_e1_dead {
            final_tick = tick;
            break;
        }
    }

    let tanya_hp_final = hp_of(&world, tanya_id).unwrap_or(0);
    let alive_e1: Vec<u32> = e1_ids
        .iter()
        .copied()
        .filter(|&id| !is_dead(&world, id))
        .collect();
    eprintln!(
        "tanya HP final = {tanya_hp_final}/{tanya_hp}, alive e1 = {:?}, end tick = {final_tick}",
        alive_e1
    );

    assert!(
        tanya_hp_final > 0,
        "tanya should survive the 4-e1 rush (HP went to {tanya_hp_final})"
    );
    assert!(
        alive_e1.is_empty(),
        "all e1 should be dead; still alive = {:?}",
        alive_e1
    );
}

/// Property pin: Tanya's actor-table stats match the spec — high HP
/// (~3x e1), fast speed (~1.5x e1), strong sidearm bound, on the
/// Infantry kind. Pins the chosen numbers so a future drift breaks
/// loud.
#[test]
fn tanya_actor_stats_match_spec() {
    let rules = GameRules::defaults();
    let e1 = rules.actor("e1").expect("e1 in defaults");
    let tanya = rules.actor("tanya").expect("tanya in defaults");

    assert_eq!(tanya.kind, ActorKind::Infantry, "tanya is Infantry");
    assert!(!tanya.is_building, "tanya is not a building");
    assert!(
        tanya.hp >= 3 * e1.hp,
        "tanya HP ({}) should be ≥ 3x e1 HP ({})",
        tanya.hp,
        e1.hp
    );
    assert!(
        tanya.speed * 10 >= e1.speed * 14,
        "tanya speed ({}) should be ≥ 1.4x e1 speed ({})",
        tanya.speed,
        e1.speed
    );
    assert_eq!(
        tanya.weapons,
        vec!["TanyaPistol".to_string()],
        "tanya should bind TanyaPistol"
    );

    let pistol = rules.weapon("TanyaPistol").expect("TanyaPistol weapon");
    assert!(pistol.damage >= 5000, "TanyaPistol damage ≥ 5000");
    assert!(pistol.range >= 4 * 1024, "TanyaPistol range ≥ 4 cells");
    assert!(
        pistol.reload_delay <= 15,
        "TanyaPistol reload_delay ≤ 15 (fast)"
    );
}
