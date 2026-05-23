//! Parsing coverage for the top-level `water_cells:` and `water_rect:`
//! scenario-YAML blocks (naval MVP overlay path).
//!
//! These keys declare WATER cells on top of an otherwise-grass map,
//! sidestepping the `map.bin` tile-encoding entirely. The engine
//! applies them in `env.rs::build_world_for_episode` after the world
//! is constructed: each declared cell becomes ground-impassable and
//! ship-passable. End-to-end naval movement / attack behaviour is
//! pinned by `openra-sim/tests/test_naval.rs`; this file pins the
//! YAML-layer parser only.

use openra_data::oramap::load_rush_hour_map_with_spawn;
use std::fs;
use std::path::PathBuf;

const BODY: &str = r#"
base_map: rush-hour-arena.oramap
agent:
  faction: allies
enemy:
  faction: soviet
actors:
- type: e1
  owner: agent
  position:
  - 5
  - 5
"#;

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
        let candidate = PathBuf::from(&home)
            .join("Projects/OpenRA-RL-Training/scenarios/maps/rush-hour-arena.oramap");
        if candidate.exists() {
            fs::copy(&candidate, &dest).unwrap();
        }
    }

    load_rush_hour_map_with_spawn(&scenario_path, 0).expect("scenario should parse")
}

#[test]
fn water_cells_defaults_to_empty() {
    let map = load_scenario(BODY);
    assert!(
        map.water_cells.is_empty(),
        "a scenario that omits water_cells has no water — got {:?}",
        map.water_cells
    );
}

#[test]
fn water_cells_block_form_is_parsed() {
    // PyYAML block form: list of `[x, y]` inline pairs.
    let text = format!(
        "{BODY}water_cells:\n  - [10, 5]\n  - [10, 6]\n  - [11, 5]\n"
    );
    let map = load_scenario(&text);
    assert_eq!(
        map.water_cells,
        vec![(10, 5), (10, 6), (11, 5)],
        "block-form water_cells parsed wrong"
    );
}

#[test]
fn water_cells_inline_form_is_parsed() {
    // Inline `water_cells: [[x, y], [x, y]]`.
    let text = format!("{BODY}water_cells: [[7, 3], [7, 4]]\n");
    let map = load_scenario(&text);
    assert_eq!(map.water_cells, vec![(7, 3), (7, 4)]);
}

#[test]
fn water_rect_expands_to_cell_list() {
    // `water_rect: [x, y, w, h]` expands to w*h cells.
    let text = format!("{BODY}water_rect: [4, 2, 2, 3]\n");
    let map = load_scenario(&text);
    // y-major, x-minor expansion in the parser.
    assert_eq!(
        map.water_cells,
        vec![
            (4, 2),
            (5, 2),
            (4, 3),
            (5, 3),
            (4, 4),
            (5, 4),
        ]
    );
}

#[test]
fn water_rect_block_form_is_parsed() {
    // PyYAML safe_dump emits the rect as a block scalar list:
    //   water_rect:
    //     - 15
    //     - 2
    //     - 2
    //     - 36
    // This is the form `_scenario_to_tmp_yaml` actually writes, so
    // the engine MUST accept it. Pins the bench-side smoke path.
    let text = format!(
        "{BODY}water_rect:\n  - 15\n  - 2\n  - 2\n  - 36\n"
    );
    let map = load_scenario(&text);
    // 2 wide × 36 tall = 72 cells.
    assert_eq!(map.water_cells.len(), 72);
    // First cell is the rect's origin.
    assert_eq!(map.water_cells[0], (15, 2));
    // All cells fall inside the rect.
    assert!(
        map.water_cells
            .iter()
            .all(|&(x, y)| (15..=16).contains(&x) && (2..=37).contains(&y))
    );
}

#[test]
fn water_cells_block_block_form_is_parsed() {
    // PyYAML safe_dump for a list-of-lists emits the nested form
    //   water_cells:
    //     - - 10
    //       - 5
    //     - - 11
    //       - 6
    // The engine accepts this too.
    let text = format!(
        "{BODY}water_cells:\n  - - 10\n    - 5\n  - - 11\n    - 6\n"
    );
    let map = load_scenario(&text);
    assert_eq!(map.water_cells, vec![(10, 5), (11, 6)]);
}

#[test]
fn water_rect_and_water_cells_combine() {
    // Both keys present — the rect is expanded and concatenated with
    // any explicit list. Order: in the YAML.
    let text = format!(
        "{BODY}water_rect: [0, 0, 1, 2]\nwater_cells:\n  - [9, 9]\n"
    );
    let map = load_scenario(&text);
    assert_eq!(map.water_cells, vec![(0, 0), (0, 1), (9, 9)]);
}
