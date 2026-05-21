//! Armor-class damage multipliers — anti-armor weapons must punish
//! vehicles, small-arms must not.
//!
//! OpenRA's combat model scales every hit by a per-armor-class
//! `Versus` multiplier *and* lets an actor carry several armaments,
//! picking the one best suited to the target. The Rust engine had the
//! `Versus` multiplier wired but always fired `weapons[0]`, so e3
//! (PRIMARY = RedEye anti-air, SECONDARY = Dragon anti-ground) shot
//! its anti-air missile at tanks and never used its anti-armor Dragon.
//!
//! This test pins:
//!  1. `best_weapon_against` picks Dragon vs Heavy armor and RedEye
//!     vs None armor for e3.
//!  2. End-to-end: 4× e3 vs 4× 2tnk — the e3 squad kills >= 2 tanks
//!     (anti-armor now bites).
//!  3. End-to-end: 4× e1 (rifle) vs a 2tnk — the rifle squad barely
//!     scratches the tank (small-arms-vs-armor is weak), confirming
//!     2tnk beats e1.
//!  4. The RPS triangle holds: e3 > 2tnk, 2tnk > e1, e1 ~ e3 trade.

use openra_data::oramap::{MapActor, OraMap, PlayerDef};
use openra_data::rules as data_rules;
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::gamerules::{ArmorType, GameRules};
use openra_sim::math::{CPos, WAngle, WPos};
use openra_sim::traits::TraitState;
use openra_sim::world::{
    self, insert_test_actor, set_actor_stance, set_test_unpaused, GameOrder, LobbyInfo, SlotInfo,
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
        title: "armor-class".into(),
        tileset: "TEMPERAT".into(),
        map_size: (50, 50),
        bounds: (0, 0, 50, 50),
        tiles: Vec::new(),
        actors: vec![
            MapActor { id: "mpspawn1".into(), actor_type: "mpspawn".into(), owner: "Neutral".into(), location: (1, 1) },
            MapActor { id: "mpspawn2".into(), actor_type: "mpspawn".into(), owner: "Neutral".into(), location: (48, 48) },
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
    let mut world = world::build_world(&map, seed, &lobby, Some(rules), 0, false);
    set_test_unpaused(&mut world);
    Some(world)
}

fn make_infantry(id: u32, owner: u32, ty: &str, at: (i32, i32), hp: i32) -> Actor {
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
        actor_type: Some(ty.into()),
        kills: 0,
        rank: 0,
    }
}

fn make_tank(id: u32, owner: u32, ty: &str, at: (i32, i32), hp: i32) -> Actor {
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
                facing: WAngle::new(512).angle,
                from_cell: cell,
                to_cell: cell,
                center_position: center,
            },
            TraitState::Health { hp },
        ],
        activity: None,
        actor_type: Some(ty.into()),
        kills: 0,
        rank: 0,
    }
}

fn alive(world: &World, id: u32) -> bool {
    world
        .actor(id)
        .and_then(|a| a.traits.iter().find_map(|t| {
            if let TraitState::Health { hp } = t { Some(*hp) } else { None }
        }))
        .map(|hp| hp > 0)
        .unwrap_or(false)
}

fn count_alive(world: &World, ids: &[u32]) -> usize {
    ids.iter().filter(|&&id| alive(world, id)).count()
}

fn strip_spawns(world: &mut World) {
    let strip: Vec<u32> = world::all_actor_ids(world)
        .into_iter()
        .filter(|&id| matches!(world.actor_kind(id), Some(ActorKind::Mcv) | Some(ActorKind::Spawn)))
        .collect();
    for id in strip {
        world::remove_test_actor(world, id);
    }
}

/// Run a squad fight where `squad` focus-fires `foes`: every living
/// squad member is ordered onto the first living foe, and the order is
/// re-issued whenever that foe dies. This is the realistic combined-
/// arms micro the capability test exercises (concentrate fire on one
/// target rather than spreading 1:1). Returns
/// `(squad_alive, foes_alive)` at the end.
fn run_focus_fire(
    world: &mut World,
    squad: &[u32],
    foes: &[u32],
    max_ticks: u32,
) -> (usize, usize) {
    let mut current_target: Option<u32> = None;
    for _ in 1..=max_ticks {
        // Pick the lowest-id living foe as the focus target.
        let live_foe = foes.iter().copied().find(|&f| alive(world, f));
        let mut orders: Vec<GameOrder> = Vec::new();
        if let Some(tid) = live_foe {
            if current_target != Some(tid) {
                current_target = Some(tid);
                for &s in squad {
                    if alive(world, s) {
                        orders.push(GameOrder {
                            order_string: "Attack".into(),
                            subject_id: Some(s),
                            target_string: None,
                            extra_data: Some(tid),
                        });
                    }
                }
            }
        }
        let _ = world.tick(&orders);
        if count_alive(world, foes) == 0 || count_alive(world, squad) == 0 {
            break;
        }
    }
    (count_alive(world, squad), count_alive(world, foes))
}

/// Unit-level: the weapon-selection helper picks the anti-armor weapon
/// against a heavy vehicle and the anti-air weapon against unarmored
/// infantry. This is the core armor-class-driven choice.
#[test]
fn e3_picks_dragon_vs_heavy_redeye_vs_none() {
    let Some(mod_dir) = vendor_mod_dir() else {
        eprintln!("skipping: vendored OpenRA mod dir not found");
        return;
    };
    let ruleset = data_rules::load_ruleset(&mod_dir).unwrap();
    let rules = GameRules::from_ruleset(&ruleset);

    // e3 carries RedEye (anti-air) + Dragon (anti-ground).
    let (vs_heavy, _) = rules
        .best_weapon_against("e3", ArmorType::Heavy)
        .expect("e3 has a weapon");
    let (vs_none, _) = rules
        .best_weapon_against("e3", ArmorType::None)
        .expect("e3 has a weapon");
    eprintln!("e3 vs Heavy -> {vs_heavy}, vs None -> {vs_none}");
    assert_eq!(
        vs_heavy, "Dragon",
        "e3 must use its anti-armor Dragon against a heavy tank"
    );
    assert_eq!(
        vs_none, "RedEye",
        "e3 should keep its higher-base-damage missile against unarmored infantry"
    );

    // Effective damage: Dragon out-damages RedEye on Heavy armor.
    let dragon = rules.weapon("Dragon").expect("Dragon parsed");
    let redeye = rules.weapon("RedEye").expect("RedEye parsed");
    let dragon_vs_heavy = GameRules::effective_damage(dragon, ArmorType::Heavy);
    let redeye_vs_heavy = GameRules::effective_damage(redeye, ArmorType::Heavy);
    eprintln!("Dragon vs Heavy={dragon_vs_heavy}, RedEye vs Heavy={redeye_vs_heavy}");
    assert!(
        dragon_vs_heavy > redeye_vs_heavy,
        "Dragon must out-damage RedEye against heavy armor"
    );

    // Tank cannon vs unarmored infantry — diagnostic.
    let w90 = rules.weapon("90mm").expect("90mm parsed");
    eprintln!("90mm versus table = {:?}", w90.versus);
    eprintln!(
        "90mm vs None={}, vs Heavy={}",
        GameRules::effective_damage(w90, ArmorType::None),
        GameRules::effective_damage(w90, ArmorType::Heavy)
    );

    // Rifle (M1Carbine) is feeble against heavy armor.
    let m1 = rules.weapon("M1Carbine").expect("M1Carbine parsed");
    let rifle_vs_heavy = GameRules::effective_damage(m1, ArmorType::Heavy);
    let rifle_vs_none = GameRules::effective_damage(m1, ArmorType::None);
    eprintln!("M1Carbine vs Heavy={rifle_vs_heavy}, vs None={rifle_vs_none}");
    assert!(
        rifle_vs_heavy * 4 < rifle_vs_none,
        "rifle must be far weaker vs heavy armor than vs unarmored infantry"
    );
    assert!(
        rifle_vs_heavy * 4 < dragon_vs_heavy,
        "rifle vs heavy armor must be much weaker than an anti-armor rocket"
    );
}


/// End-to-end: 4× e3 vs 4× 2tnk. With armor-class weapon selection the
/// e3 squad fires Dragon (anti-armor) and kills tanks. Before the fix
/// they fired RedEye and the diagnosis observed 0 tank kills.
#[test]
fn four_e3_kill_at_least_two_tanks() {
    let Some(mut world) = build_arena(7) else {
        eprintln!("skipping: vendored OpenRA mod dir not found");
        return;
    };
    strip_spawns(&mut world);
    let player_ids = world.player_ids().to_vec();
    let agent = player_ids[1];
    let enemy = player_ids[2];

    // 4 e3 rocket soldiers (agent) vs 4 2tnk heavy tanks (enemy),
    // interleaved at close range so both sides engage immediately.
    let e3_ids = [101u32, 102, 103, 104];
    let tnk_ids = [201u32, 202, 203, 204];
    for (i, &id) in e3_ids.iter().enumerate() {
        insert_test_actor(&mut world, make_infantry(id, agent, "e3", (10, 18 + i as i32), 45_000));
    }
    for (i, &id) in tnk_ids.iter().enumerate() {
        insert_test_actor(&mut world, make_tank(id, enemy, "2tnk", (13, 18 + i as i32), 260_000));
    }

    // The intended capability: the rocket squad concentrates fire.
    let (e3_left, tanks_left) = run_focus_fire(&mut world, &e3_ids, &tnk_ids, 3000);
    eprintln!("4xe3 vs 4x2tnk -> tanks alive={tanks_left}, e3 alive={e3_left}");
    assert!(
        4 - tanks_left >= 2,
        "anti-armor e3 squad must kill >= 2 tanks, killed {}",
        4 - tanks_left
    );
}

/// End-to-end: 4× e1 (rifle) vs a single 2tnk. The rifle is feeble vs
/// heavy armor — the squad must NOT destroy the tank in a long window.
/// This pins "2tnk beats e1".
#[test]
fn four_e1_barely_dent_a_tank() {
    let Some(mut world) = build_arena(7) else {
        eprintln!("skipping: vendored OpenRA mod dir not found");
        return;
    };
    strip_spawns(&mut world);
    let player_ids = world.player_ids().to_vec();
    let agent = player_ids[1];
    let enemy = player_ids[2];

    let e1_ids = [101u32, 102, 103, 104];
    for (i, &id) in e1_ids.iter().enumerate() {
        insert_test_actor(&mut world, make_infantry(id, agent, "e1", (10, 18 + i as i32), 50_000));
    }
    // Lone tank, full HP. Make it passive (stance 0) so we measure pure
    // rifle-vs-armor damage without the tank fighting back skewing the
    // window.
    insert_test_actor(&mut world, make_tank(301, enemy, "2tnk", (13, 19), 260_000));
    set_actor_stance(&mut world, 301, 0);

    let mut orders: Vec<GameOrder> = Vec::new();
    for &id in &e1_ids {
        orders.push(GameOrder {
            order_string: "Attack".into(),
            subject_id: Some(id),
            target_string: None,
            extra_data: Some(301),
        });
    }
    for tick in 1..=900 {
        let no: Vec<GameOrder> = Vec::new();
        let _ = world.tick(if tick == 1 { &orders } else { &no });
        if !alive(&world, 301) {
            break;
        }
    }
    let tank_hp = world
        .actor(301)
        .and_then(|a| a.traits.iter().find_map(|t| {
            if let TraitState::Health { hp } = t { Some(*hp) } else { None }
        }));
    eprintln!("4xe1 vs 1x2tnk -> tank hp={tank_hp:?} (start 260000)");
    assert!(
        alive(&world, 301),
        "4 rifles must NOT destroy a heavy tank — rifle vs armor is feeble"
    );
}

/// The RPS triangle, expressed at COST PARITY (the way OpenRA balance
/// actually works — e3 costs 300, 2tnk 800, e1 100). Anti-armor e3 is
/// the cost-efficient counter to heavy armor; the rifle e1 is the
/// cost-efficient counter to soft infantry but feeble against tanks.
///
///  * e3 (cost-fair count) vs 2tnk → e3 wins  (anti-armor counter)
///  * 2tnk vs e1 (cost-fair count)  → 2tnk wins (armor shrugs rifles)
///  * e1 vs e3 at equal count       → a real trade (e3 is armor None,
///    the rifle genuinely hurts it — not a 0-kill stalemate)
///
/// Cost-fairness keeps the test about the armor-class damage model
/// rather than a raw HP-pool count, while still being a real fight.
#[test]
fn rps_triangle_e3_beats_tank_tank_beats_e1() {
    let Some(_) = vendor_mod_dir() else {
        eprintln!("skipping: vendored OpenRA mod dir not found");
        return;
    };

    // --- e3 vs 2tnk at ~cost parity: 9×e3 (2700) vs 3×2tnk (2400).
    //     The anti-armor squad must come out ahead. ---
    {
        let mut world = build_arena(11).unwrap();
        strip_spawns(&mut world);
        let pid = world.player_ids().to_vec();
        let (agent, enemy) = (pid[1], pid[2]);
        let e3: Vec<u32> = (101..110).collect();
        let tnk = [201u32, 202, 203];
        for (i, &id) in e3.iter().enumerate() {
            let (col, row) = (8 + (i as i32) % 3, 16 + (i as i32) / 3);
            insert_test_actor(&mut world, make_infantry(id, agent, "e3", (col, row), 45_000));
        }
        for (i, &id) in tnk.iter().enumerate() {
            insert_test_actor(&mut world, make_tank(id, enemy, "2tnk", (13, 17 + i as i32), 260_000));
        }
        let (e3_left, tnk_left) = run_focus_fire(&mut world, &e3, &tnk, 4000);
        eprintln!("RPS e3 vs 2tnk (cost-fair) -> e3={e3_left} tnk={tnk_left}");
        assert_eq!(tnk_left, 0, "anti-armor e3 squad must destroy every tank");
        assert!(e3_left > 0, "the e3 counter must survive the engagement");
    }

    // --- 2tnk vs e1 at ~cost parity: 8×e1 (800) vs 1×2tnk (800).
    //     Rifles cannot break heavy armor — the tank wins. ---
    {
        let mut world = build_arena(11).unwrap();
        strip_spawns(&mut world);
        let pid = world.player_ids().to_vec();
        let (agent, enemy) = (pid[1], pid[2]);
        let e1: Vec<u32> = (101..109).collect();
        let tnk = [301u32];
        for (i, &id) in e1.iter().enumerate() {
            let (col, row) = (8 + (i as i32) % 4, 16 + (i as i32) / 4);
            insert_test_actor(&mut world, make_infantry(id, agent, "e1", (col, row), 50_000));
        }
        insert_test_actor(&mut world, make_tank(301, enemy, "2tnk", (13, 17), 260_000));
        // A realistic engagement window (~13 decision turns at ~90
        // ticks/turn). The rifle's 10%-vs-heavy multiplier means a
        // cost-equal rifle squad cannot break a tank inside a real
        // fight — the tank shrugs them off and wins.
        let (e1_left, tnk_left) = run_focus_fire(&mut world, &e1, &tnk, 1200);
        eprintln!("RPS 2tnk vs e1 (cost-fair) -> e1={e1_left} tnk={tnk_left}");
        assert_eq!(tnk_left, 1, "a cost-equal rifle squad must NOT kill a 2tnk in a real fight");
        assert!(e1_left < 8, "the tank's cannon must inflict real rifle losses");
    }

    // --- e1 vs e3 at equal count: a real fight, real losses. ---
    {
        let mut world = build_arena(11).unwrap();
        strip_spawns(&mut world);
        let pid = world.player_ids().to_vec();
        let (agent, enemy) = (pid[1], pid[2]);
        let e1 = [101u32, 102, 103, 104];
        let e3 = [201u32, 202, 203, 204];
        for (i, &id) in e1.iter().enumerate() {
            insert_test_actor(&mut world, make_infantry(id, agent, "e1", (10, 18 + i as i32), 50_000));
        }
        for (i, &id) in e3.iter().enumerate() {
            insert_test_actor(&mut world, make_infantry(id, enemy, "e3", (13, 18 + i as i32), 45_000));
        }
        let (e1_left, e3_left) = run_focus_fire(&mut world, &e1, &e3, 2000);
        eprintln!("RPS e1 vs e3 -> e1={e1_left} e3={e3_left}");
        // e3 is unarmored infantry (armor None) — the rifle genuinely
        // hurts it, so this is a real trade, not a 0-kill stalemate.
        assert!(
            4 - e3_left >= 1,
            "e1 rifles must land real kills on e3 infantry (armor None): e3 killed {}",
            4 - e3_left
        );
    }
}
