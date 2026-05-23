//! Naval MVP — water terrain + Ship locomotor + shore strike.
//!
//! Pins the three invariants the naval feature carries:
//!
//! 1. A water cell is GROUND-IMPASSABLE — a ground unit (e1/2tnk)
//!    given a Move order across a water band cannot enter any water
//!    cell, and `find_path` rejects routes that would cross water.
//! 2. A water cell is SHIP-PASSABLE and *only* water is — a `dd`
//!    destroyer ordered to a water destination slides along the water
//!    band, and a ship ordered onto land never leaves the water.
//! 3. A `dd` with weapon and Mobile.Speed can engage a ground target
//!    standing on the shore (Activity::Attack + chase + damage).
//!
//! The test runs entirely on `GameRules::defaults()` so it never
//! touches the vendored RA mod (the worktree lacks the submodule).
//! It manually constructs a small World with hand-built actors and
//! stamps a water column into the terrain. The default `dd` entry
//! ships with `kind = Ship, hp = 100000, cost = 1000` but `speed = 0`
//! and no weapons; the test attaches a Mobile trait with a non-zero
//! speed and exercises the default fallback weapon (damage 100,
//! range 5) — sufficient to verify that the engine's attack pipeline
//! sees a Ship attacker correctly.

use openra_data::oramap::{MapActor, OraMap, PlayerDef};
use openra_sim::actor::{Actor, ActorKind, Activity};
use openra_sim::gamerules::GameRules;
use openra_sim::math::{CPos, WPos};
use openra_sim::pathfinder;
use openra_sim::traits::TraitState;
use openra_sim::world::{
    self, insert_test_actor, set_actor_stance, set_test_unpaused, GameOrder, LobbyInfo, SlotInfo,
    World,
};

/// Build a minimal arena world (60×30) with two playable players.
/// Strips auto-spawned MCVs + spawn beacons before returning so the
/// caller's hand-placed actors are the only mobile units present.
fn arena() -> World {
    let spawn_actors = vec![
        MapActor {
            id: "mpspawn1".into(),
            actor_type: "mpspawn".into(),
            owner: "Neutral".into(),
            location: (2, 2),
        },
        MapActor {
            id: "mpspawn2".into(),
            actor_type: "mpspawn".into(),
            owner: "Neutral".into(),
            location: (55, 25),
        },
    ];
    let map = OraMap {
        title: "naval-arena".into(),
        tileset: "TEMPERAT".into(),
        map_size: (60, 30),
        bounds: (0, 0, 60, 30),
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
            SlotInfo {
                player_reference: "P1".into(),
                faction: "allies".into(),
                is_bot: false,
            },
            SlotInfo {
                player_reference: "P2".into(),
                faction: "soviet".into(),
                is_bot: false,
            },
        ],
    };
    let mut w = world::build_world(&map, 0, &lobby, Some(GameRules::defaults()), 0, false);
    set_test_unpaused(&mut w);
    let strip: Vec<u32> = world::all_actor_ids(&w)
        .into_iter()
        .filter(|&id| {
            matches!(
                w.actor_kind(id),
                Some(ActorKind::Mcv) | Some(ActorKind::Spawn)
            )
        })
        .collect();
    for id in strip {
        world::remove_test_actor(&mut w, id);
    }
    w
}

/// Stamp a water column at x = `col_x` spanning the full map height.
/// Mirrors what a scenario YAML's `water_rect: [col_x, 0, 1, height]`
/// would produce.
fn stamp_water_column(world: &mut World, col_x: i32) {
    for y in 0..world.terrain.height {
        world.terrain.set_water(col_x, y, true);
    }
}

/// Stamp a water band at columns `x_lo..=x_hi` (inclusive) spanning
/// the map height. Used for the dd traversal test (needs > 1 cell so
/// the dd has room to lerp between water cells).
fn stamp_water_band(world: &mut World, x_lo: i32, x_hi: i32) {
    for y in 0..world.terrain.height {
        for x in x_lo..=x_hi {
            world.terrain.set_water(x, y, true);
        }
    }
}

fn make_ship(id: u32, owner: u32, at: (i32, i32), ty: &str, _speed: i32) -> Actor {
    let cell = CPos::new(at.0, at.1);
    let center = WPos::new(at.0 * 1024 + 512, at.1 * 1024 + 512, 0);
    Actor {
        id,
        kind: ActorKind::Ship,
        owner_id: Some(owner),
        location: Some(at),
        traits: vec![
            TraitState::BodyOrientation { quantized_facings: 32 },
            TraitState::Mobile {
                facing: 0,
                from_cell: cell,
                to_cell: cell,
                center_position: center,
            },
            TraitState::Health { hp: 100000 },
        ],
        activity: None,
        actor_type: Some(ty.into()),
        kills: 0,
        rank: 0,
    }
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
                facing: 0,
                from_cell: cell,
                to_cell: cell,
                center_position: center,
            },
            TraitState::Health { hp: 5000 },
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

fn playable_owner_ids(w: &World) -> (u32, u32) {
    // build_world returns [Neutral, P1, P2, Everyone] → P1, P2 are
    // indices 1 and 2.
    let ids: Vec<u32> = w.player_ids().to_vec();
    (ids[1], ids[2])
}

// ────────────────────────────────────────────────────────────────────
// 1. Pathfinder pins
// ────────────────────────────────────────────────────────────────────

#[test]
fn pathfinder_ground_cannot_cross_water_column() {
    let mut w = arena();
    // Full-height water column at x=30 splits the map in two.
    stamp_water_column(&mut w, 30);
    // Ground pathfind from (5,15) to (55,15) is impossible — the wall
    // spans the whole height with no detour.
    let path = pathfinder::find_path(&w.terrain, (5, 15), (55, 15), None);
    assert!(
        path.is_none(),
        "ground unit found a path crossing a full-height water wall: {path:?}"
    );
}

#[test]
fn pathfinder_ship_can_only_path_on_water() {
    let mut w = arena();
    stamp_water_column(&mut w, 30);
    // Ship pathfind from (30, 2) to (30, 25) along the water column.
    let path = pathfinder::find_path_for_kind(&w.terrain, (30, 2), (30, 25), None, true)
        .expect("ship path along water column");
    assert!(
        path.iter().all(|&(x, _)| x == 30),
        "ship pathfinder stepped off water: {path:?}"
    );
    // A ship cannot path through dry land (starting on water (30,2)
    // toward a land cell (40,15) — destination is not water so it
    // fails as a ship destination).
    let bad = pathfinder::find_path_for_kind(&w.terrain, (30, 2), (40, 15), None, true);
    assert!(
        bad.is_none(),
        "ship pathfinder produced a route onto land: {bad:?}"
    );
}

// ────────────────────────────────────────────────────────────────────
// 2. Movement / order_move pins
// ────────────────────────────────────────────────────────────────────

#[test]
fn ground_unit_move_order_stops_at_shore() {
    let mut w = arena();
    stamp_water_column(&mut w, 30);
    let (p1, _) = playable_owner_ids(&w);
    insert_test_actor(&mut w, make_e1(101, p1, (5, 15)));
    // Force speed: defaults give e1 speed 43. Order it east.
    w.process_frame(&[GameOrder {
        order_string: "Move".into(),
        subject_id: Some(101),
        target_string: Some("55,15".into()),
        extra_data: None,
    }]);
    // Run a long time.
    for _ in 0..400 {
        w.process_frame(&[]);
    }
    // The e1 must never have entered a water cell.
    let final_loc = w.actor(101).and_then(|a| a.location).expect("e1 alive");
    assert!(
        !w.terrain.is_water(final_loc.0, final_loc.1),
        "e1 ended up in a water cell at {final_loc:?}"
    );
    // It can't have crossed to x>=31 either (water at x=30 spans the
    // whole height, no detour exists; pathfinder should refuse).
    assert!(
        final_loc.0 < 30,
        "e1 crossed a full-height water wall — landed at {final_loc:?}"
    );
}

#[test]
fn dd_ship_moves_along_water_band() {
    let mut w = arena();
    // 2-cell-wide water band at x=29..=30 spans the height.
    stamp_water_band(&mut w, 29, 30);
    let (p1, _) = playable_owner_ids(&w);
    // dd uses defaults' speed (92 u/tick after the naval fallback
    // patch in `actor_speed`). Hand-built actor — `insert_test_actor`
    // marks the start cell as the dd's occupant.
    let dd = make_ship(201, p1, (30, 5), "dd", 0);
    insert_test_actor(&mut w, dd);
    // Order the dd south down the water band.
    w.process_frame(&[GameOrder {
        order_string: "Move".into(),
        subject_id: Some(201),
        target_string: Some("30,24".into()),
        extra_data: None,
    }]);
    let start = w.actor(201).and_then(|a| a.location).unwrap();
    for _ in 0..600 {
        w.process_frame(&[]);
        if let Some(loc) = w.actor(201).and_then(|a| a.location) {
            assert!(
                w.terrain.is_water(loc.0, loc.1),
                "dd stepped onto a non-water cell at {loc:?}"
            );
            assert!(
                (29..=30).contains(&loc.0),
                "dd left the water band x=29..30 at {loc:?}"
            );
        } else {
            panic!("dd was destroyed during move test");
        }
    }
    // It MUST have advanced south — sprinting along ~20 cells of
    // water at ~92 u/tick fits inside 600 ticks several times over.
    let end = w.actor(201).and_then(|a| a.location).unwrap();
    assert!(
        end.1 > start.1 + 5,
        "dd never made meaningful progress along the water band: start {start:?} end {end:?}"
    );
    // Sanity — the pathfinder still reports a valid water path.
    let _ = pathfinder::find_path_for_kind(&w.terrain, (30, 5), (30, 24), Some(201), true)
        .expect("ship path along band must exist");
}

// ────────────────────────────────────────────────────────────────────
// 3. Attack pin — dd attacks an e1 standing on the shore.
// ────────────────────────────────────────────────────────────────────

#[test]
fn dd_can_attack_a_ground_target_on_the_shore() {
    let mut w = arena();
    // Water at x=20; shore is x=21.
    stamp_water_column(&mut w, 20);
    let (p1, p2) = playable_owner_ids(&w);
    // dd at (20, 10) on water; e1 at (21, 10) on land directly east.
    insert_test_actor(&mut w, make_ship(301, p1, (20, 10), "dd", 0));
    insert_test_actor(&mut w, make_e1(401, p2, (21, 10)));
    // Hold the e1 still so it doesn't run.
    set_actor_stance(&mut w, 401, 0);
    set_actor_stance(&mut w, 301, 3);

    let e1_start_hp = hp_of(&w, 401).unwrap();

    // Explicit Attack order from the dd at the e1. The engine's
    // "Attack" order reads the target actor id from `extra_data`,
    // matching the C# `Attack` order semantics.
    w.process_frame(&[GameOrder {
        order_string: "Attack".into(),
        subject_id: Some(301),
        target_string: None,
        extra_data: Some(401),
    }]);

    // The Attack order must have queued an Activity::Attack on the dd
    // (the engine accepted a Ship attacker against a ground target).
    let queued = w
        .actor(301)
        .and_then(|a| a.activity.as_ref())
        .map(|act| matches!(act, Activity::Attack { .. }))
        .unwrap_or(false);
    assert!(
        queued,
        "dd did not receive an Activity::Attack after an explicit Attack order"
    );

    // Run enough ticks to fire several volleys. The default fallback
    // weapon is damage=100, range=5, reload_delay=1 — at 1 cell apart
    // the dd should drain a chunk of the e1's hp inside ~400 ticks.
    for _ in 0..400 {
        if w.actor(401).is_none() {
            break;
        }
        w.process_frame(&[]);
    }

    let damage = e1_start_hp - hp_of(&w, 401).unwrap_or(0);
    assert!(
        damage > 0,
        "dd never landed a shot on a ground target sitting one cell away \
         on the shore (damage = {damage})"
    );
}
