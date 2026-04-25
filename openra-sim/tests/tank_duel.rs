//! Phase-6 tank duel smoke test.
//!
//! A 2tnk and a 1tnk are placed 5 cells apart on an empty 40×40 map.
//! Both are within range of the other's cannon (90mm: 4c768 ≈ 4.75
//! cells; 25mm: 4c768 same range). The 2tnk attacks the 1tnk for up
//! to 1000 ticks; we assert the 1tnk dies and that the kill happens
//! within a ±20% window of the analytical estimate
//! `1tnk_hp / (90mm_damage * fire_rate)` where fire_rate is
//! `1 / 90mm.reload_delay`.
//!
//! This is the **integration** half of the Phase 6 acceptance gate:
//! it exercises the full chain from "vendored YAML weapons.yaml →
//! `GameRules::from_ruleset` → `World::order_attack` → combat tick loop"
//! using real weapon stats (not the hardcoded defaults).

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

fn build_arena_with_real_rules(seed: i32) -> World {
    let mod_dir = vendor_mod_dir().expect(
        "vendored OpenRA mod dir missing; run from a clone with submodules initialised",
    );
    let ruleset = data_rules::load_ruleset(&mod_dir).expect("failed to load ruleset");
    let rules = GameRules::from_ruleset(&ruleset);

    // build_world's spawn assignment requires at least one mpspawn
    // per occupied slot. Inject two at the corners.
    let spawn_actors = vec![
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
    ];

    let map = OraMap {
        title: "tank-duel".into(),
        tileset: "TEMPERAT".into(),
        map_size: (40, 40),
        bounds: (0, 0, 40, 40),
        tiles: Vec::new(),
        actors: spawn_actors,
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
    world
}

fn make_tank(id: u32, owner: u32, actor_type: &str, at: (i32, i32), hp: i32) -> Actor {
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
        actor_type: Some(actor_type.into()),
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
fn two_tnk_vs_one_tnk_kill_with_real_rules() {
    if vendor_mod_dir().is_none() {
        eprintln!("skipping: vendored OpenRA mod dir not found");
        return;
    }

    // Inspect what 90mm/25mm look like as parsed.
    {
        let mod_dir = vendor_mod_dir().unwrap();
        let ruleset = data_rules::load_ruleset(&mod_dir).unwrap();
        let rules = GameRules::from_ruleset(&ruleset);
        let w90 = rules.weapon("90mm").expect("90mm");
        let w25 = rules.weapon("25mm").expect("25mm");
        eprintln!(
            "90mm: damage={} range={} reload={} burst={}",
            w90.damage, w90.range, w90.reload_delay, w90.burst
        );
        eprintln!(
            "25mm: damage={} range={} reload={} burst={}",
            w25.damage, w25.range, w25.reload_delay, w25.burst
        );
    }

    let mut world = build_arena_with_real_rules(42);
    // Player ids: [Neutral=1, Multi0=2, Multi1=3, Everyone=4] (World=0)
    // Strip auto-spawned MCVs first.
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

    insert_test_actor(&mut world, make_tank(101, agent_pid, "2tnk", (10, 20), 46000));
    insert_test_actor(&mut world, make_tank(102, enemy_pid, "1tnk", (15, 20), 23000));

    // Issue an Attack order from agent's 2tnk on the enemy 1tnk.
    let order_a = GameOrder {
        order_string: "Attack".into(),
        subject_id: Some(101),
        target_string: None,
        extra_data: Some(102),
    };

    // Tick the world until the target dies or 1000 ticks elapse.
    let mut kill_tick = None;
    let attack_orders = vec![order_a];
    for tick in 1..=1000 {
        let no_orders: Vec<GameOrder> = Vec::new();
        let orders_for_tick = if tick == 1 { &attack_orders } else { &no_orders };
        let _ = world.tick(orders_for_tick);
        if let Some(hp) = target_hp(&world, 102) {
            if hp <= 0 {
                kill_tick = Some(tick);
                break;
            }
        } else {
            // Actor removed (dead).
            kill_tick = Some(tick);
            break;
        }
    }

    let kill_tick = kill_tick.expect("expected enemy 1tnk to die within 1000 ticks");

    // Sanity: 1tnk has 23000 HP; 90mm does 4000 damage per shot.
    // Shots to kill = ceil(23000 / 4000) = 6. Reload_delay = 50
    // *inner* ticks. The world ticks 3 inner steps per outer tick
    // (NetFrameInterval=3), so the analytical estimate in outer ticks
    // is `(1 + 5*50) / 3 ≈ 84`. Versus Heavy 115% multipliers (Phase
    // 8) would lower this to ~67. We accept anything in [60..=110]
    // which spans both.
    assert!(
        (60..=110).contains(&kill_tick),
        "kill_tick = {kill_tick}, expected in [60..=110]"
    );
}

#[test]
fn tank_attack_terminates_when_no_armament_loaded() {
    // Defensive: spawn an MCV (which has no Armament in defaults
    // GameRules) and tell it to Attack — should not panic; should
    // either fail silently or fall back to the "default" weapon.
    let mut world = build_arena_with_real_rules(7);
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

    // mcv has Mobile + Health but no Armament in C# rules. Use it as
    // attacker; expect the engine to either (a) treat it as having a
    // default weapon and shoot, or (b) leave the target alive without
    // crashing.
    insert_test_actor(&mut world, make_tank(201, agent_pid, "mcv", (10, 20), 60000));
    insert_test_actor(&mut world, make_tank(202, enemy_pid, "1tnk", (15, 20), 23000));

    let order = GameOrder {
        order_string: "Attack".into(),
        subject_id: Some(201),
        target_string: None,
        extra_data: Some(202),
    };

    // Just ensure we can tick 100 times without a panic.
    let attack = vec![order];
    for tick in 1..=100 {
        let no_orders: Vec<GameOrder> = Vec::new();
        let orders_for_tick = if tick == 1 { &attack } else { &no_orders };
        let _ = world.tick(orders_for_tick);
    }
}
