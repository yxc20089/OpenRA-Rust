//! Engine guardrail: placing a 2nd `proc` auto-spawns its `harv`
//! NEAR the new proc (not piled on top of the lowest-id proc),
//! AND a freshly spawned harvester picks the path-shortest refinery
//! as its delivery target.
//!
//! Historical footgun (closed by this commit):
//!   * `order_place_building` called `spawn_unit("harv", owner)`,
//!     which routed through `find_spawn_location` — that helper
//!     sorts production-building candidates by `(!is_primary, id)`,
//!     so the auto-harv always materialised next to the LOWEST-ID
//!     proc, never the new one. A 2nd refinery placed far from the
//!     1st gained no throughput from its own auto-harv.
//!   * `find_refinery` returned the first (lowest-id) `proc` in
//!     `BTreeMap` order, so every harv deposited at the closest by
//!     ID, not by distance. Expansion was a no-op.
//!
//! Bench-side mirror:
//! `OpenRA-Bench/tests/test_proc_auto_spawn_python.py`.

use openra_data::oramap::{MapActor, OraMap, PlayerDef};
use openra_data::rules as data_rules;
use openra_sim::actor::{Activity, Actor, ActorKind};
use openra_sim::gamerules::GameRules;
use openra_sim::math::{CPos, WPos};
use openra_sim::traits::TraitState;
use openra_sim::world::{
    self, insert_test_actor, set_test_cash, set_test_unpaused, GameOrder, LobbyInfo, SlotInfo,
    World,
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
        title: "proc-spawn".into(),
        tileset: "TEMPERAT".into(),
        map_size: (96, 96),
        bounds: (0, 0, 96, 96),
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
                location: (90, 90),
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

fn make_building(id: u32, owner: u32, actor_type: &str, at: (i32, i32), hp: i32) -> Actor {
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
        actor_type: Some(actor_type.into()),
        kills: 0,
        rank: 0,
    }
}

/// Get the cell of an actor by id.
fn loc_of(w: &World, id: u32) -> Option<(i32, i32)> {
    for a in &w.snapshot().actors {
        if a.id == id {
            return Some((a.x, a.y));
        }
    }
    None
}

/// Locate the auto-spawned `harv` owned by `pid` whose actor id is
/// strictly greater than `min_id` (we use the post-place
/// `next_actor_id` to filter out any harv that existed before).
fn newest_harv(w: &World, pid: u32, min_id: u32) -> Option<u32> {
    let mut found: Option<u32> = None;
    for a in &w.snapshot().actors {
        if a.owner == pid && a.actor_type == "harv" && a.id >= min_id {
            found = Some(match found {
                Some(prev) if prev > a.id => prev,
                _ => a.id,
            });
        }
    }
    found
}

#[test]
fn second_proc_autospawns_harv_at_the_new_proc_not_the_old_one() {
    let mut world = match build_arena(1) {
        Some(w) => w,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };
    let agent_pid = world.player_ids()[1];

    // Pre-place a base: construction yard, power, and a 1st proc on
    // the west side. We DON'T pre-place the auto-harv for the 1st
    // proc; the existing one already covers the lowest-id case.
    insert_test_actor(&mut world, make_building(9001, agent_pid, "fact", (10, 10), 1000));
    insert_test_actor(&mut world, make_building(9002, agent_pid, "powr", (14, 10), 200));
    insert_test_actor(&mut world, make_building(9003, agent_pid, "powr", (16, 10), 200));
    insert_test_actor(&mut world, make_building(9004, agent_pid, "proc", (10, 14), 900));

    // Build a 2nd proc via the production queue + place it FAR EAST.
    // Cash for the proc (≈1400).
    set_test_cash(&mut world, agent_pid, 5000);
    world.process_frame(&[GameOrder {
        order_string: "StartProduction".into(),
        subject_id: Some(agent_pid),
        target_string: Some("proc".into()),
        extra_data: None,
    }]);
    // Spin ticks until the proc is built (no fixed budget — just
    // generous enough; if the build never finishes the placement is a
    // no-op and the test fails cleanly below).
    for _ in 0..1200 {
        world.process_frame(&[]);
    }

    // Record the id boundary so we can identify the brand-new harv.
    let pre_place_next = world.snapshot().actors.iter().map(|a| a.id).max().unwrap_or(0);

    // Place the 2nd proc FAR EAST.
    let east_proc_x = 70;
    let east_proc_y = 14;
    world.process_frame(&[GameOrder {
        order_string: "PlaceBuilding".into(),
        subject_id: Some(agent_pid),
        target_string: Some(format!("proc,{},{}", east_proc_x, east_proc_y)),
        extra_data: None,
    }]);
    // Let the SpawnUnit frame-end task fire.
    world.process_frame(&[]);

    // Find the new harv.
    let new_harv = newest_harv(&world, agent_pid, pre_place_next + 1)
        .expect("a 2nd harv must auto-spawn after placing a 2nd proc");
    let (hx, hy) = loc_of(&world, new_harv)
        .expect("new harv must have a location");

    // ASSERT 1: the new harv is adjacent (≤ ~3 cells Chebyshev) to
    // the NEW (east) proc — not to the OLD (west) proc.
    let dx_east = (hx - east_proc_x).abs();
    let dy_east = (hy - east_proc_y).abs();
    let cheb_east = dx_east.max(dy_east);
    let dx_west = (hx - 10).abs();
    let dy_west = (hy - 14).abs();
    let cheb_west = dx_west.max(dy_west);
    assert!(
        cheb_east <= 3,
        "auto-harv must spawn near the NEW proc; harv at \
         ({hx},{hy}), new proc at ({east_proc_x},{east_proc_y}), \
         Chebyshev distance={cheb_east} (must be ≤3)"
    );
    assert!(
        cheb_east < cheb_west,
        "auto-harv must be CLOSER to the new (east) proc than to the \
         old (west) proc; got east={cheb_east} west={cheb_west}"
    );

    // ASSERT 2: the new harv's Activity::Harvest carries the NEW
    // proc's id as the refinery (path-shortest tiebreak), not the
    // 1st (lowest-id) proc's. Find the new proc actor id first.
    let new_proc_id = world
        .snapshot()
        .actors
        .iter()
        .find(|a| {
            a.owner == agent_pid
                && a.actor_type == "proc"
                && a.x == east_proc_x
                && a.y == east_proc_y
        })
        .map(|a| a.id)
        .expect("east proc must exist after placement");
    assert_ne!(new_proc_id, 9004, "new proc id must differ from old proc id");

    // Reach into the live actor and pull `refinery_id` out of its
    // Harvest activity. (No bespoke test accessor — public `actor()`
    // is enough.)
    let bound_refinery = match world.actor(new_harv).and_then(|a| a.activity.as_ref()) {
        Some(Activity::Harvest { refinery_id, .. }) => *refinery_id,
        other => panic!("new harv must have Harvest activity; got {other:?}"),
    };
    assert_eq!(
        bound_refinery, new_proc_id,
        "new harv must bind to the path-shortest proc (id={new_proc_id}) \
         from its own spawn cell, not the lowest-id proc (id=9004)",
    );
}
