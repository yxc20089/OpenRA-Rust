//! Air-unit MVP acceptance: a `heli` (helicopter) must move over
//! ground obstacles that would block a tank, and must engage and kill
//! an enemy vehicle parked behind that obstacle.
//!
//! Two tests, both built on the no-vendor defaults() ruleset so they
//! never depend on the submodule:
//!
//! 1. `heli_flies_over_impassable_terrain`: place a heli at (5, 10),
//!    paint an impassable wall across column x=10 from y=5..=15, and
//!    issue `move_units` to (20, 10). The heli must traverse the wall
//!    (straight-line air path) and arrive on the far side.
//!
//! 2. `heli_kills_vehicle_behind_obstacle_wall`: same wall, but now
//!    park an enemy `2tnk` at (20, 10) and issue `attack_unit` from
//!    the heli. The heli must close range over the wall and the tank
//!    must drop to 0 HP within the test budget.
//!
//! These pin the *ground-obstacle bypass* — the MVP win condition.
//! Refuelling / landing on hpad is deliberately out of scope.

use openra_data::oramap::{MapActor, OraMap, PlayerDef};
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::math::{CPos, WAngle, WPos};
use openra_sim::terrain::COST_IMPASSABLE;
use openra_sim::traits::TraitState;
use openra_sim::world::{
    self, insert_test_actor, set_actor_stance, set_test_unpaused, GameOrder, LobbyInfo, SlotInfo,
    World,
};

fn build_arena(seed: i32) -> World {
    let map = OraMap {
        title: "aircraft-arena".into(),
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
        allow_spectators: false,
        occupied_slots: vec![
            SlotInfo { player_reference: "P1".into(), faction: "allies".into(), is_bot: false },
            SlotInfo { player_reference: "P2".into(), faction: "soviet".into(), is_bot: false },
        ],
    };
    // Use defaults so we don't depend on the vendor dir.
    let mut world = world::build_world(&map, seed, &lobby, None, 0, false);
    set_test_unpaused(&mut world);
    // Strip auto-spawned MCVs / spawn beacons so we start with a clean grid.
    let strip: Vec<u32> = world::all_actor_ids(&world)
        .into_iter()
        .filter(|&id| matches!(world.actor_kind(id), Some(ActorKind::Mcv) | Some(ActorKind::Spawn)))
        .collect();
    for id in strip {
        world::remove_test_actor(&mut world, id);
    }
    world
}

fn playable_owner_ids(w: &World) -> (u32, u32) {
    let mut ids: Vec<u32> = w.player_ids().to_vec();
    ids.pop(); // everyone
    let p2 = ids.pop().unwrap();
    let p1 = ids.pop().unwrap();
    (p1, p2)
}

fn make_heli(id: u32, owner: u32, at: (i32, i32), hp: i32) -> Actor {
    let cell = CPos::new(at.0, at.1);
    let center = WPos::new(at.0 * 1024 + 512, at.1 * 1024 + 512, 0);
    Actor {
        id,
        kind: ActorKind::Aircraft,
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
        actor_type: Some("heli".into()),
        kills: 0,
        rank: 0,
    }
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
        actor_type: Some("2tnk".into()),
        kills: 0,
        rank: 0,
    }
}

fn paint_wall(world: &mut World, x: i32, y_lo: i32, y_hi: i32) {
    for y in y_lo..=y_hi {
        world.terrain.set_cost(x, y, COST_IMPASSABLE);
    }
}

fn loc_of(w: &World, id: u32) -> Option<(i32, i32)> {
    w.actor(id).and_then(|a| a.location)
}

fn hp_of(w: &World, id: u32) -> Option<i32> {
    w.actor(id)?.traits.iter().find_map(|t| match t {
        TraitState::Health { hp } => Some(*hp),
        _ => None,
    })
}

/// A heli ordered to move past a wall of impassable terrain that
/// completely blocks the equivalent ground path must arrive on the
/// far side. A ground unit issued the same order would either A*
/// around the wall (and fail if it's tall enough) or stall — the
/// heli's `straight_line_path` should cross the wall directly.
#[test]
fn heli_flies_over_impassable_terrain() {
    let mut world = build_arena(7);
    let (p1, _p2) = playable_owner_ids(&world);

    // Wall from (10, 0) all the way down to (10, 39) — no ground path
    // around it in this arena.
    paint_wall(&mut world, 10, 0, 39);

    insert_test_actor(&mut world, make_heli(201, p1, (5, 10), 100000));
    let start = loc_of(&world, 201).unwrap();
    assert_eq!(start, (5, 10), "heli starts at (5, 10)");

    world.process_frame(&[GameOrder {
        order_string: "Move".into(),
        subject_id: Some(201),
        target_string: Some("20,10".into()),
        extra_data: None,
    }]);

    // Heli speed 128 world u/tick; distance ~15 cells × 1024 ≈ 15360
    // world units ⇒ ~120 ticks; give a comfortable budget.
    for _ in 0..400 {
        world.process_frame(&[]);
        if let Some(loc) = loc_of(&world, 201) {
            if loc.0 >= 20 {
                break;
            }
        }
    }

    let end = loc_of(&world, 201).expect("heli should still be alive");
    assert!(
        end.0 >= 20,
        "heli failed to fly over the impassable wall: arrived at {end:?} \
         (wall at x=10, target (20,10))"
    );
    // Sanity: a ground unit cannot path through that wall — confirm
    // the wall really IS impassable (regression guard against a future
    // refactor that silently makes set_cost into a no-op).
    assert!(
        !world.terrain.is_terrain_passable(10, 10),
        "test scaffolding broken: wall cell (10,10) is still passable"
    );
}

/// Heli + attack_unit on an enemy vehicle behind the same wall must
/// kill it. The heli has to fly over the wall to close range, then
/// the auto-engage / attack activity must fire its HellfireAG and
/// deplete the tank's HP.
#[test]
fn heli_kills_vehicle_behind_obstacle_wall() {
    let mut world = build_arena(11);
    let (p1, p2) = playable_owner_ids(&world);

    paint_wall(&mut world, 15, 0, 39);

    let heli_id = 201;
    let tank_id = 301;
    insert_test_actor(&mut world, make_heli(heli_id, p1, (5, 20), 100000));
    insert_test_actor(&mut world, make_tank(tank_id, p2, (25, 20), 26000));
    // Heli AttackAnything so it auto-closes if attack_unit handling
    // misbehaves; not strictly required since we issue the explicit
    // order below, but cheap insurance.
    set_actor_stance(&mut world, heli_id, 3);
    set_actor_stance(&mut world, tank_id, 0); // tank holds fire (no AA in defaults)

    let start_hp = hp_of(&world, tank_id).expect("tank has Health");

    world.process_frame(&[GameOrder {
        order_string: "Attack".into(),
        subject_id: Some(heli_id),
        target_string: None,
        extra_data: Some(tank_id),
    }]);

    // 600 ticks ≈ 6.6 decision turns at 90 ticks/turn. With HellfireAG
    // (6000 base × 90% vs Heavy ⇒ 5400/shot, burst 2, reload 34) the
    // 26k-HP 2tnk should drop in well under a minute of flight.
    let mut killed = false;
    for _ in 0..900 {
        world.process_frame(&[]);
        if world.actor(tank_id).is_none() {
            killed = true;
            break;
        }
    }

    assert!(
        killed,
        "heli failed to kill the vehicle behind the impassable wall \
         within budget (tank HP {:?} of start {})",
        hp_of(&world, tank_id),
        start_hp
    );
}

/// Minimum-bar sanity: yak and mig are present as Aircraft actor
/// entries with non-zero speed + a weapon, even if they don't get
/// fancier flight behaviour than `heli` in this MVP. Covers the
/// "yak/mig as actor entries only" deliverable.
#[test]
fn yak_and_mig_are_buildable_aircraft_entries() {
    let world = build_arena(0);
    for name in &["yak", "mig", "heli", "hind"] {
        let stats = world
            .rules
            .actor(name)
            .unwrap_or_else(|| panic!("{name} missing from defaults() rules"));
        assert_eq!(
            stats.kind,
            ActorKind::Aircraft,
            "{name} should be ActorKind::Aircraft"
        );
        assert!(
            stats.speed > 0,
            "{name} should have non-zero speed (got {})",
            stats.speed
        );
        assert!(
            !stats.weapons.is_empty(),
            "{name} should carry at least one armament"
        );
    }
    // hpad must be a placeable building that produces the Aircraft queue.
    let hpad = world.rules.actor("hpad").expect("hpad missing from defaults");
    assert!(hpad.is_building, "hpad is a building");
    assert!(
        hpad.footprint.0 >= 2 && hpad.footprint.1 >= 2,
        "hpad has a real footprint (got {:?})",
        hpad.footprint
    );
}
