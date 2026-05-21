//! Engine acceptance: a *built* `pbox` (pillbox) auto-fires on hostile
//! actors and kills an enemy `e1` in range.
//!
//! RA's `pbox` is an `AttackGarrisoned` defense — in C# its offensive
//! power comes from infantry loaded into its `Cargo`, so the YAML carries
//! NO direct `Armament` trait. The engine does not model garrisoning, so
//! before the `pbox`-weapon fix the auto-target loop's `weapons.first()`
//! returned `None` and a built pbox stood completely inert.
//!
//! `GameRules::from_ruleset` now assigns the canonical RA anti-infantry
//! pillbox weapon `M60mg` to garrison-only ground-turret defenses when
//! they carry no explicit `Armament`. This test pins that: a pbox with
//! NO orders must auto-target and kill an enemy `e1` standing in range.
//!
//! The companion test `pbox_does_not_fire_on_friendly` guards against
//! the auto-target picking a same-owner unit.

use openra_data::oramap::{MapActor, OraMap, PlayerDef};
use openra_data::rules as data_rules;
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::gamerules::GameRules;
use openra_sim::math::{CPos, WPos};
use openra_sim::traits::TraitState;
use openra_sim::world::{
    self, insert_test_actor, set_test_unpaused, LobbyInfo, SlotInfo, World,
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
        title: "pbox-fires".into(),
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
            },
            SlotInfo {
                player_reference: "Multi1".into(),
                faction: "soviet".into(),
                is_bot: false,
            },
        ],
    };
    let mut world = world::build_world(&map, seed, &lobby, Some(rules), 0, false);
    set_test_unpaused(&mut world);
    Some(world)
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

fn make_pbox(id: u32, owner: u32, at: (i32, i32), hp: i32) -> Actor {
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
        actor_type: Some("pbox".into()),
        kills: 0,
        rank: 0,
    }
}

fn read_hp(world: &World, id: u32) -> Option<i32> {
    world.actor(id)?.traits.iter().find_map(|t| {
        if let TraitState::Health { hp } = t { Some(*hp) } else { None }
    })
}

#[test]
fn pbox_has_a_default_direct_fire_weapon() {
    let mod_dir = match vendor_mod_dir() {
        Some(d) => d,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };
    let ruleset = data_rules::load_ruleset(&mod_dir).unwrap();
    let rules = GameRules::from_ruleset(&ruleset);

    // pbox must have at least one weapon resolvable to positive damage.
    let stats = rules.actor("pbox").expect("pbox not parsed");
    let wname = stats
        .weapons
        .first()
        .expect("pbox should have a default direct-fire weapon");
    let w = rules.weapon(wname).expect("pbox weapon must be parsed");
    assert!(w.damage > 0, "pbox weapon damage must be positive (got {})", w.damage);
    assert!(w.range > 0, "pbox weapon range must be positive (got {})", w.range);

    // It must be weaker per-shot than the `gun` turret's TurretGun.
    let turret = rules.weapon("TurretGun").expect("TurretGun not parsed");
    assert!(
        w.damage < turret.damage,
        "pbox weapon ({}) should be weaker per-shot than TurretGun ({})",
        w.damage,
        turret.damage,
    );
}

#[test]
fn pbox_fires_on_enemy_e1_and_kills_it() {
    let mut world = match build_arena(7) {
        Some(w) => w,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };

    // Strip auto-spawned MCVs / spawn markers.
    let strip: Vec<u32> = world::all_actor_ids(&world)
        .into_iter()
        .filter(|&id| matches!(world.actor_kind(id), Some(ActorKind::Mcv) | Some(ActorKind::Spawn)))
        .collect();
    for id in strip {
        world::remove_test_actor(&mut world, id);
    }

    let player_ids = world.player_ids().to_vec();
    let agent_pid = player_ids[1];
    let enemy_pid = player_ids[2];

    // Built pbox owned by the agent; an enemy e1 standing 3 cells away
    // (inside the M60mg 4-cell range). No orders issued to the pbox.
    let e1_hp = 50000;
    insert_test_actor(&mut world, make_pbox(201, agent_pid, (16, 20), 40000));
    insert_test_actor(&mut world, make_e1(202, enemy_pid, (13, 20), e1_hp));

    // Run up to 200 outer ticks; the pbox must kill the e1.
    let mut killed = false;
    for _ in 0..200 {
        let _ = world.tick(&[]);
        if read_hp(&world, 202).map(|hp| hp <= 0).unwrap_or(true) {
            killed = true;
            break;
        }
    }
    assert!(
        killed,
        "built pbox should auto-fire and kill the enemy e1 within 200 ticks (e1 hp now {:?})",
        read_hp(&world, 202),
    );
}

#[test]
fn pbox_does_not_fire_on_friendly() {
    let mut world = match build_arena(8) {
        Some(w) => w,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };

    let strip: Vec<u32> = world::all_actor_ids(&world)
        .into_iter()
        .filter(|&id| matches!(world.actor_kind(id), Some(ActorKind::Mcv) | Some(ActorKind::Spawn)))
        .collect();
    for id in strip {
        world::remove_test_actor(&mut world, id);
    }

    let player_ids = world.player_ids().to_vec();
    let agent_pid = player_ids[1];

    // pbox and e1 both owned by the same player — no friendly fire.
    let e1_hp = 50000;
    insert_test_actor(&mut world, make_pbox(201, agent_pid, (16, 20), 40000));
    insert_test_actor(&mut world, make_e1(202, agent_pid, (13, 20), e1_hp));

    for _ in 0..100 {
        let _ = world.tick(&[]);
    }

    let hp = read_hp(&world, 202).unwrap_or(0);
    assert_eq!(hp, e1_hp, "friendly e1 HP must be unchanged (no friendly fire)");
}
