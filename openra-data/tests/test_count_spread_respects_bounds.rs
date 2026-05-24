//! P1 fix from PR #13 review: `next_free_spiral` (the helper that
//! spreads a scenario actor `count: N` across N distinct cells) used
//! to check only the `used` set when picking each candidate. With an
//! anchor near a map edge the spiral could walk OUTSIDE the playable
//! rectangle — the engine panics on out-of-bounds actor placement.
//!
//! The fix threads the base-map's playable `bounds` rectangle into
//! the spiral so candidate cells outside `[bx, bx+bw) × [by, by+bh)`
//! are rejected. This test pins the behaviour by placing 30 e1s
//! anchored at the south-east interior corner of the rush-hour-arena
//! playable rect: every emitted ScenarioActor must land inside the
//! playable bounds, and the per-cell uniqueness invariant must hold.
//!
//! Rush-hour-arena bounds (validated by `rush_hour_map::map_loads`)
//! are `(x=2, y=2, w=124, h=36)`, so the playable rectangle is
//! `x ∈ [2, 126)` and `y ∈ [2, 38)`.

use openra_data::oramap::load_rush_hour_map_with_spawn;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

fn load_scenario(text: &str) -> openra_data::oramap::MapDef {
    let tmpdir = tempfile::tempdir().expect("tempdir");
    let scenario_path: PathBuf = tmpdir.path().join("scen.yaml");
    fs::write(&scenario_path, text).unwrap();

    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("rush-hour-arena.oramap");
    let dest = tmpdir.path().join("rush-hour-arena.oramap");
    if fixture.exists() {
        fs::copy(&fixture, &dest).unwrap();
    } else if let Ok(home) = std::env::var("HOME") {
        for candidate in [
            "Projects/openra-rl/maps/rush-hour-arena.oramap",
            "Projects/OpenRA-RL-Training/scenarios/maps/rush-hour-arena.oramap",
        ] {
            let p = PathBuf::from(&home).join(candidate);
            if p.exists() {
                fs::copy(&p, &dest).unwrap();
                break;
            }
        }
    }

    load_rush_hour_map_with_spawn(&scenario_path, 0)
        .expect("scenario should parse")
}

fn in_bounds(p: (i32, i32), bounds: (i32, i32, i32, i32)) -> bool {
    let (bx, by, bw, bh) = bounds;
    p.0 >= bx && p.0 < bx + bw && p.1 >= by && p.1 < by + bh
}

/// `count: 30` anchored at the south-east interior corner: every
/// spread cell must land inside the playable rectangle. Without the
/// fix the spiral walks outside the rect and the engine subsequently
/// panics; with the fix the spiral picks the nearest in-bounds free
/// cells.
#[test]
fn count_spread_near_se_edge_stays_in_bounds() {
    // Anchor at (124, 36) — one inside each bound on the SE side.
    // Playable rect is x ∈ [2, 126), y ∈ [2, 38). A naive spiral with
    // r = 1..=64 around (124, 36) would routinely propose cells like
    // (126, 38) which are off-map; only the bounds gate keeps them
    // off.
    let scen = r#"
base_map: rush-hour-arena.oramap
agent:
  faction: allies
enemy:
  faction: soviet
actors:
- type: e1
  owner: agent
  position:
  - 124
  - 36
  count: 30
"#;
    let map = load_scenario(scen);

    assert_eq!(map.bounds, (2, 2, 124, 36));
    assert_eq!(map.actors.len(), 30, "all 30 copies must be emitted");

    let mut seen: HashSet<(i32, i32)> = HashSet::new();
    for a in &map.actors {
        assert!(
            in_bounds(a.position, map.bounds),
            "actor at {:?} fell outside playable bounds {:?}",
            a.position,
            map.bounds
        );
        // The non-zero copies must NOT stack. (The 0th copy starts at
        // the anchor; subsequent copies use the spiral, which inserts
        // into `used` so subsequent spirals reject those cells.)
        assert!(
            seen.insert(a.position),
            "actor at {:?} is stacked on a previous actor",
            a.position
        );
    }
}

/// `count: 12` anchored at the NW interior corner (2, 2) — symmetric
/// to the SE test. The spiral must NOT propose any cell with x < 2
/// or y < 2.
#[test]
fn count_spread_near_nw_edge_stays_in_bounds() {
    let scen = r#"
base_map: rush-hour-arena.oramap
agent:
  faction: allies
enemy:
  faction: soviet
actors:
- type: e1
  owner: agent
  position:
  - 2
  - 2
  count: 12
"#;
    let map = load_scenario(scen);

    assert_eq!(map.actors.len(), 12);
    for a in &map.actors {
        assert!(
            in_bounds(a.position, map.bounds),
            "actor at {:?} fell outside playable bounds {:?}",
            a.position,
            map.bounds
        );
    }
}

/// Back-compat: a `count: N` anchored well inside the map, where the
/// nearest free cells are all in-bounds, produces the same result as
/// pre-fix — N distinct cells, none stacked.
#[test]
fn count_spread_interior_anchor_unchanged() {
    let scen = r#"
base_map: rush-hour-arena.oramap
agent:
  faction: allies
enemy:
  faction: soviet
actors:
- type: e1
  owner: agent
  position:
  - 60
  - 20
  count: 9
"#;
    let map = load_scenario(scen);
    assert_eq!(map.actors.len(), 9);
    let mut seen: HashSet<(i32, i32)> = HashSet::new();
    for a in &map.actors {
        assert!(in_bounds(a.position, map.bounds));
        assert!(seen.insert(a.position), "stacked at {:?}", a.position);
    }
    // The first (anchor) copy must be exactly at the requested cell.
    assert_eq!(map.actors[0].position, (60, 20));
}
