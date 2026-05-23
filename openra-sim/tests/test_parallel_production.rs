//! Engine guardrail: multiple production buildings of the same category
//! must produce IN PARALLEL (OpenRA parity).
//!
//! Before the parallel-production fix the engine modelled production as
//! ONE per-player queue per category — a 2nd war factory added zero
//! throughput. Real OpenRA produces concurrently from every completed,
//! powered production building of that category, so two war factories
//! roughly double vehicle output.
//!
//! This test pins the fix: queueing 2tnk with TWO `weap` buildings must
//! finish meaningfully faster (>= 1.6x the tanks in a fixed tick budget)
//! than the same queue with ONE `weap`. The bench-side mirror is
//! `OpenRA-Bench/tests/test_parallel_production.py`.

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
        title: "parallel-production".into(),
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

/// Count `2tnk` actors owned by `pid` in the current snapshot.
fn count_tanks(world: &World, pid: u32) -> usize {
    world
        .snapshot()
        .actors
        .iter()
        .filter(|a| a.owner == pid && a.actor_type == "2tnk")
        .count()
}

/// Build an arena, pre-place a base + `weap_count` war factories for the
/// agent, give it `cash`, queue `queue_n` × 2tnk, run `ticks` frames and
/// return the number of finished tanks.
fn run_throughput(seed: i32, weap_count: u32, cash: i32, queue_n: u32, ticks: u32) -> Option<usize> {
    let mut world = build_arena(seed)?;
    let agent_pid = world.player_ids()[1];

    // Base + power so production is not low-power throttled.
    insert_test_actor(&mut world, make_building(8001, agent_pid, "fact", (10, 10), 1000));
    insert_test_actor(&mut world, make_building(8002, agent_pid, "powr", (14, 10), 200));
    insert_test_actor(&mut world, make_building(8003, agent_pid, "powr", (16, 10), 200));
    insert_test_actor(&mut world, make_building(8004, agent_pid, "powr", (18, 10), 200));
    // Service depot — `2tnk` requires `fix` as a prerequisite.
    insert_test_actor(&mut world, make_building(8005, agent_pid, "fix", (10, 14), 800));

    // The war factories of the agent.
    for i in 0..weap_count {
        insert_test_actor(&mut world, make_building(
            8100 + i,
            agent_pid,
            "weap",
            (20 + 3 * i as i32, 14),
            1000,
        ));
    }

    // Cash for the whole order.
    set_test_cash(&mut world, agent_pid, cash);

    // Queue queue_n × 2tnk.
    let orders: Vec<GameOrder> = (0..queue_n)
        .map(|_| GameOrder {
            order_string: "StartProduction".into(),
            subject_id: Some(agent_pid),
            target_string: Some("2tnk".into()),
            extra_data: None,
        })
        .collect();
    world.process_frame(&orders);

    for _ in 0..ticks {
        world.process_frame(&[]);
    }
    Some(count_tanks(&world, agent_pid))
}

#[test]
fn two_war_factories_roughly_double_vehicle_throughput() {
    // 2tnk costs 850; build time ≈ 510 world-ticks each. `process_frame`
    // advances 3 world-ticks (NetFrameInterval=3), so one factory clears
    // ~1 tank per 170 frames. Queue 8 and use a frame budget where ONE
    // factory finishes only ~3 tanks but TWO finish ~6 — the budget must
    // bite for the single-factory baseline. Give plenty of cash so the
    // bottleneck is build TIME, not money.
    const QUEUE_N: u32 = 8;
    const CASH: i32 = 40_000;
    const TICKS: u32 = 560;

    let one = match run_throughput(1, 1, CASH, QUEUE_N, TICKS) {
        Some(n) => n,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };
    let two = run_throughput(1, 2, CASH, QUEUE_N, TICKS).expect("arena built once already");

    eprintln!("1 weap finished {one} tanks; 2 weap finished {two} tanks in {TICKS} ticks");

    assert!(
        one >= 1,
        "sanity: a single war factory should finish at least one tank in {TICKS} ticks (got {one})"
    );
    assert!(
        two >= 1,
        "two war factories should finish at least one tank (got {two})"
    );
    // Parallel production: two factories must roughly double output.
    assert!(
        two as f64 >= 1.6 * one as f64,
        "two war factories must produce >= 1.6x the tanks of one \
         (got 2-weap={two} vs 1-weap={one}); parallel production is not working"
    );
}

#[test]
fn three_war_factories_beat_two() {
    const QUEUE_N: u32 = 12;
    const CASH: i32 = 60_000;
    const TICKS: u32 = 560;

    let two = match run_throughput(2, 2, CASH, QUEUE_N, TICKS) {
        Some(n) => n,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };
    let three = run_throughput(2, 3, CASH, QUEUE_N, TICKS).expect("arena built once already");

    eprintln!("2 weap finished {two} tanks; 3 weap finished {three} tanks");
    assert!(
        three > two,
        "three war factories must out-produce two (got 3-weap={three} vs 2-weap={two})"
    );
}
