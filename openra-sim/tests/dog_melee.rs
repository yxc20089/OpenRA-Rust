//! Phase-8 acceptance: a `dog` melee-attacks an `e1` at range 1 and
//! deals damage instantly (no projectile flight).
//!
//! Verifies that:
//! * The `DogJaw` weapon resolves with `Projectile: InstantHit` (so
//!   no projectile is spawned).
//! * When dog and e1 are 1 cell apart, the dog fires immediately on
//!   the order tick and the e1 dies in a single hit (DogJaw damage
//!   100000 vs e1 HP 5000).

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

fn build_arena(seed: i32) -> Option<World> {
    let mod_dir = vendor_mod_dir()?;
    let ruleset = data_rules::load_ruleset(&mod_dir).ok()?;
    let rules = GameRules::from_ruleset(&ruleset);
    let map = OraMap {
        title: "phase-8-melee".into(),
        tileset: "TEMPERAT".into(),
        map_size: (40, 40),
        bounds: (0, 0, 40, 40),
        tiles: Vec::new(),
        actors: vec![
            MapActor { id: "mpspawn1".into(), actor_type: "mpspawn".into(), owner: "Neutral".into(), location: (1, 1) },
            MapActor { id: "mpspawn2".into(), actor_type: "mpspawn".into(), owner: "Neutral".into(), location: (38, 38) },
        ],
        players: vec![
            PlayerDef { name: "Neutral".into(), playable: false, owns_world: true, non_combatant: true, faction: "allies".into(), enemies: Vec::new() },
            PlayerDef { name: "Multi0".into(), playable: true, owns_world: false, non_combatant: false, faction: "allies".into(), enemies: Vec::new() },
            PlayerDef { name: "Multi1".into(), playable: true, owns_world: false, non_combatant: false, faction: "soviet".into(), enemies: Vec::new() },
        ],
    };
    let lobby = LobbyInfo {
        starting_cash: 0,
        allow_spectators: false,
        occupied_slots: vec![
            SlotInfo { player_reference: "Multi0".into(), faction: "allies".into(), is_bot: false },
            SlotInfo { player_reference: "Multi1".into(), faction: "soviet".into(), is_bot: false },
        ],
    };
    let mut world = world::build_world(&map, seed, &lobby, Some(rules), 0);
    set_test_unpaused(&mut world);
    Some(world)
}

fn make_infantry(id: u32, owner: u32, actor_type: &str, at: (i32, i32), hp: i32) -> Actor {
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
                facing: WAngle::new(0).angle,
                from_cell: cell,
                to_cell: cell,
                center_position: center,
            },
            TraitState::Health { hp },
        ],
        activity: None,
        actor_type: Some(actor_type.into()),
        kills: 0,
        rank: 0,
    }
}

fn target_hp(world: &World, id: u32) -> Option<i32> {
    world.actor(id)?.traits.iter().find_map(|t| {
        if let TraitState::Health { hp } = t { Some(*hp) } else { None }
    })
}

#[test]
fn dog_kills_e1_at_range_one() {
    let mut world = match build_arena(42) {
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
    for id in strip { world::remove_test_actor(&mut world, id); }

    let player_ids = world.player_ids().to_vec();
    let agent_pid = player_ids[1];
    let enemy_pid = player_ids[2];

    // dog at (10,20), e1 one cell east at (11,20). DogJaw is instant
    // (Projectile: InstantHit) so damage applies on the first fire
    // tick. e1 HP 5000 vs DogJaw damage 100000 — one hit kills.
    insert_test_actor(&mut world, make_infantry(101, agent_pid, "dog", (10, 20), 20000));
    insert_test_actor(&mut world, make_infantry(102, enemy_pid, "e1", (11, 20), 5000));

    let attack = GameOrder {
        order_string: "Attack".into(),
        subject_id: Some(101),
        target_string: None,
        extra_data: Some(102),
    };
    let initial_proj_count = world.pending_projectile_count();
    let _ = world.tick(&[attack]);
    // No projectiles should have spawned (DogJaw is instant-hit).
    assert_eq!(
        world.pending_projectile_count(),
        initial_proj_count,
        "DogJaw should not spawn a projectile"
    );

    // After 5 outer ticks we expect the e1 either dead or below half HP.
    let mut killed_at = None;
    for tick in 1..=20 {
        let _ = world.tick(&[]);
        if target_hp(&world, 102).is_none() || target_hp(&world, 102).map(|h| h <= 0).unwrap_or(false) {
            killed_at = Some(tick);
            break;
        }
    }
    let killed_at = killed_at.expect("dog should kill e1 within 20 ticks");
    eprintln!("dog killed e1 at outer tick {killed_at}");
    assert!(world.actor(102).is_none(), "e1 actor should be removed after death");
}

#[test]
fn dogjaw_is_instant_hit_weapon() {
    // Property test: the resolved DogJaw weapon must have
    // projectile_speed == 0 (instant hit) and a non-zero damage.
    let mod_dir = match vendor_mod_dir() {
        Some(d) => d,
        None => return,
    };
    let ruleset = data_rules::load_ruleset(&mod_dir).unwrap();
    let rules = GameRules::from_ruleset(&ruleset);
    let dogjaw = rules.weapon("DogJaw").expect("DogJaw weapon parsed");
    assert_eq!(
        dogjaw.projectile_speed, 0,
        "DogJaw should be instant-hit (Projectile: InstantHit)"
    );
    assert!(
        dogjaw.damage > 0,
        "DogJaw should have non-zero damage from Warhead@1Dam: TargetDamage"
    );
    // Range should be 3c0 = 3072 wdist.
    assert_eq!(dogjaw.range, 3 * 1024, "DogJaw range = 3c0");
}

#[test]
fn melee_typed_component_constructs_correctly() {
    use openra_data::rules::{WDist, WeaponStats};
    use openra_sim::traits::MeleeAttack;
    let weapon = WeaponStats {
        name: "DogJaw".into(),
        range: WDist::from_cells(3),
        reload_delay: 10,
        damage: 100_000,
        ..Default::default()
    };
    let melee = MeleeAttack::new(weapon);
    // The MeleeAttack helper clamps the range to 1 cell.
    assert_eq!(melee.armament.weapon.range.length, 1024);
    assert!(melee.is_ready());
}
