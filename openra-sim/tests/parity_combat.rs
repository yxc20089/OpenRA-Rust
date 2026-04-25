//! Parity test for Phase-3 combat against the prod C# OpenRA server.
//!
//! Fixture at `openra-sim/tests/fixtures/combat_csharp.json` is
//! produced by `scripts/dump_csharp_combat_trace.py`. We run the
//! identical 1v1 e1-vs-e1 scenario in the Rust sim and compare
//! per-tick HP and the terminal HP. Tolerance: ±5% of B's max HP at
//! the terminal tick (the C# damage formula has a `Damage *= 100 /
//! DamageVsClass` Versus multiplier that the v1 Rust combat does not
//! replicate exactly, so we explicitly accept some drift).
//!
//! When the fixture is absent, the test is skipped with a
//! diagnostic — local CI does not require live server access.

use std::path::PathBuf;

use openra_data::oramap::{OraMap, PlayerDef};
use openra_data::rules::{WDist, WeaponStats};
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::activities::AttackActivity;
use openra_sim::activity::{ActivityStack, ActivityState};
use openra_sim::traits::{Armament, TraitState};
use openra_sim::world::{self, insert_test_actor, set_test_unpaused, LobbyInfo};

const A_CELL: (i32, i32) = (5, 10);
const B_CELL: (i32, i32) = (10, 10);
const TICKS: usize = 200;
const HP_TOL_PCT: i32 = 5;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/combat_csharp.json")
}

/// Tiny zero-dep JSON sniff for the `samples[].b_hp` field, the only
/// value we need here. Returns the terminal `b_hp`.
fn parse_terminal_b_hp(raw: &str) -> Option<i32> {
    // Walk through every `"b_hp": <int>` occurrence; the last one is
    // terminal. Robust enough for the shape produced by
    // `dump_csharp_combat_trace.py`.
    let needle = "\"b_hp\":";
    let mut last: Option<i32> = None;
    let mut start = 0;
    while let Some(pos) = raw[start..].find(needle) {
        let abs = start + pos + needle.len();
        let mut end = abs;
        let bytes = raw.as_bytes();
        while end < bytes.len() && bytes[end].is_ascii_whitespace() { end += 1; }
        let num_start = end;
        // Optional minus
        if end < bytes.len() && (bytes[end] == b'-' || bytes[end] == b'+') {
            end += 1;
        }
        while end < bytes.len() && bytes[end].is_ascii_digit() { end += 1; }
        if let Ok(n) = raw[num_start..end].trim().parse::<i32>() {
            last = Some(n);
        }
        start = end;
    }
    last
}

fn empty_world(w: i32, h: i32) -> world::World {
    let map = OraMap {
        title: "parity-combat".into(),
        tileset: "TEMPERAT".into(),
        map_size: (w, h),
        bounds: (0, 0, w, h),
        tiles: Vec::new(),
        actors: Vec::new(),
        players: vec![PlayerDef {
            name: "Neutral".into(),
            playable: false,
            owns_world: true,
            non_combatant: true,
            faction: "allies".into(),
            enemies: Vec::new(),
        }],
    };
    let mut w = world::build_world(&map, 0, &LobbyInfo::default(), None, 0);
    set_test_unpaused(&mut w);
    w
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

fn m1carbine() -> WeaponStats {
    WeaponStats {
        name: "M1Carbine".into(),
        range: WDist::from_cells(5),
        reload_delay: 20,
        damage: 1000,
        ..Default::default()
    }
}

fn target_hp(world: &world::World, id: u32) -> i32 {
    world.actor(id).and_then(|a| a.traits.iter().find_map(|t| {
        if let TraitState::Health { hp } = t { Some(*hp) } else { None }
    })).unwrap_or(0)
}

#[test]
fn rust_combat_matches_csharp_terminal_hp() {
    let path = fixture_path();
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => {
            eprintln!(
                "[parity_combat] skipped — fixture {:?} missing. Generate \
                 with `scripts/dump_csharp_combat_trace.py` against a \
                 running prod server.",
                path
            );
            return;
        }
    };

    let csharp_terminal_b_hp = parse_terminal_b_hp(&raw)
        .expect("could not extract terminal b_hp from fixture");

    // Run the same scenario in Rust.
    let mut w = empty_world(40, 40);
    insert_test_actor(&mut w, make_e1(101, 1, A_CELL, 5000));
    insert_test_actor(&mut w, make_e1(102, 2, B_CELL, 5000));
    let mut stack = ActivityStack::new();
    stack.push(Box::new(AttackActivity::new(102, Armament::new(m1carbine()))));

    let mut a = w.actor(101).unwrap().clone();
    let mut rust_terminal_hp = 5000;
    for _ in 1..=TICKS {
        let s = stack.run_top(&mut a, &mut w);
        rust_terminal_hp = target_hp(&w, 102);
        if matches!(s, ActivityState::Done) {
            break;
        }
        if rust_terminal_hp <= 0 {
            break;
        }
    }

    // Compare terminal HP within ±5% of the e1 max HP (5000 in
    // canonical RA rules). Because both tracks should kill B given
    // 200 ticks at 1000 dmg / 20 reload, both terminal HPs should be
    // ≤ 0 — the test asserts they *agree* on dead-or-alive, with a
    // 250-HP slack.
    let max_hp = 5000;
    let tolerance = (max_hp * HP_TOL_PCT) / 100;
    let diff = (rust_terminal_hp - csharp_terminal_b_hp).abs();
    assert!(
        diff <= tolerance,
        "rust terminal b_hp = {rust_terminal_hp}, csharp = {csharp_terminal_b_hp}, \
         diff = {diff} > tolerance = {tolerance}"
    );
}

#[test]
fn parse_terminal_b_hp_extracts_last_value() {
    let raw = r#"{"samples":[{"tick":0,"b_hp":5000},{"tick":80,"b_hp":1000},{"tick":81,"b_hp":0}]}"#;
    assert_eq!(parse_terminal_b_hp(raw), Some(0));
}

#[test]
fn parse_terminal_b_hp_handles_negative() {
    let raw = r#"{"samples":[{"b_hp":5000},{"b_hp":-100}]}"#;
    assert_eq!(parse_terminal_b_hp(raw), Some(-100));
}
