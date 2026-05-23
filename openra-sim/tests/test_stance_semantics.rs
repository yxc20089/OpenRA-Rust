//! Stance semantics regression / engagement contract.
//!
//! Pins the three behaviourally-distinct stances so packs that key
//! on a stance flip (e.g. `combat-stance-mgmt-attack`,
//! `def-stance-mgmt-hold-then-attack`) are not silently equivalent.
//!
//!   • stance:0 (HoldFire)       — does NOT auto-engage anything.
//!   • stance:1 (ReturnFire)     — fires ONLY at attackers that have
//!                                  recently damaged this unit.
//!   • stance:3 (AttackAnything) — fires at in-range enemies AND
//!                                  ADVANCES toward visible enemies
//!                                  that sit beyond weapon range
//!                                  ("hunt" semantics).
//!
//! Background: the original engine only suppressed engagement at
//! stance:0; stance:1 and stance:3 collapsed into "auto-fire on any
//! in-range enemy, never advance" — making the `combat-stance-mgmt-
//! attack` pack indistinguishable from `def-stance-mgmt-hold-then-
//! attack`. The pack was retired pending this fix. See
//! `OpenRA-Bench/CLAUDE.md` "engine blockers" + the stance footgun.
//!
//! These tests run on the vendored RA mod so 2tnk has real weapon
//! stats (90mm cannon, range ≈5 cells, damage 4000). They skip
//! gracefully when the vendored mod isn't present (CI without
//! submodules).

use openra_data::oramap::{MapActor, OraMap, PlayerDef};
use openra_data::rules as data_rules;
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::gamerules::GameRules;
use openra_sim::math::{CPos, WAngle, WPos};
use openra_sim::traits::TraitState;
use openra_sim::world::{
    self, insert_test_actor, set_actor_stance, set_test_unpaused, LobbyInfo, SlotInfo, World,
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
            location: (60, 60),
        },
    ];
    let map = OraMap {
        title: "stance-arena".into(),
        tileset: "TEMPERAT".into(),
        map_size: (64, 64),
        bounds: (0, 0, 64, 64),
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
    // Strip the auto-spawned MCVs / spawn markers so they don't
    // wander into the action.
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

fn make_e1(id: u32, owner: u32, at: (i32, i32), hp: i32) -> Actor {
    Actor {
        id,
        kind: ActorKind::Infantry,
        owner_id: Some(owner),
        location: Some(at),
        traits: vec![TraitState::Health { hp }],
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

// ── stance:0 contract ────────────────────────────────────────────

#[test]
fn test_stance_0_holds_fire() {
    let Some(mut world) = arena() else {
        eprintln!("skip: vendored RA mod dir not present");
        return;
    };
    let (p1, p2) = playable_owner_ids(&world);
    // Agent tank on stance:0, enemy infantry on stance:3 firing at it.
    insert_test_actor(&mut world, make_tank(201, p1, (10, 10), 46000));
    set_actor_stance(&mut world, 201, 0);
    insert_test_actor(&mut world, make_e1(301, p2, (13, 10), 1000));
    set_actor_stance(&mut world, 301, 3);

    let start_hp = hp_of(&world, 301).unwrap();
    for _ in 0..100 {
        world.process_frame(&[]);
    }
    // Tank on HoldFire must NOT have hit the enemy.
    let end_hp = hp_of(&world, 301).unwrap_or(0);
    assert_eq!(
        end_hp, start_hp,
        "stance:0 (HoldFire) tank fired on enemy: hp {start_hp} → {end_hp}"
    );
}

// ── stance:1 contract ────────────────────────────────────────────

#[test]
fn test_stance_1_return_fire_only_against_passive_enemy() {
    let Some(mut world) = arena() else {
        eprintln!("skip: vendored RA mod dir not present");
        return;
    };
    let (p1, p2) = playable_owner_ids(&world);
    // Agent tank on stance:1, enemy infantry on stance:0 (will NEVER
    // fire first). The tank must NOT auto-engage — return-fire is
    // conditional on having received damage.
    insert_test_actor(&mut world, make_tank(202, p1, (10, 10), 46000));
    set_actor_stance(&mut world, 202, 1);
    insert_test_actor(&mut world, make_e1(302, p2, (13, 10), 1000));
    set_actor_stance(&mut world, 302, 0);

    let start_hp = hp_of(&world, 302).unwrap();
    for _ in 0..100 {
        world.process_frame(&[]);
    }
    let end_hp = hp_of(&world, 302).unwrap_or(0);
    assert_eq!(
        end_hp, start_hp,
        "stance:1 (ReturnFire) tank fired on a passive enemy that \
         never attacked: hp {start_hp} → {end_hp}. ReturnFire must \
         require an attack to trigger."
    );
}

#[test]
fn test_stance_1_returns_fire_on_attacker() {
    let Some(mut world) = arena() else {
        eprintln!("skip: vendored RA mod dir not present");
        return;
    };
    let (p1, p2) = playable_owner_ids(&world);
    // Agent tank on stance:1, enemy infantry on stance:3 firing at it
    // (and in range of the M1Carbine rifle). The tank MUST return
    // fire and kill the e1.
    insert_test_actor(&mut world, make_tank(203, p1, (10, 10), 46000));
    set_actor_stance(&mut world, 203, 1);
    insert_test_actor(&mut world, make_e1(303, p2, (13, 10), 1000));
    set_actor_stance(&mut world, 303, 3);

    for _ in 0..200 {
        world.process_frame(&[]);
        if hp_of(&world, 303).unwrap_or(0) <= 0 {
            break;
        }
    }
    let end_hp = hp_of(&world, 303).unwrap_or(0);
    assert!(
        end_hp <= 0,
        "stance:1 tank failed to return fire on its attacker; \
         enemy hp {end_hp} after 200 frames"
    );
}

// ── stance:3 contract ────────────────────────────────────────────

#[test]
fn test_stance_3_hunts_visible_enemy() {
    let Some(mut world) = arena() else {
        eprintln!("skip: vendored RA mod dir not present");
        return;
    };
    let (p1, p2) = playable_owner_ids(&world);
    // Agent tank on stance:3, enemy infantry on stance:0 (passive),
    // 15 cells away — OUT of cannon range (≈5 cells) but inside
    // sight range (the tank's RevealsShroud reveals 7-8 cells; we
    // sprinkle scouts to cover the corridor so the e1 is visible
    // via fact-shared sight regardless).
    insert_test_actor(&mut world, make_tank(204, p1, (10, 20), 46000));
    set_actor_stance(&mut world, 204, 3);
    // Scouts along the corridor so the target is visible to the
    // hunter at start (sight range alone may be ~7 cells; the
    // hunting logic should advance toward a visible enemy
    // regardless, and CombatReveal triggers on first hit).
    insert_test_actor(&mut world, make_e1(401, p1, (15, 20), 1000));
    set_actor_stance(&mut world, 401, 0);
    insert_test_actor(&mut world, make_e1(402, p1, (20, 20), 1000));
    set_actor_stance(&mut world, 402, 0);
    // The actual target — 15 cells east, well out of cannon range.
    insert_test_actor(&mut world, make_e1(304, p2, (25, 20), 1000));
    set_actor_stance(&mut world, 304, 0);

    let start_loc = world.actor(204).and_then(|a| a.location).unwrap();
    let mut killed = false;
    for _ in 0..300 {
        world.process_frame(&[]);
        if hp_of(&world, 304).unwrap_or(0) <= 0 {
            killed = true;
            break;
        }
    }
    let end_loc = world.actor(204).and_then(|a| a.location);
    assert!(
        killed,
        "stance:3 tank failed to HUNT a visible-but-out-of-range \
         enemy. start={start_loc:?} end={end_loc:?} target_hp={:?}",
        hp_of(&world, 304)
    );
    // Sanity: the tank should have advanced east (target was east).
    if let Some(end) = end_loc {
        assert!(
            end.0 > start_loc.0,
            "stance:3 tank killed the enemy without moving east \
             (start={start_loc:?}, end={end:?}); something else \
             scored the kill"
        );
    }
}
