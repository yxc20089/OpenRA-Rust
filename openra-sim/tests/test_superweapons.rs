//! Engine acceptance: the three superweapons (mslo / iron / pdox) all
//! charge under their launcher building, fire correctly, and produce
//! the documented effect — nuke kills enemy actors at a target cell,
//! iron-curtain makes a friendly actor immune to damage for ~30s, and
//! the chronosphere teleports a friendly actor to a chosen cell.
//!
//! Uses `GameRules::defaults()` so the test runs without the vendored
//! RA YAML (mslo/iron/pdox are registered in the defaults table).

use openra_data::oramap::{MapActor, OraMap, PlayerDef};
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::gamerules::GameRules;
use openra_sim::math::{CPos, WPos};
use openra_sim::superweapon::{SuperweaponKind, NUKE_RADIUS_CELLS};
use openra_sim::traits::TraitState;
use openra_sim::world::{
    self, insert_test_actor, set_test_unpaused, LobbyInfo, SlotInfo, World,
};

fn build_arena(seed: i32) -> World {
    let rules = GameRules::defaults();
    let map = OraMap {
        title: "superweapon-test".into(),
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
            SlotInfo {
                player_reference: "P1".into(),
                faction: "allies".into(),
                is_bot: false,
                starting_cash: None,
            },
            SlotInfo {
                player_reference: "P2".into(),
                faction: "soviet".into(),
                is_bot: false,
                starting_cash: None,
            },
        ],
    };
    let mut w = world::build_world(&map, seed, &lobby, Some(rules), 0, false);
    set_test_unpaused(&mut w);
    // Strip the auto-spawned MCVs / spawn beacons so they don't
    // accidentally get caught by the nuke radius.
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

fn make_building(id: u32, owner: u32, kind_name: &str, at: (i32, i32), hp: i32) -> Actor {
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
        actor_type: Some(kind_name.into()),
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
                facing: 0,
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

fn read_hp(world: &World, id: u32) -> Option<i32> {
    world.actor(id)?.traits.iter().find_map(|t| {
        if let TraitState::Health { hp } = t { Some(*hp) } else { None }
    })
}

fn fully_charge(world: &mut World, kind: SuperweaponKind, owner: u32) {
    // Tick the world until the typed manager reports ready (the launcher
    // building must exist before we call this).
    for _ in 0..(kind.charge_ticks() + 5) {
        if world.superweapons.is_ready(kind, owner) {
            return;
        }
        let _ = world.tick(&[]);
    }
    assert!(
        world.superweapons.is_ready(kind, owner),
        "superweapon {:?} should be ready after {} ticks",
        kind,
        kind.charge_ticks()
    );
}

#[test]
fn mslo_nuke_kills_enemy_cluster() {
    let mut w = build_arena(11);
    let pids = w.player_ids().to_vec();
    let agent = pids[1];
    let enemy = pids[2];

    // Agent owns a nuclear silo; enemy has a cluster of 5 e1s.
    insert_test_actor(&mut w, make_building(1001, agent, "mslo", (5, 5), 100000));
    let cluster_centre = (25, 25);
    for (i, off) in [(0, 0), (1, 0), (0, 1), (-1, 0), (0, -1)].iter().enumerate() {
        let pos = (cluster_centre.0 + off.0, cluster_centre.1 + off.1);
        insert_test_actor(&mut w, make_e1(2000 + i as u32, enemy, pos, 50000));
    }

    fully_charge(&mut w, SuperweaponKind::Nuke, agent);

    // Nuke not allowed for the enemy (no launcher).
    assert!(w
        .fire_superweapon(SuperweaponKind::Nuke, enemy, Some(cluster_centre), None)
        .is_err());

    // Fire the nuke at the cluster centre.
    let hit = w
        .fire_superweapon(SuperweaponKind::Nuke, agent, Some(cluster_centre), None)
        .expect("nuke should fire");
    assert!(hit >= 5, "nuke should hit at least the 5 e1s, got {hit}");
    // Every e1 in the cluster must be dead (the nuke base damage of
    // 500k against e1 HP of 50k is overkill at every cell within R=4).
    for i in 0..5 {
        assert!(
            read_hp(&w, 2000 + i as u32).map(|hp| hp <= 0).unwrap_or(true),
            "e1 #{i} should be dead after nuke",
        );
    }
    // Charge resets after firing.
    assert!(!w.superweapons.is_ready(SuperweaponKind::Nuke, agent));

    // Sanity: the nuke radius constant matches what we exercised.
    assert!(NUKE_RADIUS_CELLS >= 4);
}

#[test]
fn iron_curtain_makes_friendly_invulnerable() {
    let mut w = build_arena(22);
    let pids = w.player_ids().to_vec();
    let agent = pids[1];
    let enemy = pids[2];

    // Agent: Iron Curtain launcher + a friendly tank that we shield.
    insert_test_actor(&mut w, make_building(1001, agent, "iron", (5, 5), 100000));
    insert_test_actor(&mut w, make_tank(3001, agent, (20, 20), 100000));
    // Bystander enemy e1 (won't actually attack via auto-engage in this
    // simple test; we manually apply a nuke for damage and confirm the
    // tank takes none).
    insert_test_actor(&mut w, make_building(1099, enemy, "mslo", (30, 30), 100000));

    fully_charge(&mut w, SuperweaponKind::IronCurtain, agent);

    // Apply Iron Curtain to the friendly tank.
    let aff = w
        .fire_superweapon(SuperweaponKind::IronCurtain, agent, None, Some(3001))
        .expect("iron curtain should fire");
    assert_eq!(aff, 1, "iron curtain should affect exactly one actor");

    let hp_before = read_hp(&w, 3001).unwrap();

    // Now have the enemy fire their own nuke right on top of the tank.
    fully_charge(&mut w, SuperweaponKind::Nuke, enemy);
    let _ = w
        .fire_superweapon(SuperweaponKind::Nuke, enemy, Some((20, 20)), None)
        .expect("enemy nuke should fire");
    // Tank still alive AND undamaged because it was invulnerable.
    let hp_after = read_hp(&w, 3001).expect("tank should still exist");
    assert_eq!(
        hp_after, hp_before,
        "iron-curtained tank must take zero damage from incoming nuke",
    );
}

#[test]
fn chronosphere_teleports_friendly_unit() {
    let mut w = build_arena(33);
    let pids = w.player_ids().to_vec();
    let agent = pids[1];
    let enemy = pids[2];

    // Agent owns the chronosphere + a tank to teleport. Enemy owns a
    // throwaway tank that we'll try (and fail) to teleport — chrono
    // refuses non-friendly targets.
    insert_test_actor(&mut w, make_building(1001, agent, "pdox", (5, 5), 100000));
    insert_test_actor(&mut w, make_tank(3001, agent, (10, 10), 100000));
    insert_test_actor(&mut w, make_tank(4001, enemy, (15, 15), 100000));

    fully_charge(&mut w, SuperweaponKind::Chronosphere, agent);

    // Refuses to teleport an enemy unit.
    assert!(w
        .fire_superweapon(
            SuperweaponKind::Chronosphere,
            agent,
            Some((30, 30)),
            Some(4001),
        )
        .is_err());

    // Teleport the friendly tank to (30, 30).
    let aff = w
        .fire_superweapon(
            SuperweaponKind::Chronosphere,
            agent,
            Some((30, 30)),
            Some(3001),
        )
        .expect("chrono should fire");
    assert_eq!(aff, 1, "chrono should affect exactly one actor");

    let new_loc = w.actor_location(3001).expect("tank still alive");
    assert_eq!(new_loc, (30, 30), "tank should be at the teleport target cell");
}

#[test]
fn superweapon_requires_launcher() {
    let mut w = build_arena(44);
    let pids = w.player_ids().to_vec();
    let agent = pids[1];
    // No launcher building → fire is rejected with an Err.
    assert!(w
        .fire_superweapon(SuperweaponKind::Nuke, agent, Some((10, 10)), None)
        .is_err());
    assert!(w
        .fire_superweapon(SuperweaponKind::IronCurtain, agent, None, Some(99))
        .is_err());
    assert!(w
        .fire_superweapon(SuperweaponKind::Chronosphere, agent, Some((10, 10)), Some(99))
        .is_err());
}

#[test]
fn superweapon_must_be_charged() {
    let mut w = build_arena(55);
    let pids = w.player_ids().to_vec();
    let agent = pids[1];
    insert_test_actor(&mut w, make_building(1001, agent, "mslo", (5, 5), 100000));
    // Tick once so the timer is registered but NOT done.
    let _ = w.tick(&[]);
    assert!(!w.superweapons.is_ready(SuperweaponKind::Nuke, agent));
    let err = w
        .fire_superweapon(SuperweaponKind::Nuke, agent, Some((10, 10)), None)
        .unwrap_err();
    assert!(err.contains("not charged"), "expected 'not charged' err, got {err:?}");
}
