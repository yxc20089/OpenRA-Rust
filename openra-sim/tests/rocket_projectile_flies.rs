//! Phase-8 acceptance: a `RedEye` rocket fired from `e3` flies for
//! several inner ticks before impacting and damaging the target.
//!
//! Verifies that:
//! * After the e3 fires, at least one `Projectile` exists in the
//!   world's `pending_projectiles` map.
//! * Damage is NOT applied immediately on the firing tick (target HP
//!   unchanged for the first one or two outer ticks).
//! * Eventually the projectile arrives and the target HP drops.

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
        title: "phase-8".into(),
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

fn make_e3(id: u32, owner: u32, at: (i32, i32), hp: i32) -> Actor {
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
        actor_type: Some("e3".into()),
        kills: 0,
        rank: 0,
    }
}

fn make_e1_target(id: u32, owner: u32, at: (i32, i32), hp: i32) -> Actor {
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
        actor_type: Some("e1".into()),
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
fn e3_redeye_projectile_flies_before_impacting() {
    let mut world = match build_arena(42) {
        Some(w) => w,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };
    // Strip auto-spawned MCVs.
    let strip: Vec<u32> = world::all_actor_ids(&world)
        .into_iter()
        .filter(|&id| matches!(world.actor_kind(id), Some(ActorKind::Mcv) | Some(ActorKind::Spawn)))
        .collect();
    for id in strip { world::remove_test_actor(&mut world, id); }

    let player_ids = world.player_ids().to_vec();
    let agent_pid = player_ids[1];
    let enemy_pid = player_ids[2];

    // e3 (rocket soldier) at (10,20), e1 target at (15,20). 5 cells
    // east. RedEye's resolved Range falls back to `^AntiGroundMissile`'s
    // 5c0 (the `Nike` parent isn't in the abstract-only lookup), so we
    // place the target right at the range limit.
    insert_test_actor(&mut world, make_e3(101, agent_pid, (10, 20), 50000));
    insert_test_actor(&mut world, make_e1_target(102, enemy_pid, (15, 20), 50000));

    let attack_order = GameOrder {
        order_string: "Attack".into(),
        subject_id: Some(101),
        target_string: None,
        extra_data: Some(102),
    };

    // Tick 1: order is processed; the e3 fires its rocket. Damage
    // should NOT yet apply — the projectile must be in flight.
    let initial_hp = target_hp(&world, 102).expect("target alive at start");
    let _ = world.tick(&[attack_order]);

    // Confirm a projectile spawned (RedEye Speed is 298 wdist/tick;
    // distance is 8 cells = 8192 wdist → ~28 inner ticks of flight).
    // After the first outer tick (3 inner ticks ≈ 894 wdist travelled),
    // the rocket should still be airborne.
    assert!(
        world.pending_projectile_count() >= 1,
        "expected a projectile in flight after firing, got {}",
        world.pending_projectile_count()
    );
    assert_eq!(
        target_hp(&world, 102),
        Some(initial_hp),
        "target HP should not change while projectile is mid-flight"
    );

    // Run more ticks until impact. At RedEye Speed=298 vs 8c0 distance,
    // ~28 inner ticks = ~10 outer ticks of flight, so 30 outer ticks is
    // plenty of headroom.
    let mut hp_dropped = false;
    let mut hit_tick = None;
    for tick in 2..=30 {
        let _ = world.tick(&[]);
        if target_hp(&world, 102).map(|h| h < initial_hp).unwrap_or(true) {
            hp_dropped = true;
            hit_tick = Some(tick);
            break;
        }
    }
    assert!(hp_dropped, "expected target HP to drop after rocket flight");
    eprintln!("rocket impacted at outer tick {hit_tick:?}");
}
