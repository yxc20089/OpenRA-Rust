//! Pin the contract that an explicit agent order ALWAYS overrides
//! auto-engage. Three scenarios:
//!
//! 1. `move_through_enemy_does_not_divert` — unit on Move past an
//!    in-range enemy stays on Move; auto-engage scan does not flip
//!    it to Attack. This is the regression that caused unit 1008
//!    to freeze on (50,15) for 12+ turns in dagger-v7 seed 3 of
//!    `eval-maginot-fix-objective-20260508`.
//!
//! 2. `idle_unit_auto_engages_in_range_enemy` — symmetric: a unit
//!    with Activity::None next to an enemy DOES auto-engage. This
//!    is the only path through which auto-engage should fire post-
//!    fix; otherwise the engine becomes a pure sandbox.
//!
//! 3. `move_overrides_running_attack` — agent issues Move while a
//!    unit is in Activity::Attack (set by a prior `attack_target`
//!    call or by auto-engage). The new Move replaces the Attack
//!    on the next tick. Guarantees `move` semantics never get
//!    hijacked by stale combat state.
//!
//! Uses the vendored RA YAML so weapons + ranges + sights are real
//! (the fallback `GameRules::defaults()` has empty weapon lists for
//! infantry, so the auto-engage range check would be a no-op).

use openra_data::oramap::{MapActor, OraMap, PlayerDef};
use openra_data::rules as data_rules;
use openra_sim::actor::{Activity, Actor, ActorKind};
use openra_sim::gamerules::GameRules;
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
    // Skip silently if the vendored RA mod isn't present (CI without
    // submodules). The test still passes in that environment.
    let mod_dir = vendor_mod_dir()?;
    let ruleset = data_rules::load_ruleset(&mod_dir).ok()?;
    let rules = GameRules::from_ruleset(&ruleset);

    let spawn_actors = vec![
        MapActor {
            id: "mpspawn1".into(), actor_type: "mpspawn".into(),
            owner: "Neutral".into(), location: (1, 1),
        },
        MapActor {
            id: "mpspawn2".into(), actor_type: "mpspawn".into(),
            owner: "Neutral".into(), location: (38, 38),
        },
    ];
    let map = OraMap {
        title: "auto-engage".into(),
        tileset: "TEMPERAT".into(),
        map_size: (40, 40),
        bounds: (0, 0, 40, 40),
        tiles: Vec::new(),
        actors: spawn_actors,
        players: vec![
            PlayerDef {
                name: "Neutral".into(), playable: false, owns_world: true,
                non_combatant: true, faction: "allies".into(), enemies: Vec::new(),
            },
            PlayerDef {
                name: "P1".into(), playable: true, owns_world: false,
                non_combatant: false, faction: "allies".into(), enemies: vec!["P2".into()],
            },
            PlayerDef {
                name: "P2".into(), playable: true, owns_world: false,
                non_combatant: false, faction: "soviet".into(), enemies: vec!["P1".into()],
            },
        ],
    };
    let lobby = LobbyInfo {
        starting_cash: 5000,
        allow_spectators: true,
        occupied_slots: vec![
            SlotInfo { player_reference: "P1".into(), faction: "allies".into(), is_bot: false },
            SlotInfo { player_reference: "P2".into(), faction: "soviet".into(), is_bot: false },
        ],
    };
    let mut w = world::build_world(&map, 0, &lobby, Some(rules), 0, false);
    set_test_unpaused(&mut w);
    Some(w)
}

fn make_e1(id: u32, owner: u32, at: (i32, i32), hp: i32) -> Actor {
    Actor {
        id, kind: ActorKind::Infantry, owner_id: Some(owner),
        location: Some(at),
        traits: vec![TraitState::Health { hp }],
        activity: None, actor_type: Some("e1".into()), kills: 0, rank: 0,
    }
}

fn playable_owner_ids(w: &World) -> (u32, u32) {
    // build_world: non-playable players first (Neutral), then occupied
    // slots, then "everyone". Skip last (everyone), take last 2.
    let mut ids: Vec<u32> = w.player_ids().to_vec();
    ids.pop(); // everyone
    let p2 = ids.pop().unwrap();
    let p1 = ids.pop().unwrap();
    (p1, p2)
}

fn activity_is_move(a: &Option<Activity>) -> bool {
    match a {
        Some(Activity::Move { .. }) => true,
        // Engine wraps a Move in a Turn for facing alignment when the
        // path direction doesn't match the unit's current facing. The
        // nested Move is what auto-engage WOULD have overwritten, so
        // count Turn{ then: Move } as still-on-Move.
        Some(Activity::Turn { then, .. }) => matches!(
            then.as_deref(),
            Some(Activity::Move { .. })
        ),
        _ => false,
    }
}

#[test]
fn move_through_enemy_does_not_divert() {
    let Some(mut world) = arena() else {
        eprintln!("skip: vendored RA mod dir not present");
        return;
    };
    let (p1, p2) = playable_owner_ids(&world);
    insert_test_actor(&mut world, make_e1(101, p1, (5, 10), 5000));
    insert_test_actor(&mut world, make_e1(102, p2, (8, 10), 5000));

    world.process_frame(&[GameOrder {
        order_string: "Move".into(),
        subject_id: Some(101),
        target_string: Some("20,10".into()),
        extra_data: None,
    }]);
    for _ in 0..5 {
        world.process_frame(&[]);
    }

    let actor = world.actor(101).expect("actor still alive");
    assert!(
        activity_is_move(&actor.activity),
        "Move must survive auto-engage when an enemy is in range. Got: {:?}",
        actor.activity
    );
    let enemy_hp = world
        .actor(102)
        .and_then(|a| a.traits.iter().find_map(|t| match t {
            TraitState::Health { hp } => Some(*hp),
            _ => None,
        }))
        .unwrap();
    assert_eq!(enemy_hp, 5000, "moving unit should not have opened fire");
}

#[test]
fn idle_unit_auto_engages_in_range_enemy() {
    let Some(mut world) = arena() else {
        eprintln!("skip: vendored RA mod dir not present");
        return;
    };
    let (p1, p2) = playable_owner_ids(&world);
    insert_test_actor(&mut world, make_e1(101, p1, (5, 10), 5000));
    insert_test_actor(&mut world, make_e1(102, p2, (8, 10), 5000));

    assert!(world.actor(101).unwrap().activity.is_none(), "starts idle");

    // Run a few frames so auto-engage scans run multiple times.
    for _ in 0..3 {
        world.process_frame(&[]);
    }

    let activity = world.actor(101).unwrap().activity.clone();
    match activity {
        Some(Activity::Attack { target_id, .. }) => {
            assert_eq!(target_id, 102, "auto-engage should pick the only enemy");
        }
        other => panic!(
            "idle unit must auto-engage in-range enemy; got {other:?}"
        ),
    }
}

#[test]
fn move_overrides_running_attack() {
    let Some(mut world) = arena() else {
        eprintln!("skip: vendored RA mod dir not present");
        return;
    };
    let (p1, p2) = playable_owner_ids(&world);
    // Use a high-HP enemy so it doesn't die during the warm-up.
    insert_test_actor(&mut world, make_e1(101, p1, (5, 10), 5000));
    insert_test_actor(&mut world, make_e1(102, p2, (8, 10), 100_000));

    // Warm-up: idle attacker auto-engages.
    for _ in 0..3 {
        world.process_frame(&[]);
    }
    assert!(
        matches!(
            world.actor(101).unwrap().activity,
            Some(Activity::Attack { .. })
        ),
        "warm-up: expected Activity::Attack from auto-engage, got {:?}",
        world.actor(101).unwrap().activity,
    );

    // Now agent says "move away".
    world.process_frame(&[GameOrder {
        order_string: "Move".into(),
        subject_id: Some(101),
        target_string: Some("30,10".into()),
        extra_data: None,
    }]);

    let actor = world.actor(101).expect("actor still alive");
    assert!(
        activity_is_move(&actor.activity),
        "Move order must replace the running Attack; got: {:?}",
        actor.activity
    );
}
