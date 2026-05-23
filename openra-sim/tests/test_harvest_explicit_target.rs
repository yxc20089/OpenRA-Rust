//! Pin (currently FAILING / `#[ignore]`d): explicit `Harvest(target)`
//! orders should keep the harvester bound to the patch around the
//! authored target, not drift to whatever ore is closest to its
//! current position.
//!
//! Bug: `tick_harvesters::FindingOre` calls
//! `terrain.find_nearest_resource(center, 15)` where `center =
//! last_harvest_cell.or(loc)`. The first scan honours the explicit
//! target (set as `last_harvest_cell` by `order_harvest`). After the
//! first deposit, `last_harvest_cell` is updated to the actually-
//! harvested cell — so subsequent FindingOre cycles drift toward
//! whatever patch is densest within 15 cells of the harv's last
//! mining position. With two patches inside a 15-cell window of each
//! other, the explicit allocation evaporates within ~30 ticks.
//!
//! Spec: respect a `bound_patch: Option<(x, y, radius)>` field on
//! `Activity::Harvest` (set by `order_harvest` from the explicit
//! target, derived from the registered `OrePatchDef` containing the
//! cell), and have FindingOre's resource search restrict to cells
//! inside the patch disc.
//!
//! Triaged in ENGINE_FOLLOWUPS_TRIAGE.md finding #4. The test is
//! `#[ignore]`d until the fix lands so `cargo test` stays green;
//! flip to `#[test]` after `bound_patch` ships.

use openra_data::oramap::{MapActor, OraMap, PlayerDef};
use openra_data::rules as data_rules;
use openra_sim::actor::{Actor, ActorKind, Activity, HarvestState};
use openra_sim::gamerules::GameRules;
use openra_sim::math::{CPos, WPos};
use openra_sim::resource::{seed_ore_patch, OrePatch};
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

fn build_arena(seed: i32) -> Option<(World, u32)> {
    let mod_dir = vendor_mod_dir()?;
    let ruleset = data_rules::load_ruleset(&mod_dir).ok()?;
    let rules = GameRules::from_ruleset(&ruleset);

    let map = OraMap {
        title: "harv-bind-test".into(),
        tileset: "TEMPERAT".into(),
        map_size: (60, 30),
        bounds: (0, 0, 60, 30),
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
                location: (58, 28),
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
                enemies: vec!["Multi1".into()],
            },
            PlayerDef {
                name: "Multi1".into(),
                playable: true,
                owns_world: false,
                non_combatant: false,
                faction: "soviet".into(),
                enemies: vec!["Multi0".into()],
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
    let mut w = world::build_world(&map, seed, &lobby, Some(rules), 0, false);
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
    set_test_unpaused(&mut w);
    let pids = w.player_ids().to_vec();
    Some((w, pids[1]))
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
                facing: 0,
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
            TraitState::Immobile { top_left: cell, center_position: center },
            TraitState::Health { hp: 100000 },
        ],
        activity: None,
        actor_type: Some("proc".into()),
        kills: 0,
        rank: 0,
    }
}

#[test]
#[ignore = "TODO ENGINE_FOLLOWUPS_TRIAGE finding #4: harv drifts to nearest patch after first deposit; needs bound_patch on Activity::Harvest"]
fn explicit_harvest_target_stays_bound_to_far_patch() {
    let (mut w, agent) = match build_arena(11) {
        Some(t) => t,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };

    // Two patches: NEAR (10,15) radius 3, FAR (40,15) radius 3.
    // Proc at (5,15) — closer to NEAR patch.
    // Harvester at (25,15) — between the two patches but slightly
    // closer to NEAR patch's edge.
    seed_ore_patch(&mut w.terrain, OrePatch { x: 10, y: 15, amount: 5000, radius: 3 });
    seed_ore_patch(&mut w.terrain, OrePatch { x: 40, y: 15, amount: 5000, radius: 3 });
    insert_test_actor(&mut w, make_proc(401, agent, (5, 15)));
    insert_test_actor(&mut w, make_harv(402, agent, (25, 15)));

    // Issue an explicit Harvest order targeting the FAR patch.
    // (We can't easily route through Command::Harvest from here,
    // so set the activity directly — equivalent to what
    // `order_harvest` does.)
    {
        let actor = w.actor_mut(402).expect("harv exists");
        actor.activity = Some(Activity::Harvest {
            state: HarvestState::FindingOre,
            refinery_id: 401,
            carried_ore: 0,
            carried_gems: 0,
            capacity: 20,
            path: Vec::new(),
            path_index: 0,
            speed: 56,
            harvest_ticks: 0,
            last_harvest_cell: Some((40, 15)), // explicit target
        });
    }

    // Drive the world forward and record the cells where the
    // harvester actually mines from.
    let mut harvest_cells: Vec<(i32, i32)> = Vec::new();
    let mut prev_carried = 0i32;
    let mut deposits = 0u32;
    let mut prev_cash = w.actor(agent).map(|a| a.cash()).unwrap_or(0);
    for _ in 0..3000 {
        let _ = w.tick(&[]);
        if let Some(a) = w.actor(402) {
            if let Some(Activity::Harvest { carried_ore, last_harvest_cell, .. }) = &a.activity {
                if *carried_ore > prev_carried {
                    if let Some(c) = last_harvest_cell {
                        harvest_cells.push(*c);
                    }
                }
                prev_carried = *carried_ore;
            }
        }
        let cash_now = w.actor(agent).map(|a| a.cash()).unwrap_or(0);
        if cash_now > prev_cash {
            deposits += 1;
            prev_cash = cash_now;
        }
        if deposits >= 3 {
            break;
        }
    }

    assert!(
        deposits >= 3,
        "harvester should deliver at least 3 deposits; got {} (cells={:?})",
        deposits, harvest_cells,
    );

    // Every harvested cell should be within Chebyshev distance ≤ 4
    // of the FAR patch centre (40, 15). The clear-cut failure
    // signature is: cells close to the NEAR patch (10, 15) appear,
    // proving the harv has drifted off the explicit target.
    let drifted: Vec<(i32, i32)> = harvest_cells
        .iter()
        .filter(|(x, y)| (*x - 40).abs().max((*y - 15).abs()) > 4)
        .copied()
        .collect();
    assert!(
        drifted.is_empty(),
        "harvester drifted off the explicit FAR-patch target: cells \
         {:?} are >4 cells from (40,15)",
        drifted,
    );
}
