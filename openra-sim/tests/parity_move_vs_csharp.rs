//! Parity test: spawn one e1 infantry at (10,10), order Move to (15,15),
//! tick 50 game-ticks, and compare the Rust-sim trajectory against a
//! reference trace recorded from the prod C# OpenRA server.
//!
//! The reference is `openra-sim/tests/fixtures/move_trace.json`,
//! produced by `scripts/dump_csharp_move_trace.py` (see the script
//! header for prod-server connection details). When the fixture is
//! missing, this test is skipped with a diagnostic — local CI does not
//! require live server access.

use std::path::PathBuf;

use openra_data::oramap::{OraMap, PlayerDef};
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::math::{CPos, WAngle};
use openra_sim::traits::TraitState;
use openra_sim::world::{self, GameOrder, LobbyInfo};

const FROM: (i32, i32) = (10, 10);
const TO: (i32, i32) = (15, 15);
const TICKS: usize = 50;
/// Tolerance for cell-based parity (pathfinder may differ slightly).
const CELL_DRIFT_TOL: i32 = 2;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/move_trace.json")
}

fn empty_world(w: i32, h: i32) -> world::World {
    let map = OraMap {
        title: "parity".into(),
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
    world::build_world(&map, 0, &LobbyInfo::default(), None, 0)
}

fn spawn_e1(world: &mut world::World, cell: (i32, i32)) -> u32 {
    let id = 1000;
    let cpos = CPos::new(cell.0, cell.1);
    let center = openra_sim::world::center_of_cell(cell.0, cell.1);
    let actor = Actor {
        id,
        kind: ActorKind::Infantry,
        owner_id: None,
        location: Some(cell),
        traits: vec![
            TraitState::BodyOrientation { quantized_facings: 32 },
            TraitState::Mobile {
                facing: WAngle::new(640).angle, // SE — matches a (1,1) step from (10,10)
                from_cell: cpos,
                to_cell: cpos,
                center_position: center,
            },
            TraitState::Health { hp: 50000 },
        ],
        activity: None,
        actor_type: Some("e1".into()),
        kills: 0,
        rank: 0,
    };
    world::insert_test_actor(world, actor);
    id
}

fn cell_of(world: &world::World, id: u32) -> (i32, i32) {
    let snap = world.snapshot();
    let s = snap.actors.iter().find(|a| a.id == id).expect("actor missing");
    (s.x, s.y)
}

fn run_rust_trace(actor_id: u32, world: &mut world::World) -> Vec<(i32, i32)> {
    world::set_test_unpaused(world);

    // Issue the move on frame 1.
    world.process_frame(&[GameOrder {
        order_string: "Move".into(),
        subject_id: Some(actor_id),
        target_string: Some(format!("{},{}", TO.0, TO.1)),
        extra_data: None,
    }]);

    let mut cells = Vec::with_capacity(TICKS + 1);
    cells.push(cell_of(world, actor_id));
    // process_frame ticks 3 game-ticks. Loop until we reach the
    // destination or hit a generous safety bound (5×5 diagonal cells
    // at speed 43 takes ≈120 game-ticks ≈ 40 process_frame calls).
    let max_frames = 200;
    for _ in 0..max_frames {
        world.process_frame(&[]);
        cells.push(cell_of(world, actor_id));
        if cells.last() == Some(&TO) {
            break;
        }
    }
    cells
}

#[test]
fn parity_move_matches_csharp_trace_when_fixture_present() {
    let path = fixture_path();
    if !path.exists() {
        eprintln!(
            "SKIP: fixture missing at {}. Run scripts/dump_csharp_move_trace.py \
             against the prod OpenRA server (see script header).",
            path.display()
        );
        return;
    }

    let raw = std::fs::read_to_string(&path).expect("read fixture");
    // Lightweight JSON parsing: pull `samples[*].cell_x/cell_y` without
    // requiring a serde_json dependency.
    let samples: Vec<(i32, i32)> = parse_cells(&raw).expect("parse fixture");
    assert!(samples.len() >= TICKS, "fixture too short: {}", samples.len());

    let mut world = empty_world(40, 40);
    let actor_id = spawn_e1(&mut world, FROM);
    let rust_cells = run_rust_trace(actor_id, &mut world);

    // Compare cell trajectories tick-by-tick (up to the shorter of the
    // two traces). Allow ≤2 cells of drift to account for pathfinder
    // tie-breaking differences.
    let min_len = rust_cells.len().min(samples.len());
    let mut max_drift = 0i32;
    for i in 0..min_len {
        let dx = (rust_cells[i].0 - samples[i].0).abs();
        let dy = (rust_cells[i].1 - samples[i].1).abs();
        max_drift = max_drift.max(dx.max(dy));
    }
    assert!(
        max_drift <= CELL_DRIFT_TOL,
        "parity drift {} exceeds tolerance {} (cells)",
        max_drift,
        CELL_DRIFT_TOL,
    );

    // Both implementations should arrive at the destination cell.
    assert_eq!(rust_cells.last(), Some(&TO), "rust trace must reach target");
    assert_eq!(samples.last(), Some(&TO), "csharp trace must reach target");
}

/// Self-test: even without a fixture, the Rust trace from (10,10) to
/// (15,15) must monotonically advance toward the target on each
/// process-frame and finish at the destination cell.
#[test]
fn rust_trace_reaches_target() {
    let mut world = empty_world(40, 40);
    let actor_id = spawn_e1(&mut world, FROM);
    let cells = run_rust_trace(actor_id, &mut world);
    assert!(!cells.is_empty());
    assert_eq!(cells.first(), Some(&FROM));
    let last = *cells.last().unwrap();
    assert_eq!(last, TO, "rust trace must reach {:?}, got {:?}", TO, last);
    // Monotonic chebyshev distance to target.
    let mut prev_d = chebyshev(cells[0], TO);
    for &c in &cells[1..] {
        let d = chebyshev(c, TO);
        assert!(d <= prev_d, "non-monotone progress: {} → {} at {:?}", prev_d, d, c);
        prev_d = d;
    }
}

fn chebyshev(a: (i32, i32), b: (i32, i32)) -> i32 {
    (a.0 - b.0).abs().max((a.1 - b.1).abs())
}

/// Minimal JSON cell-extractor — looks for `"cell_x":` / `"cell_y":`
/// pairs. Tolerates whitespace; ignores everything else.
fn parse_cells(raw: &str) -> Option<Vec<(i32, i32)>> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while let Some(idx) = raw[cursor..].find("\"cell_x\"") {
        let start = cursor + idx;
        let after = &raw[start..];
        let cx = scan_int_after(after, "cell_x")?;
        let cy = scan_int_after(after, "cell_y")?;
        out.push((cx, cy));
        cursor = start + 1;
    }
    if out.is_empty() { None } else { Some(out) }
}

fn scan_int_after(s: &str, key: &str) -> Option<i32> {
    let pat = format!("\"{key}\"");
    let pos = s.find(&pat)?;
    let after = &s[pos + pat.len()..];
    let colon = after.find(':')?;
    let tail = after[colon + 1..].trim_start();
    let end = tail
        .find(|c: char| !(c == '-' || c.is_ascii_digit()))
        .unwrap_or(tail.len());
    tail[..end].parse().ok()
}

#[cfg(test)]
mod parsing_tests {
    use super::*;

    #[test]
    fn parse_cells_simple() {
        let raw = r#"{"samples":[{"cell_x": 10, "cell_y": 10}, {"cell_x": 11, "cell_y": 11}]}"#;
        let cells = parse_cells(raw).unwrap();
        assert_eq!(cells, vec![(10, 10), (11, 11)]);
    }

    #[test]
    fn parse_cells_negative() {
        let raw = r#"{"cell_x": -3, "cell_y": -7}"#;
        assert_eq!(parse_cells(raw), Some(vec![(-3, -7)]));
    }

    #[test]
    fn parse_cells_returns_none_when_empty() {
        assert!(parse_cells(r#"{}"#).is_none());
    }
}
