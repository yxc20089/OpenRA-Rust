//! Engine pin: an AA defense (`sam` / `agun`) auto-fires on enemy
//! aircraft (`heli`/`mig`/`yak`/`hind`) and damages them.
//!
//! This pin is load-bearing for the Family-11 "all-air LOSES against
//! enemy AA" wrong-arm trap. The auto-engage scan used to skip both
//! AntiAirOnly defenses (filtered out at `classify_defense`) AND
//! `ActorKind::Aircraft` actors (excluded from the candidate filter),
//! so a heli could fly over a sam unmolested. The fix:
//!   * include `DefenseKind::AntiAirOnly` in the defense auto-scan,
//!   * extend the candidate filter so AA defenses see Aircraft (and
//!     ground defenses still exclude Aircraft).
//!
//! Test runs on the no-vendor defaults() ruleset. The defaults() table
//! attaches a stub AA weapon (`AAStub`: damage 2000, range 8c, reload
//! 12, burst 2) to sam/agun so the engagement is observable without
//! the submodule.

use openra_data::oramap::{MapActor, OraMap, PlayerDef};
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::math::{CPos, WPos};
use openra_sim::traits::TraitState;
use openra_sim::world::{
    self, insert_test_actor, set_test_unpaused, LobbyInfo, SlotInfo, World,
};

fn arena() -> World {
    let map = OraMap {
        title: "aa-arena".into(),
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
            SlotInfo { player_reference: "P1".into(), faction: "allies".into(), is_bot: false, starting_cash: None },
            SlotInfo { player_reference: "P2".into(), faction: "soviet".into(), is_bot: false, starting_cash: None },
        ],
    };
    let mut world = world::build_world(&map, 11, &lobby, None, 0, false);
    set_test_unpaused(&mut world);
    let strip: Vec<u32> = world::all_actor_ids(&world)
        .into_iter()
        .filter(|&id| matches!(world.actor_kind(id), Some(ActorKind::Mcv) | Some(ActorKind::Spawn)))
        .collect();
    for id in strip {
        world::remove_test_actor(&mut world, id);
    }
    world
}

fn insert_aa(world: &mut World, id: u32, owner: u32, btype: &str, at: (i32, i32)) {
    let top_left = CPos::new(at.0, at.1);
    let stats = world.rules.actor(btype).expect("known AA building");
    insert_test_actor(world, Actor {
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
    });
}

fn insert_heli(world: &mut World, id: u32, owner: u32, at: (i32, i32)) {
    use openra_sim::math::WAngle;
    let cell = CPos::new(at.0, at.1);
    let center_x = at.0 * 1024 + 512;
    let center_y = at.1 * 1024 + 512;
    let center = WPos::new(center_x, center_y, 0);
    let facing = WAngle::new(512).angle;
    insert_test_actor(world, Actor {
        id,
        kind: ActorKind::Aircraft,
        owner_id: Some(owner),
        location: Some(at),
        traits: vec![
            TraitState::BodyOrientation { quantized_facings: 32 },
            TraitState::Mobile {
                facing,
                from_cell: cell,
                to_cell: cell,
                center_position: center,
            },
            TraitState::Health { hp: 12000 },
        ],
        activity: None,
        actor_type: Some("heli".into()),
        kills: 0,
        rank: 0,
    });
}

fn heli_hp(world: &World, heli_id: u32) -> Option<i32> {
    world.actor(heli_id).and_then(|a| {
        a.traits.iter().find_map(|t| match t {
            TraitState::Health { hp } => Some(*hp),
            _ => None,
        })
    })
}

#[test]
fn agun_auto_fires_on_enemy_heli_in_range_and_damages_it() {
    let mut world = arena();
    let p1 = world.player_ids().get(1).copied().expect("P1 id");
    let p2 = world.player_ids().get(2).copied().expect("P2 id");

    // P1 places an `agun` (Allied AA) at (10, 10). The defaults()
    // path attaches `AAStub` (range 8c) so the heli at (12, 12)
    // sits well inside range (Chebyshev = 2).
    insert_aa(&mut world, 1001, p1, "agun", (10, 10));

    // P2 hovers a heli within range.
    let heli_id = 2001;
    insert_heli(&mut world, heli_id, p2, (12, 12));

    let initial_hp = heli_hp(&world, heli_id).expect("heli has Health trait");
    assert_eq!(initial_hp, 12000);

    // Run enough ticks for the auto-scan + a few reloads.
    for _ in 0..200 {
        world.process_frame(&[]);
    }

    let final_hp = heli_hp(&world, heli_id).unwrap_or(0);
    assert!(
        final_hp < initial_hp,
        "heli should have taken damage from the AA gun; \
         initial_hp={initial_hp}, final_hp={final_hp}"
    );
}

#[test]
fn sam_auto_fires_on_enemy_heli_in_range_and_damages_it() {
    let mut world = arena();
    let p1 = world.player_ids().get(1).copied().expect("P1 id");
    let p2 = world.player_ids().get(2).copied().expect("P2 id");

    // P2 places a `sam` (Soviet AA), P1 owns the heli. Mirrors the
    // agun test on the opposite faction.
    insert_aa(&mut world, 1001, p2, "sam", (10, 10));
    let heli_id = 2001;
    insert_heli(&mut world, heli_id, p1, (12, 12));

    let initial_hp = heli_hp(&world, heli_id).unwrap_or(0);
    for _ in 0..200 {
        world.process_frame(&[]);
    }
    let final_hp = heli_hp(&world, heli_id).unwrap_or(0);
    assert!(
        final_hp < initial_hp,
        "heli should have taken damage from the SAM; \
         initial_hp={initial_hp}, final_hp={final_hp}"
    );
}

#[test]
fn ground_turret_does_not_target_aircraft() {
    // Discrimination guardrail: a `gun` (ground turret) ignores
    // aircraft, so the AA distinction is meaningful. Without this,
    // every ground turret would shred helis and the wrong-arm trap
    // collapses.
    let mut world = arena();
    let p1 = world.player_ids().get(1).copied().expect("P1 id");
    let p2 = world.player_ids().get(2).copied().expect("P2 id");

    insert_aa(&mut world, 1001, p1, "gun", (10, 10));
    let heli_id = 2001;
    insert_heli(&mut world, heli_id, p2, (12, 12));

    let initial_hp = heli_hp(&world, heli_id).unwrap_or(0);
    for _ in 0..200 {
        world.process_frame(&[]);
    }
    let final_hp = heli_hp(&world, heli_id).unwrap_or(0);
    assert_eq!(
        final_hp, initial_hp,
        "ground turret must NOT hit aircraft; gun should ignore the heli"
    );
}
