//! Bug B regression: a unit transiting an enemy kill zone on a long
//! `move` order must be a NORMAL target — enemies in weapon range
//! should hit it.
//!
//! Background: a unit given a long-distance move order took
//! effectively zero fire while crossing enemy weapon range, even
//! passing point-blank by many hostiles ("sprint-invincibility").
//! Expected: a moving unit is hittable like any other; an enemy whose
//! weapon bears on the corridor draws blood as the unit passes.
//!
//! Runs on the vendored RA mod so units carry real weapon + speed
//! stats; skips gracefully when the submodule isn't present.

use openra_data::oramap::{MapActor, OraMap, PlayerDef};
use openra_data::rules as data_rules;
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::gamerules::GameRules;
use openra_sim::math::{CPos, WAngle, WPos};
use openra_sim::traits::TraitState;
use openra_sim::world::{
    self, insert_test_actor, set_actor_stance, set_test_unpaused, GameOrder, LobbyInfo, SlotInfo,
    World,
};
use std::path::PathBuf;

fn vendor_mod_dir() -> Option<PathBuf> {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let p = PathBuf::from(format!("{manifest}/../vendor/OpenRA/mods/ra"));
    if p.exists() { Some(p) } else { None }
}

fn arena() -> Option<World> {
    let mod_dir = vendor_mod_dir()?;
    let ruleset = data_rules::load_ruleset(&mod_dir).ok()?;
    let rules = GameRules::from_ruleset(&ruleset);

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
            location: (98, 98),
        },
    ];
    let map = OraMap {
        title: "killzone-arena".into(),
        tileset: "TEMPERAT".into(),
        map_size: (100, 100),
        bounds: (0, 0, 100, 100),
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
                name: "P1".into(),
                playable: true,
                owns_world: false,
                non_combatant: false,
                faction: "allies".into(),
                enemies: vec!["P2".into()],
            },
            PlayerDef {
                name: "P2".into(),
                playable: true,
                owns_world: false,
                non_combatant: false,
                faction: "soviet".into(),
                enemies: vec!["P1".into()],
            },
        ],
    };
    let lobby = LobbyInfo {
        starting_cash: 0,
        allow_spectators: true,
        occupied_slots: vec![
            SlotInfo { player_reference: "P1".into(), faction: "allies".into(), is_bot: false, starting_cash: None },
            SlotInfo { player_reference: "P2".into(), faction: "soviet".into(), is_bot: false, starting_cash: None },
        ],
    };
    let mut w = world::build_world(&map, 0, &lobby, Some(rules), 0, false);
    set_test_unpaused(&mut w);
    let strip: Vec<u32> = world::all_actor_ids(&w)
        .into_iter()
        .filter(|&id| matches!(w.actor_kind(id), Some(ActorKind::Mcv) | Some(ActorKind::Spawn)))
        .collect();
    for id in strip {
        world::remove_test_actor(&mut w, id);
    }
    Some(w)
}

fn playable_owner_ids(w: &World) -> (u32, u32) {
    let mut ids: Vec<u32> = w.player_ids().to_vec();
    ids.pop(); // everyone
    let p2 = ids.pop().unwrap();
    let p1 = ids.pop().unwrap();
    (p1, p2)
}

fn make_vehicle(id: u32, owner: u32, at: (i32, i32), hp: i32, ty: &str) -> Actor {
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
        actor_type: Some(ty.into()),
        kills: 0,
        rank: 0,
    }
}

fn make_e1(id: u32, owner: u32, at: (i32, i32), hp: i32) -> Actor {
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
        actor_type: Some("e1".into()),
        kills: 0,
        rank: 0,
    }
}

fn hp_of(w: &World, id: u32) -> Option<i32> {
    w.actor(id)?.traits.iter().find_map(|t| match t {
        TraitState::Health { hp } => Some(*hp),
        _ => None,
    })
}

/// Core sprint-invincibility fix: a unit executing a `Move` activity
/// is a normal combatant — an enemy that is ITSELF moving alongside it
/// (within weapon range) must draw blood. Previously only IDLE units
/// auto-engaged, so a unit on a long `move` order crossed enemy weapon
/// range untouched whenever the defenders were themselves in motion.
///
/// Two infantry start one cell apart and both receive a long move
/// order east at identical speed — they glide down parallel rows
/// staying permanently inside M1Carbine range. Both are in
/// `Activity::Move` the whole way, so this exercises moving-shooter
/// AND moving-target at once. With the fix each takes fire; without
/// it neither does.
#[test]
fn moving_units_in_range_exchange_fire() {
    let Some(mut world) = arena() else {
        eprintln!("skip: vendored RA mod dir not present");
        return;
    };
    let (p1, p2) = playable_owner_ids(&world);

    // Agent rifleman on row 30, enemy rifleman one cell south on row
    // 31 — one cell apart, well inside M1Carbine range (5 cells).
    insert_test_actor(&mut world, make_e1(201, p1, (5, 30), 5000));
    insert_test_actor(&mut world, make_e1(301, p2, (5, 31), 5000));
    // Default stance (Defend) auto-fires on in-range enemies.
    set_actor_stance(&mut world, 201, 2);
    set_actor_stance(&mut world, 301, 2);

    let agent_start = hp_of(&world, 201).unwrap();
    let enemy_start = hp_of(&world, 301).unwrap();

    // Both ordered on a long parallel sprint east — same speed, so
    // they stay locked one cell apart for the whole run.
    world.process_frame(&[
        GameOrder {
            order_string: "Move".into(),
            subject_id: Some(201),
            target_string: Some("90,30".into()),
            extra_data: None,
        },
        GameOrder {
            order_string: "Move".into(),
            subject_id: Some(301),
            target_string: Some("90,31".into()),
            extra_data: None,
        },
    ]);
    for _ in 0..900 {
        world.process_frame(&[]);
        // Stop once either side is destroyed or both have arrived.
        let agent_done = world
            .actor(201)
            .map_or(true, |a| a.location.map_or(true, |l| l.0 >= 88));
        if world.actor(201).is_none() || world.actor(301).is_none() || agent_done {
            break;
        }
    }

    let agent_dmg = agent_start - hp_of(&world, 201).unwrap_or(0);
    let enemy_dmg = enemy_start - hp_of(&world, 301).unwrap_or(0);
    assert!(
        agent_dmg > 0,
        "a unit on a long `move` order took ZERO fire while gliding \
         one cell from an enemy for the whole run — sprint-\
         invincibility (agent damage {agent_dmg})"
    );
    assert!(
        enemy_dmg > 0,
        "a moving unit failed to OPPORTUNISTICALLY fire at an enemy \
         locked one cell away the whole run (enemy damage {enemy_dmg})"
    );
}

/// A fast jeep ordered to sprint past a line of STATIC enemy infantry
/// must take meaningful fire — pins the already-working idle-defender
/// path so a regression there is caught too.
#[test]
fn fast_unit_takes_fire_sprinting_kill_zone() {
    let Some(mut world) = arena() else {
        eprintln!("skip: vendored RA mod dir not present");
        return;
    };
    let (p1, p2) = playable_owner_ids(&world);

    insert_test_actor(&mut world, make_vehicle(201, p1, (5, 30), 15000, "jeep"));
    let mut eid = 300;
    let mut n_enemies = 0;
    for cx in (12..=78).step_by(5) {
        insert_test_actor(&mut world, make_e1(eid, p2, (cx, 30), 5000));
        set_actor_stance(&mut world, eid, 2);
        n_enemies += 1;
        eid += 1;
    }

    let start_hp = hp_of(&world, 201).unwrap();
    world.process_frame(&[GameOrder {
        order_string: "Move".into(),
        subject_id: Some(201),
        target_string: Some("95,30".into()),
        extra_data: None,
    }]);
    for _ in 0..900 {
        world.process_frame(&[]);
        if world.actor(201).is_none() {
            break;
        }
        if let Some(loc) = world.actor(201).and_then(|a| a.location) {
            if loc.0 >= 93 {
                break;
            }
        }
    }

    let damage = start_hp - hp_of(&world, 201).unwrap_or(0);
    let floor = n_enemies * 200;
    assert!(
        damage >= floor,
        "fast jeep sprinted past {n_enemies} enemy riflemen and took \
         only {damage} damage; expected at least {floor}."
    );
}
