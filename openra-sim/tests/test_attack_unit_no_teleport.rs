//! Bug A regression: `attack_unit` on an out-of-sight target must
//! PATH toward it at normal movement speed, not TELEPORT.
//!
//! Background: an explicit `Attack` order against an enemy that is
//! outside the attacker's weapon range produces an `Activity::Attack`
//! whose chase logic closed distance. The chase used to warp the
//! attacker a FULL CELL per tick — for infantry (speed ≈ 43 world
//! units/tick ≈ 0.04 cell/tick) that is a ~24x speed-up, so an
//! `attack_unit` on a distant target crossed dozens of cells in a
//! single 90-tick decision frame. Expected behaviour: the chase
//! advances at the actor's real `Mobile` speed, identical to a plain
//! `move` order.
//!
//! Runs on the vendored RA mod so 2tnk / e1 carry real weapon + speed
//! stats; skips gracefully when the submodule isn't present.

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
        title: "teleport-arena".into(),
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
            SlotInfo { player_reference: "P1".into(), faction: "allies".into(), is_bot: false },
            SlotInfo { player_reference: "P2".into(), faction: "soviet".into(), is_bot: false },
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

fn cheb(a: (i32, i32), b: (i32, i32)) -> i32 {
    (a.0 - b.0).abs().max((a.1 - b.1).abs())
}

/// An explicit `attack_unit` order targeting a far-off enemy must not
/// warp the attacker across the map: in one 90-tick decision frame the
/// attacker should cover only a handful of cells (its real speed), not
/// dozens.
#[test]
fn attack_unit_far_target_paths_at_normal_speed() {
    let Some(mut world) = arena() else {
        eprintln!("skip: vendored RA mod dir not present");
        return;
    };
    let (p1, p2) = playable_owner_ids(&world);

    // Attacker at (10,10); enemy 70 cells east — far outside any
    // weapon range and outside the attacker's sight.
    insert_test_actor(&mut world, make_tank(201, p1, (10, 10), 46000));
    insert_test_actor(&mut world, make_tank(301, p2, (80, 10), 46000));

    let start = world.actor(201).and_then(|a| a.location).unwrap();

    // Baseline: how far does a *plain move* of the same actor travel
    // in 90 ticks? That is the legitimate distance budget.
    let mut probe = arena().unwrap();
    insert_test_actor(&mut probe, make_tank(401, p1, (10, 10), 46000));
    probe.process_frame(&[GameOrder {
        order_string: "Move".into(),
        subject_id: Some(401),
        target_string: Some("80,10".into()),
        extra_data: None,
    }]);
    for _ in 0..89 {
        probe.process_frame(&[]);
    }
    let move_loc = probe.actor(401).and_then(|a| a.location).unwrap();
    let move_dist = cheb(start, move_loc);
    assert!(
        move_dist > 0,
        "plain move probe didn't move at all — test harness broken"
    );

    // Issue the attack order, then advance one 90-tick decision frame.
    world.process_frame(&[GameOrder {
        order_string: "Attack".into(),
        subject_id: Some(201),
        target_string: None,
        extra_data: Some(301),
    }]);
    for _ in 0..89 {
        world.process_frame(&[]);
    }

    let end = world.actor(201).and_then(|a| a.location).unwrap();
    let attack_dist = cheb(start, end);

    // The chase must not move FASTER than a plain move. Allow a small
    // slack (a couple cells) for path/turn differences.
    assert!(
        attack_dist <= move_dist + 3,
        "attack_unit on an out-of-sight target TELEPORTED: moved \
         {attack_dist} cells in 90 ticks, but a plain move of the \
         same unit covers only {move_dist} cells. start={start:?} \
         end={end:?}"
    );
    // And it should still be making progress toward the target.
    assert!(
        attack_dist > 0,
        "attacker didn't advance toward the target at all"
    );
}
