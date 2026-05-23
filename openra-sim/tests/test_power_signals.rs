//! Engine guardrail: `power_provided` / `power_drained` in the player
//! snapshot must reflect PRE-PLACED buildings (not just buildings
//! created by `order_place_building` mid-episode), AND the `PowerDown`
//! order must toggle a building's contribution.
//!
//! Before the recompute-at-snapshot fix `update_player_power` was only
//! called from `order_place_building` — pre-placed scenario buildings
//! bypassed it, so `power_surplus_gte` / `power_provided_gte` were
//! inert in scenarios. This test pins the fix in place on the Rust
//! side; the bench-side mirror is
//! `OpenRA-Bench/tests/test_power_signals_python.py`.

use openra_data::oramap::{MapActor, OraMap, PlayerDef};
use openra_data::rules as data_rules;
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::gamerules::GameRules;
use openra_sim::math::{CPos, WPos};
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
        title: "power-signals".into(),
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

fn make_building(
    id: u32,
    owner: u32,
    actor_type: &str,
    at: (i32, i32),
    hp: i32,
) -> Actor {
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

fn snapshot_power(world: &World, pid: u32) -> (i32, i32) {
    let snap = world.snapshot();
    let ps = snap
        .players
        .iter()
        .find(|p| p.index == pid)
        .expect("player snapshot missing");
    (ps.power_provided, ps.power_drained)
}

#[test]
fn pre_placed_buildings_surface_in_player_snapshot_power_totals() {
    let mut world = match build_arena(42) {
        Some(w) => w,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };

    let agent_pid = world.player_ids()[1];

    // Pre-place a powr (+100) and a proc (-30) for the agent — never
    // ordered via place_building, never touched update_player_power.
    insert_test_actor(&mut world, make_building(9001, agent_pid, "powr", (10, 10), 200));
    insert_test_actor(&mut world, make_building(9002, agent_pid, "proc", (14, 10), 300));

    let (provided, drained) = snapshot_power(&world, agent_pid);
    assert!(
        provided > 0,
        "pre-placed powr must surface power_provided > 0; got {provided}"
    );
    assert!(
        drained > 0,
        "pre-placed proc must surface power_drained > 0; got {drained}"
    );

    // The exact values come from the ruleset (powr = +100, proc = -30).
    assert_eq!(provided, 100, "powr power_provided expected 100");
    assert_eq!(drained, 30, "proc power_drained expected 30");
}

#[test]
fn power_down_order_toggles_building_contribution() {
    let mut world = match build_arena(43) {
        Some(w) => w,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };
    let agent_pid = world.player_ids()[1];

    insert_test_actor(&mut world, make_building(9101, agent_pid, "powr", (10, 10), 200));
    insert_test_actor(&mut world, make_building(9102, agent_pid, "powr", (14, 10), 200));
    insert_test_actor(&mut world, make_building(9103, agent_pid, "proc", (18, 10), 300));

    // Baseline: 2 × powr (+100 each) provided, 1 × proc (-30) drained.
    let (p0, d0) = snapshot_power(&world, agent_pid);
    assert_eq!(p0, 200, "baseline provided 200 expected");
    assert_eq!(d0, 30, "baseline drained 30 expected");

    // Power-down one powr.
    let order = GameOrder {
        order_string: "PowerDown".into(),
        subject_id: Some(9101),
        target_string: None,
        extra_data: None,
    };
    world.process_frame(&[order]);

    let (p1, d1) = snapshot_power(&world, agent_pid);
    assert_eq!(
        p1, 100,
        "after PowerDown one powr, provided should drop by 100 (got {p1})"
    );
    assert_eq!(d1, 30, "drained should be unchanged (got {d1})");

    // Toggle the same powr back on.
    let order = GameOrder {
        order_string: "PowerDown".into(),
        subject_id: Some(9101),
        target_string: None,
        extra_data: None,
    };
    world.process_frame(&[order]);

    let (p2, d2) = snapshot_power(&world, agent_pid);
    assert_eq!(p2, 200, "toggling PowerDown back on should restore +100");
    assert_eq!(d2, 30, "drained should be unchanged on toggle");
}
