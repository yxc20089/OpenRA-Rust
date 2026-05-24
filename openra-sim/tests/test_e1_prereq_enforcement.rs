//! Engine guardrail: `StartProduction` for `e1` (and other units with a
//! `~barracks` prerequisite) must be BLOCKED when the player owns no
//! building providing `barracks` (no `tent` / `barr`).
//!
//! Vendor RA `infantry.yaml` declares `e1.Buildable.Prerequisites: ~barracks,
//! ~techlevel.infonly`. The `~` prefix only hides the entry from the build
//! palette UI — it does NOT make the prerequisite optional. A player with no
//! `barracks`-providing building must NOT be able to train `e1`.
//!
//! `has_prerequisites` (world.rs) strips the `~` prefix, skips
//! `~techlevel.*`, and checks the residual prerequisite against the
//! player's virtual-prerequisite set (`compute_player_prerequisites`,
//! seeded from each owned building's `ProvidesPrerequisite` traits). This
//! test pins the bench-relevant end-to-end behaviour: a `fact`-only base
//! cannot train `e1`; adding a `tent` (which provides `barracks`) unblocks
//! it. The bench-side mirror is
//! `OpenRA-Bench/tests/test_e1_prereq_python.py`.

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
        title: "e1-prereq".into(),
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

/// A player with `fact` + `powr` (no `tent`, no `barr`) must NOT be able
/// to train `e1` — the `~barracks` prerequisite has no provider.
#[test]
fn e1_blocked_without_barracks() {
    let mut world = match build_arena(1) {
        Some(w) => w,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };
    let agent_pid = world.player_ids()[1];

    // Base WITHOUT a barracks (no tent / no barr).
    insert_test_actor(&mut world, make_building(8001, agent_pid, "fact", (10, 10), 1000));
    insert_test_actor(&mut world, make_building(8002, agent_pid, "powr", (14, 10), 200));

    set_test_cash(&mut world, agent_pid, 10_000);

    // Spam StartProduction for e1.
    let orders: Vec<GameOrder> = (0..5)
        .map(|_| GameOrder {
            order_string: "StartProduction".into(),
            subject_id: Some(agent_pid),
            target_string: Some("e1".into()),
            extra_data: None,
        })
        .collect();
    world.process_frame(&orders);

    // Tick long enough for any queued e1 to finish (build time ~60 ticks).
    for _ in 0..200 {
        world.process_frame(&[]);
    }

    let n = count_e1(&world, agent_pid);
    assert_eq!(
        n, 0,
        "without a barracks-providing building, no e1 may spawn (got {n})",
    );
}

/// Sanity baseline: the same setup but WITH a `tent` (provides `barracks`)
/// — `e1` production must succeed. This pins that the block above is
/// specifically the prerequisite gate, not some unrelated production
/// failure.
#[test]
fn e1_succeeds_with_tent() {
    let mut world = match build_arena(2) {
        Some(w) => w,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };
    let agent_pid = world.player_ids()[1];

    insert_test_actor(&mut world, make_building(8001, agent_pid, "fact", (10, 10), 1000));
    insert_test_actor(&mut world, make_building(8002, agent_pid, "powr", (14, 10), 200));
    // ADD a tent (provides barracks).
    insert_test_actor(&mut world, make_building(8003, agent_pid, "tent", (14, 14), 800));

    set_test_cash(&mut world, agent_pid, 10_000);

    let orders: Vec<GameOrder> = (0..5)
        .map(|_| GameOrder {
            order_string: "StartProduction".into(),
            subject_id: Some(agent_pid),
            target_string: Some("e1".into()),
            extra_data: None,
        })
        .collect();
    world.process_frame(&orders);

    for _ in 0..200 {
        world.process_frame(&[]);
    }

    let n = count_e1(&world, agent_pid);
    assert!(
        n >= 1,
        "with a tent (barracks), e1 production must succeed (got {n})",
    );
}

/// Sister units with `~barr` (heavy infantry / dog / spy / engineer-tier)
/// must also be blocked without `barr` — the parser strips both `~barr`
/// and `~barracks` consistently. Pick `e3` (rocket soldier) which has
/// `Prerequisites: ~barracks, ~techlevel.infonly` in vendor — same gate
/// as `e1`. A `tent`-only base must still satisfy it (tent provides
/// `barracks`).
#[test]
fn e3_blocked_without_barracks_too() {
    let mut world = match build_arena(3) {
        Some(w) => w,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };
    let agent_pid = world.player_ids()[1];

    insert_test_actor(&mut world, make_building(8001, agent_pid, "fact", (10, 10), 1000));
    insert_test_actor(&mut world, make_building(8002, agent_pid, "powr", (14, 10), 200));

    set_test_cash(&mut world, agent_pid, 10_000);

    let orders: Vec<GameOrder> = (0..5)
        .map(|_| GameOrder {
            order_string: "StartProduction".into(),
            subject_id: Some(agent_pid),
            target_string: Some("e3".into()),
            extra_data: None,
        })
        .collect();
    world.process_frame(&orders);

    for _ in 0..200 {
        world.process_frame(&[]);
    }

    let n = world.snapshot().actors.iter()
        .filter(|a| a.owner == agent_pid && a.actor_type == "e3").count();
    assert_eq!(
        n, 0,
        "without a barracks-providing building, no e3 may spawn (got {n})",
    );
}
