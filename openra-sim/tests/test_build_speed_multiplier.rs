//! Engine guardrail: per-scenario `build_speed_multiplier` scales the
//! per-tick production-queue advance count.
//!
//! Default `1.0` ⇒ every existing pack inherits the unchanged
//! production rate (an `e1` finishes in roughly the original ~90
//! world-ticks). A scenario declaring `build_speed_multiplier: 4.0`
//! makes production ~4× faster — the same `e1` should be ready in
//! ~22-25 world-ticks. This pins the scale-up direction.
//!
//! The bench-side mirror is
//! `OpenRA-Bench/tests/test_build_speed_multiplier_python.py`.

use openra_data::oramap::{MapActor, OraMap, PlayerDef};
use openra_data::rules as data_rules;
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::gamerules::GameRules;
use openra_sim::math::{CPos, WPos};
use openra_sim::traits::TraitState;
use openra_sim::world::{
    self, insert_test_actor, set_test_cash, set_test_unpaused, GameOrder, LobbyInfo, SlotInfo,
    World,
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
        title: "build-speed-multiplier".into(),
        tileset: "TEMPERAT".into(),
        map_size: (48, 48),
        bounds: (0, 0, 48, 48),
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
                location: (46, 46),
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
                starting_cash: None,
            },
            SlotInfo {
                player_reference: "Multi1".into(),
                faction: "soviet".into(),
                is_bot: false,
                starting_cash: None,
            },
        ],
    };
    let mut world = world::build_world(&map, seed, &lobby, Some(rules), 0, false);
    set_test_unpaused(&mut world);
    Some(world)
}

fn make_building(id: u32, owner: u32, actor_type: &str, at: (i32, i32), hp: i32) -> Actor {
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
        actor_type: Some(actor_type.into()),
        kills: 0,
        rank: 0,
    }
}

fn count_e1(world: &World, pid: u32) -> usize {
    world
        .snapshot()
        .actors
        .iter()
        .filter(|a| a.owner == pid && a.actor_type == "e1")
        .count()
}

/// Queue 1× e1 with the given multiplier; return the world tick at which
/// it appears in the snapshot (the unit spawns at frame-end after the
/// queue completes). `None` ⇒ the e1 never spawned within the budget.
fn ticks_to_first_e1(seed: i32, multiplier: f32, budget_world_ticks: u32) -> Option<u32> {
    let mut world = build_arena(seed)?;
    world.build_speed_multiplier = multiplier;

    let agent_pid = world.player_ids()[1];

    // Base: fact + powr + tent (tent provides the `barracks` prereq).
    insert_test_actor(&mut world, make_building(8001, agent_pid, "fact", (10, 10), 1000));
    insert_test_actor(&mut world, make_building(8002, agent_pid, "powr", (14, 10), 200));
    insert_test_actor(&mut world, make_building(8003, agent_pid, "tent", (14, 14), 800));

    set_test_cash(&mut world, agent_pid, 10_000);

    // Queue one e1.
    let orders = vec![GameOrder {
        order_string: "StartProduction".into(),
        subject_id: Some(agent_pid),
        target_string: Some("e1".into()),
        extra_data: None,
    }];
    world.process_frame(&orders);

    // Step one frame at a time (NetFrameInterval = 3 world ticks per
    // frame) until the e1 appears in the snapshot, or we exhaust the
    // budget. `process_frame` advances `world.world_tick` by 3 every
    // call.
    let budget_frames = budget_world_ticks.div_ceil(3) + 2;
    for _ in 0..budget_frames {
        world.process_frame(&[]);
        if count_e1(&world, agent_pid) > 0 {
            return Some(world.world_tick);
        }
    }
    None
}

/// Default multiplier (1.0) preserves the legacy build time. An e1
/// (cost 100, build duration ~ cost * 60 / 100 = 60 internal ticks
/// modelled through the queue tick path, surfacing as ~90 world-ticks
/// of `process_frame` advancement once production-bot scheduling and
/// the per-frame multiple advances are accounted for) is expected to
/// finish in the 60-120 world-tick band. The exact value is engine-
/// internal; we just pin a band that REQUIRES the multiplier-1.0
/// branch to keep behaviour stable.
#[test]
fn default_multiplier_preserves_build_time() {
    let t = match ticks_to_first_e1(1, 1.0, 300) {
        Some(t) => t,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };
    eprintln!("default multiplier 1.0: e1 ready at world_tick {t}");
    assert!(
        (40..=180).contains(&t),
        "default 1.0× multiplier must keep e1 build time in the 40-180 world-tick band \
         (got {t}); something changed in production tick semantics"
    );
}

/// 4× multiplier scenario (the `adversarial-1v1-macro` setting):
/// production must finish in roughly a quarter of the default time.
/// We bracket the expected window loosely (10-35 world ticks) — the
/// exact ratio depends on integer rounding inside the queue tick path.
#[test]
fn four_x_multiplier_quarters_build_time() {
    let t_default = match ticks_to_first_e1(2, 1.0, 300) {
        Some(t) => t,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };
    let t_fast = ticks_to_first_e1(2, 4.0, 300)
        .expect("4× scenario must finish e1 inside the 300-tick budget");
    eprintln!(
        "multiplier 1.0 ⇒ e1 ready at {t_default}; multiplier 4.0 ⇒ {t_fast}"
    );

    // 4× faster: at least 2× faster (gives integer-rounding slack).
    assert!(
        (t_fast as f32) <= (t_default as f32) / 2.0,
        "4× multiplier must finish in <= half the default time (default={t_default}, fast={t_fast})",
    );
    // And actually somewhere in the ~22-tick neighbourhood — give the
    // band 10..=35 world ticks of slack.
    assert!(
        (10..=35).contains(&t_fast),
        "4× multiplier should finish an e1 in ~22 world ticks (got {t_fast})",
    );
}

/// Half-speed multiplier (0.5) approximately doubles the build time.
/// This pins the slow-down direction of the same accumulator path.
#[test]
fn half_multiplier_doubles_build_time() {
    let t_default = match ticks_to_first_e1(3, 1.0, 400) {
        Some(t) => t,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };
    let t_slow = ticks_to_first_e1(3, 0.5, 400)
        .expect("0.5× scenario must finish e1 inside the 400-tick budget");
    eprintln!(
        "multiplier 1.0 ⇒ e1 ready at {t_default}; multiplier 0.5 ⇒ {t_slow}"
    );
    assert!(
        (t_slow as f32) >= 1.5 * (t_default as f32),
        "0.5× multiplier must finish in >= 1.5× the default time (default={t_default}, slow={t_slow})",
    );
}
