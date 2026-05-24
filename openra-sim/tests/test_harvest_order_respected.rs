//! Engine regression test for Fix #4 from OpenRA-Bench's
//! ENGINE_FOLLOWUPS Wave-12/13: an explicit `Harvest(unit_id, x, y)`
//! order from the agent must NOT be silently overridden by the
//! `auto_route_idle_harvesters` back-stop. Before the fix the FAR
//! patch (the user's chosen target) was always ignored once the
//! harvester briefly went idle (between cycles or right after the
//! order), and the harv was re-routed to the nearest patch — so
//! every patch-allocation policy collapsed to the same throughput.

use openra_data::oramap::{MapActor, OraMap, PlayerDef};
use openra_sim::actor::{Activity, Actor, ActorKind};
use openra_sim::gamerules::GameRules;
use openra_sim::math::{CPos, WPos};
use openra_sim::terrain::ResourceType;
use openra_sim::traits::TraitState;
use openra_sim::world::{
    self, insert_test_actor, set_test_unpaused, GameOrder, LobbyInfo, SlotInfo, World,
};

fn build_arena(seed: i32) -> World {
    let rules = GameRules::vendor_cached();
    let map = OraMap {
        title: "harv-order-test".into(),
        tileset: "TEMPERAT".into(),
        map_size: (96, 64),
        bounds: (0, 0, 96, 64),
        tiles: Vec::new(),
        actors: vec![
            MapActor { id: "mpspawn1".into(), actor_type: "mpspawn".into(),
                       owner: "Neutral".into(), location: (1, 1) },
            MapActor { id: "mpspawn2".into(), actor_type: "mpspawn".into(),
                       owner: "Neutral".into(), location: (94, 62) },
        ],
        players: vec![
            PlayerDef { name: "Neutral".into(), playable: false, owns_world: true,
                        non_combatant: true, faction: "allies".into(), enemies: Vec::new() },
            PlayerDef { name: "P1".into(), playable: true, owns_world: false,
                        non_combatant: false, faction: "allies".into(),
                        enemies: vec!["P2".into()] },
            PlayerDef { name: "P2".into(), playable: true, owns_world: false,
                        non_combatant: false, faction: "soviet".into(),
                        enemies: vec!["P1".into()] },
        ],
    };
    let lobby = LobbyInfo {
        starting_cash: 0, allow_spectators: false,
        occupied_slots: vec![
            SlotInfo { player_reference: "P1".into(), faction: "allies".into(),
                       is_bot: false, starting_cash: None },
            SlotInfo { player_reference: "P2".into(), faction: "soviet".into(),
                       is_bot: false, starting_cash: None },
        ],
    };
    let mut w = world::build_world(&map, seed, &lobby, Some(rules), 0, false);
    set_test_unpaused(&mut w);
    let strip: Vec<u32> = world::all_actor_ids(&w)
        .into_iter()
        .filter(|&id| matches!(
            w.actor_kind(id),
            Some(ActorKind::Mcv) | Some(ActorKind::Spawn)
        ))
        .collect();
    for id in strip {
        world::remove_test_actor(&mut w, id);
    }
    w
}

fn make_building(id: u32, owner: u32, kind_name: &str, at: (i32, i32), hp: i32) -> Actor {
    let cell = CPos::new(at.0, at.1);
    let center = WPos::new(at.0 * 1024 + 512, at.1 * 1024 + 512, 0);
    Actor {
        id, kind: ActorKind::Building, owner_id: Some(owner), location: Some(at),
        traits: vec![
            TraitState::BodyOrientation { quantized_facings: 1 },
            TraitState::Building { top_left: cell },
            TraitState::Immobile { top_left: cell, center_position: center },
            TraitState::Health { hp },
        ],
        activity: None, actor_type: Some(kind_name.into()), kills: 0, rank: 0,
    }
}

fn make_harv(id: u32, owner: u32, at: (i32, i32), hp: i32) -> Actor {
    let cell = CPos::new(at.0, at.1);
    let center = WPos::new(at.0 * 1024 + 512, at.1 * 1024 + 512, 0);
    Actor {
        id, kind: ActorKind::Vehicle, owner_id: Some(owner), location: Some(at),
        traits: vec![
            TraitState::BodyOrientation { quantized_facings: 32 },
            TraitState::Mobile {
                facing: 0, from_cell: cell, to_cell: cell, center_position: center,
            },
            TraitState::Health { hp },
        ],
        activity: None, // start idle — auto-route will pick it up
        actor_type: Some("harv".into()), kills: 0, rank: 0,
    }
}

/// Drop an ore patch (3x3) around (cx, cy).
fn drop_ore_patch(w: &mut World, cx: i32, cy: i32) {
    for dy in -1..=1 {
        for dx in -1..=1 {
            w.terrain.set_resource(cx + dx, cy + dy, ResourceType::Ore, 12);
        }
    }
}

fn harv_target_cell(w: &World, hid: u32) -> Option<(i32, i32)> {
    // The Harvest activity stores `last_harvest_cell` once the FSM
    // sets a target; we use that as a proxy for "which patch was the
    // engine sent toward".
    let actor = w.actor(hid)?;
    if let Some(Activity::Harvest { last_harvest_cell, .. }) = &actor.activity {
        *last_harvest_cell
    } else {
        None
    }
}

fn harv_path_destination(w: &World, hid: u32) -> Option<(i32, i32)> {
    let actor = w.actor(hid)?;
    if let Some(Activity::Harvest { path, .. }) = &actor.activity {
        path.last().copied()
    } else {
        None
    }
}

fn closer_to(loc: (i32, i32), a: (i32, i32), b: (i32, i32)) -> bool {
    let da = (loc.0 - a.0).abs() + (loc.1 - a.1).abs();
    let db = (loc.0 - b.0).abs() + (loc.1 - b.1).abs();
    da < db
}

#[test]
fn explicit_harvest_order_overrides_auto_route_to_nearest() {
    let mut w = build_arena(31);
    let pids = w.player_ids().to_vec();
    let agent = pids[1];

    // Proc next to NEAR patch.
    insert_test_actor(&mut w, make_building(9000, agent, "proc", (10, 20), 900));
    let near = (16, 20);
    let far = (60, 20);
    drop_ore_patch(&mut w, near.0, near.1);
    drop_ore_patch(&mut w, far.0, far.1);

    // Spawn harv near the proc, idle (auto-route picks it up).
    let hid = 7000u32;
    insert_test_actor(&mut w, make_harv(hid, agent, (12, 20), 100));

    // Tick a few frames so the auto-route assigns the harv to FindingOre.
    for _ in 0..20 {
        let _ = w.tick(&[]);
    }
    // Sanity: it's now in Harvest activity and aimed at NEAR.
    let pre_target = harv_target_cell(&w, hid)
        .or_else(|| harv_path_destination(&w, hid));
    if let Some(t) = pre_target {
        assert!(
            closer_to(t, near, far),
            "before user order, auto-route should aim at NEAR={near:?} not FAR={far:?}; got {t:?}"
        );
    }

    // Now: user issues an explicit Harvest order to the FAR patch.
    let _ = w.tick(&[GameOrder {
        order_string: "Harvest".into(),
        subject_id: Some(hid),
        target_string: Some(format!("{},{}", far.0, far.1)),
        extra_data: None,
    }]);

    // Tick 100 frames. The harv must end up moving toward FAR, not
    // back to NEAR. Use the lesser of last_harvest_cell or the active
    // path's last cell to gauge the engine's intent.
    for _ in 0..100 {
        let _ = w.tick(&[]);
    }
    let target = harv_target_cell(&w, hid)
        .or_else(|| harv_path_destination(&w, hid))
        .expect("harv should still be in a Harvest activity after 100 ticks");
    assert!(
        closer_to(target, far, near),
        "after user Harvest({far:?}), engine target must be NEAR FAR not NEAR; \
         got target={target:?}"
    );
}

#[test]
fn no_user_order_means_auto_route_picks_nearest_patch() {
    // Negative-case control: without an explicit Harvest order, the
    // harvester should naturally go to the closest patch (NEAR).
    let mut w = build_arena(31);
    let pids = w.player_ids().to_vec();
    let agent = pids[1];

    insert_test_actor(&mut w, make_building(9000, agent, "proc", (10, 20), 900));
    let near = (16, 20);
    let far = (60, 20);
    drop_ore_patch(&mut w, near.0, near.1);
    drop_ore_patch(&mut w, far.0, far.1);

    let hid = 7000u32;
    insert_test_actor(&mut w, make_harv(hid, agent, (12, 20), 100));

    for _ in 0..50 {
        let _ = w.tick(&[]);
    }
    let target = harv_target_cell(&w, hid)
        .or_else(|| harv_path_destination(&w, hid))
        .expect("auto-route should have assigned a Harvest target");
    assert!(
        closer_to(target, near, far),
        "without a user order, auto-route should pick the NEAR patch; got {target:?}"
    );
}
