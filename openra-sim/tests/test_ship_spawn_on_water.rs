//! Engine pin: a built `dd` (destroyer) auto-spawns on a WATER cell
//! adjacent to its shipyard (`syrd`), not on dry ground.
//!
//! Historical footgun (closed by the fix this test pins):
//! `find_spawn_location` only consulted `Terrain::is_passable`, which
//! reports water cells as IMPASSABLE for ground actors. As a result a
//! finished destroyer materialised on the closest GROUND cell next to
//! the shipyard — and `Mobile::Ship` cannot enter ground, so the
//! destroyer was effectively stuck on land from the moment it spawned
//! (no orders could ever move it).
//!
//! The fix routes the per-kind spawn search through:
//!   * Ship → require water cell (`terrain::is_water`)
//!   * Aircraft → ground-passable (cells aren't terrain-locked for
//!     aircraft, but the search needs an in-bounds anchor)
//!   * Infantry / vehicles → ground-passable (unchanged)
//!
//! This test exercises the WATER half of the fix end-to-end through
//! the production queue: a player owns a syrd next to a water column,
//! issues StartProduction(dd), waits for the build to finish, and
//! verifies the spawned destroyer's location is a water cell.

use openra_data::oramap::{MapActor, OraMap, PlayerDef};
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::math::CPos;
use openra_sim::traits::TraitState;
use openra_sim::world::{
    self, insert_test_actor, set_test_unpaused, GameOrder, LobbyInfo, SlotInfo, World,
};

fn arena() -> World {
    let map = OraMap {
        title: "ship-spawn-arena".into(),
        tileset: "TEMPERAT".into(),
        map_size: (40, 40),
        bounds: (0, 0, 40, 40),
        tiles: Vec::new(),
        actors: vec![
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
                location: (37, 37),
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
        starting_cash: 20_000,
        allow_spectators: false,
        occupied_slots: vec![
            SlotInfo { player_reference: "P1".into(), faction: "allies".into(), is_bot: false, starting_cash: None },
            SlotInfo { player_reference: "P2".into(), faction: "soviet".into(), is_bot: false, starting_cash: None },
        ],
    };
    // No vendor dir; defaults() ruleset path.
    let mut world = world::build_world(&map, 7, &lobby, None, 0, false);
    set_test_unpaused(&mut world);
    // Strip auto-spawned MCVs / spawn beacons so we control the layout.
    let strip: Vec<u32> = world::all_actor_ids(&world)
        .into_iter()
        .filter(|&id| matches!(world.actor_kind(id), Some(ActorKind::Mcv) | Some(ActorKind::Spawn)))
        .collect();
    for id in strip {
        world::remove_test_actor(&mut world, id);
    }
    world
}

fn insert_building(world: &mut World, id: u32, owner: u32, btype: &str, at: (i32, i32)) {
    let top_left = CPos::new(at.0, at.1);
    let stats = world.rules.actor(btype).expect("known building type");
    let actor = Actor {
        id,
        kind: ActorKind::Building,
        owner_id: Some(owner),
        location: Some(at),
        traits: vec![
            TraitState::BodyOrientation { quantized_facings: 1 },
            TraitState::Building { top_left },
            TraitState::Health { hp: stats.hp },
        ],
        activity: None,
        actor_type: Some(btype.into()),
        kills: 0,
        rank: 0,
    };
    insert_test_actor(world, actor);
}

#[test]
fn built_dd_spawns_on_water_adjacent_to_syrd() {
    let mut world = arena();
    let p1 = world.player_ids().get(1).copied().expect("P1 player id");

    // Stamp a 2-wide water column at x=10..11 spanning y=2..37 — the
    // shipyard sits at x=7..9 (footprint 3×3), so its east face touches
    // water at x=10..11.
    for y in 2..38 {
        for x in 10..12 {
            world.terrain.set_water(x, y, true);
        }
    }

    // Pre-place the agent's tech tree adjacent to the water. We use
    // `spen` (sub pen) here because the defaults() ruleset path keys
    // dd's prereq off `spen` (the vendored RA YAML uses `~syrd`/
    // `~spen` to accept either). Either naval producer drives the
    // same spawn path, so this test is the canonical pin for the
    // "ship spawn on water" engine fix.
    insert_building(&mut world, 1001, p1, "fact", (2, 2));
    insert_building(&mut world, 1002, p1, "powr", (6, 2));
    insert_building(&mut world, 1003, p1, "proc", (2, 6));
    insert_building(&mut world, 1004, p1, "dome", (2, 10));
    insert_building(&mut world, 1005, p1, "spen", (7, 6));

    // Give the player plenty of cash so the dd can finish.
    if let Some(player_actor) = world.actor_mut(p1) {
        player_actor.set_cash(5000);
    }

    // Tell the world we're paying for a destroyer.
    let order = GameOrder {
        order_string: "StartProduction".into(),
        subject_id: Some(p1),
        target_string: Some("dd".into()),
        extra_data: None,
    };
    world.process_frame(&[order]);

    // Advance until the destroyer pops out. The default `dd` build
    // delay is short relative to the ruleset's mock; 800 frames is
    // ample headroom.
    let mut spawn_pos: Option<(i32, i32)> = None;
    for _ in 0..800 {
        world.process_frame(&[]);
        for aid in world::all_actor_ids(&world) {
            let a = match world.actor(aid) {
                Some(a) => a,
                None => continue,
            };
            if a.owner_id == Some(p1)
                && a.actor_type.as_deref() == Some("dd")
                && let Some(loc) = a.location
            {
                spawn_pos = Some(loc);
                break;
            }
        }
        if spawn_pos.is_some() {
            break;
        }
    }

    let (sx, sy) = spawn_pos.expect(
        "built dd never appeared — production stalled or spawn refused",
    );
    assert!(
        world.terrain.is_water(sx, sy),
        "built dd must spawn on a WATER cell next to the syrd; got ({sx},{sy}) \
         which is_water={}",
        world.terrain.is_water(sx, sy)
    );
}
