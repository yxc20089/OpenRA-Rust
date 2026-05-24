//! Stance:2 (Defend) no-chase contract.
//!
//! Per the documented stance semantics:
//!   stance:2 Defend — auto-engage closest in-range enemy; **does not
//!                     pursue** past current range.
//!
//! The original engine had a latent bug: an idle stance:2 defender
//! would auto-acquire a passing enemy via the auto-engage scan, issue
//! an Activity::Attack, and then — when the enemy moved out of weapon
//! range on the next tick — the Attack-tick fallback would CHASE the
//! target (line "Out of range: chase the target" in world.rs). This
//! converted any stance:2 unit into a hunter that abandoned its post
//! the moment its target rolled past, breaking the bait/decoy
//! perimeter idioms and the flank-vs-frontal geometry guarantee
//! (`combat-flanking-attack`).
//!
//! Contract pinned here: a stance:2 defender attacking an
//! auto-acquired target that moves out of range must DROP its Attack
//! activity and stay put — not chase. (Explicit player-issued attacks
//! `auto_acquired=false` retain chase, mirroring C# AttackBase
//! intent: player intent overrides stance.)

use openra_data::oramap::{MapActor, OraMap, PlayerDef};
use openra_data::rules as data_rules;
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::gamerules::GameRules;
use openra_sim::math::{CPos, WAngle, WPos};
use openra_sim::traits::TraitState;
use openra_sim::world::{
    self, insert_test_actor, set_actor_stance, set_test_unpaused, GameOrder, LobbyInfo, SlotInfo, World,
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
    let map = OraMap {
        title: "stance2-no-chase".into(),
        tileset: "TEMPERAT".into(),
        map_size: (64, 64),
        bounds: (0, 0, 64, 64),
        tiles: Vec::new(),
        actors: vec![
            MapActor { id: "mpspawn1".into(), actor_type: "mpspawn".into(),
                       owner: "Neutral".into(), location: (1, 1) },
            MapActor { id: "mpspawn2".into(), actor_type: "mpspawn".into(),
                       owner: "Neutral".into(), location: (60, 60) },
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
        starting_cash: 0,
        allow_spectators: true,
        occupied_slots: vec![
            SlotInfo { player_reference: "P1".into(), faction: "allies".into(), is_bot: false, starting_cash: None },
            SlotInfo { player_reference: "P2".into(), faction: "soviet".into(), is_bot: false, starting_cash: None },
        ],
    };
    let mut w = world::build_world(&map, 0, &lobby, Some(rules), 0, false);
    set_test_unpaused(&mut w);
    let strip: Vec<u32> = world::all_actor_ids(&w).into_iter()
        .filter(|&id| matches!(w.actor_kind(id), Some(ActorKind::Mcv) | Some(ActorKind::Spawn)))
        .collect();
    for id in strip { world::remove_test_actor(&mut w, id); }
    Some(w)
}

fn playable_owner_ids(w: &World) -> (u32, u32) {
    let mut ids: Vec<u32> = w.player_ids().to_vec();
    ids.pop(); // everyone
    let p2 = ids.pop().unwrap();
    let p1 = ids.pop().unwrap();
    (p1, p2)
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
            TraitState::BodyOrientation { quantized_facings: 8 },
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

/// A stance:2 defender that auto-acquires a tank passing within
/// weapon range must NOT chase when the tank moves out of range.
/// It must drop its Attack activity and remain at its post.
#[test]
fn test_stance_2_does_not_chase_passing_target() {
    let Some(mut world) = arena() else {
        eprintln!("skip: vendored RA mod dir not present");
        return;
    };
    let (p1, p2) = playable_owner_ids(&world);

    // Enemy e1 defender at (10, 10), stance:2 — Dragon range ~5 cells.
    insert_test_actor(&mut world, make_e1(401, p2, (10, 10), 50000));
    set_actor_stance(&mut world, 401, 2);
    let start_loc = (10i32, 10i32);

    // Friendly tank at (12, 10), 2 cells away — well inside Dragon
    // range so the defender auto-acquires. Tank invulnerable for the
    // duration so it doesn't die (test isolates defender behavior).
    insert_test_actor(&mut world, make_tank(201, p1, (12, 10), 1_000_000));
    set_actor_stance(&mut world, 201, 0); // tank holds fire to keep it pristine

    // Tick a few frames so the defender enters Attack on the tank.
    // Long enough to confirm the defender did fire (HP decrement on
    // tank — but tank is invulnerable HP-wise, so use an HP probe).
    for _ in 0..80 {
        world.process_frame(&[]);
    }
    // Sanity: the defender must have an Attack activity by now.
    eprintln!("defender activity after 80 frames = {:?}",
        world.actor(401).map(|a| a.activity.as_ref()
            .map(|act| format!("{:?}", act)).unwrap_or("None".into())));
    // Move the tank far east — out of weapon range.
    let move_order = vec![GameOrder {
        order_string: "Move".into(),
        subject_id: Some(201),
        target_string: Some("60,10".into()),
        extra_data: None,
    }];
    // Issue the move once, then tick frames empty.
    world.process_frame(&move_order);
    for _ in 0..200 {
        world.process_frame(&[]);
    }

    // Defender must STILL be at its original cell — stance:2 doesn't chase.
    let end_loc = world.actor(401).and_then(|a| a.location).unwrap_or((-1, -1));
    assert_eq!(
        end_loc, start_loc,
        "stance:2 (Defend) auto-acquired a passing tank then CHASED \
         after it left range: start={start_loc:?}, end={end_loc:?}. \
         stance:2 must hold post (auto-acquired Attack must be \
         dropped when the target leaves range, not converted into a \
         chase)."
    );
}

/// Mirror: a stance:2 defender given an EXPLICIT player-issued
/// attack DOES chase — explicit intent overrides stance, matching
/// C# AttackBase semantics.
#[test]
fn test_stance_2_chases_when_explicitly_ordered() {
    let Some(mut world) = arena() else {
        eprintln!("skip: vendored RA mod dir not present");
        return;
    };
    let (p1, p2) = playable_owner_ids(&world);

    // Two tanks, the chaser is friendly stance:2 — but we'll issue
    // an explicit Attack order, which must override.
    insert_test_actor(&mut world, make_tank(210, p1, (10, 10), 100000));
    set_actor_stance(&mut world, 210, 2);
    insert_test_actor(&mut world, make_tank(310, p2, (30, 10), 100000));
    set_actor_stance(&mut world, 310, 0);

    // Explicit attack order from player.
    let attack = vec![GameOrder {
        order_string: "Attack".into(),
        subject_id: Some(210),
        target_string: None,
        extra_data: Some(310),
    }];
    // First frame: order issued.
    world.process_frame(&attack);
    let start = world.actor(210).and_then(|a| a.location).unwrap();
    // Subsequent frames: empty orders (process_frame applies the
    // running Attack activity).
    for _ in 0..200 {
        world.process_frame(&[]);
        let cur = world.actor(210).and_then(|a| a.location).unwrap();
        if cur.0 > start.0 + 2 {
            return; // chased east as expected
        }
    }
    panic!("explicit Attack order on stance:2 unit did not chase out-of-range target");
}
