//! Engine acceptance: a scenario-placed ore patch + harvester + proc
//! drives a full harvest → deposit → cash loop with no agent orders.
//!
//! Pins the integration of three pieces:
//!
//! * `openra_sim::resource::seed_ore_patch` materialises a disk of
//!   harvestable ore on the live terrain.
//! * `World::auto_route_idle_harvesters` (per-tick) auto-issues a
//!   `Harvest` activity to any idle owned harvester whose owner has
//!   a `proc`. This is what makes a scenario-placed harv actually
//!   mine — `build_scenario_actor` injects harvs with `activity:
//!   None`, so without the auto-route they would sit inert.
//! * The pre-existing `tick_harvesters` FSM walks the harv through
//!   FindingOre → MovingToOre → Harvesting → MovingToRefinery →
//!   Unloading and deposits resources into the player's stored
//!   pool, which the existing per-tick drain converts to cash.

use openra_data::oramap::{MapActor, OraMap, PlayerDef};
use openra_data::rules as data_rules;
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::gamerules::GameRules;
use openra_sim::math::{CPos, WPos};
use openra_sim::resource::{seed_ore_patch, total_ore_density, OrePatch};
use openra_sim::terrain::ResourceType;
use openra_sim::traits::TraitState;
use openra_sim::world::{
    self, insert_test_actor, set_test_unpaused, LobbyInfo, SlotInfo, World,
};
use std::path::PathBuf;

fn vendor_mod_dir() -> Option<PathBuf> {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let p = PathBuf::from(format!("{manifest}/../vendor/OpenRA/mods/ra"));
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

fn build_arena(seed: i32) -> Option<(World, u32, u32)> {
    let mod_dir = vendor_mod_dir()?;
    let ruleset = data_rules::load_ruleset(&mod_dir).ok()?;
    let rules = GameRules::from_ruleset(&ruleset);

    let map = OraMap {
        title: "resource-layer".into(),
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
    // Strip auto-seeded MCVs/spawn markers so they don't interfere.
    let strip: Vec<u32> = world::all_actor_ids(&world)
        .into_iter()
        .filter(|&id| {
            matches!(
                world.actor_kind(id),
                Some(ActorKind::Mcv) | Some(ActorKind::Spawn)
            )
        })
        .collect();
    for id in strip {
        world::remove_test_actor(&mut world, id);
    }
    set_test_unpaused(&mut world);

    let player_ids = world.player_ids().to_vec();
    let agent_pid = player_ids[1];
    let enemy_pid = player_ids[2];
    Some((world, agent_pid, enemy_pid))
}

fn make_harv(id: u32, owner: u32, at: (i32, i32)) -> Actor {
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
                facing: 512,
                from_cell: cell,
                to_cell: cell,
                center_position: center,
            },
            TraitState::Health { hp: 100000 },
        ],
        activity: None,
        actor_type: Some("harv".into()),
        kills: 0,
        rank: 0,
    }
}

fn make_proc(id: u32, owner: u32, at: (i32, i32)) -> Actor {
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
            TraitState::Immobile {
                top_left: cell,
                center_position: center,
            },
            TraitState::Health { hp: 100000 },
        ],
        activity: None,
        actor_type: Some("proc".into()),
        kills: 0,
        rank: 0,
    }
}

#[test]
fn seed_ore_patch_writes_to_terrain() {
    let mut t = openra_sim::terrain::TerrainMap::new(20, 20);
    let placed = seed_ore_patch(
        &mut t,
        OrePatch {
            x: 10,
            y: 10,
            amount: 500,
            radius: 2,
        },
    );
    assert!(placed > 0, "patch should fill cells");
    assert!(t.has_resource(10, 10));
    let cell = t.resource(10, 10);
    assert_eq!(cell.resource_type, ResourceType::Ore);
    assert!(cell.density >= 1);
}

#[test]
fn scenario_harvester_cycles_and_grows_cash() {
    let (mut world, agent_pid, _enemy_pid) = match build_arena(11) {
        Some(t) => t,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };

    // Seed an ore patch near (12, 12) and place the agent's refinery
    // and harvester tightly around it so the round-trip is short and
    // the test runs in well under 1000 ticks.
    seed_ore_patch(
        &mut world.terrain,
        OrePatch {
            x: 12,
            y: 12,
            amount: 4000,
            radius: 2,
        },
    );
    let initial_ore_density = total_ore_density(&world.terrain);
    assert!(initial_ore_density > 0);

    // Refinery 4 cells away from the patch center; harvester
    // immediately adjacent to the patch.
    insert_test_actor(&mut world, make_proc(301, agent_pid, (16, 12)));
    insert_test_actor(&mut world, make_harv(302, agent_pid, (10, 12)));

    // The harv is scenario-placed → has Activity::None. The per-tick
    // `auto_route_idle_harvesters` should install a Harvest activity
    // the very next tick.
    let cash_before = world.actor(agent_pid).map(|a| a.cash()).unwrap_or(-1);
    let resources_before = world.actor(agent_pid).map(|a| a.resources()).unwrap_or(-1);

    // Drive the world forward. Within a few hundred ticks we expect
    // (a) the harv to have engaged Harvest, (b) the ore patch density
    // to drop, and (c) cash to grow above its starting value.
    let mut got_harvest_activity = false;
    let mut got_unloading_or_carry = false;
    for _ in 0..1500 {
        let _ = world.tick(&[]);
        if let Some(a) = world.actor(302) {
            use openra_sim::actor::Activity;
            if matches!(a.activity, Some(Activity::Harvest { .. })) {
                got_harvest_activity = true;
                if let Some(Activity::Harvest {
                    carried_ore,
                    state,
                    ..
                }) = &a.activity
                {
                    use openra_sim::actor::HarvestState;
                    if *carried_ore > 0
                        || matches!(
                            state,
                            HarvestState::Unloading | HarvestState::MovingToRefinery
                        )
                    {
                        got_unloading_or_carry = true;
                    }
                }
            }
        }
        let cash_now = world.actor(agent_pid).map(|a| a.cash()).unwrap_or(0);
        if cash_now > cash_before + 100 {
            break;
        }
    }

    assert!(
        got_harvest_activity,
        "scenario-placed harv should auto-route into Harvest activity within 1500 ticks"
    );
    assert!(
        got_unloading_or_carry,
        "harv should reach carrying or delivering state"
    );

    let cash_after = world.actor(agent_pid).map(|a| a.cash()).unwrap_or(0);
    let resources_after = world
        .actor(agent_pid)
        .map(|a| a.resources())
        .unwrap_or(0);
    let final_density = total_ore_density(&world.terrain);

    assert!(
        cash_after > cash_before,
        "cash should grow from harvest (before={}, after={}, stored before/after={}/{})",
        cash_before,
        cash_after,
        resources_before,
        resources_after,
    );
    assert!(
        final_density < initial_ore_density,
        "ore patch should deplete (before={}, after={})",
        initial_ore_density,
        final_density,
    );
}

#[test]
fn refinery_far_from_ore_is_slower_than_refinery_near_ore() {
    // Placement matters: a `proc` adjacent to the ore patch produces
    // income faster than one placed far away (round-trip is shorter).
    // We run two identical worlds for a fixed budget and assert the
    // "near" arrangement yields strictly more cash.

    fn run(refinery_at: (i32, i32), harv_at: (i32, i32), ticks: u32) -> (i32, i32) {
        let (mut world, agent_pid, _) = build_arena(17).expect("vendor dir present");
        seed_ore_patch(
            &mut world.terrain,
            OrePatch {
                x: 12,
                y: 12,
                amount: 4000,
                radius: 2,
            },
        );
        insert_test_actor(&mut world, make_proc(401, agent_pid, refinery_at));
        insert_test_actor(&mut world, make_harv(402, agent_pid, harv_at));
        for _ in 0..ticks {
            let _ = world.tick(&[]);
        }
        let cash = world.actor(agent_pid).map(|a| a.cash()).unwrap_or(0);
        let stored = world.actor(agent_pid).map(|a| a.resources()).unwrap_or(0);
        (cash, stored)
    }

    if vendor_mod_dir().is_none() {
        eprintln!("skipping: vendored OpenRA mod dir not found");
        return;
    }

    // Near: refinery one tile east of the ore patch center.
    // Far:  refinery at the opposite corner of the map.
    let (near_cash, near_stored) = run((16, 12), (10, 12), 1200);
    let (far_cash, far_stored) = run((36, 36), (10, 12), 1200);

    // Final economy value = cash + stored (the per-tick drain converts
    // stored → cash slowly; comparing only `cash` would penalise the
    // near run if the drain ran fewer cycles).
    let near_total = near_cash + near_stored;
    let far_total = far_cash + far_stored;
    assert!(
        near_total > far_total,
        "near refinery should out-earn far refinery (near total={}, far total={})",
        near_total,
        far_total,
    );
}
