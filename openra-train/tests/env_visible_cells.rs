//! Engine pin: per-tick `visible_cells` surfaces the agent's typed
//! shroud `visible` mask via PyO3 (parity with `explored_cells`).
//!
//! The bench previously approximated this by drawing Chebyshev sight
//! discs around live agent actors and the vendor RA `Sight` table —
//! correct for vendor units, ~1-cell-ring inaccuracy for any actor
//! with a non-standard sight range or whose footprint biased the
//! engine's true reveal disc. With this engine surface the bench can
//! read the truth directly.
//!
//! Three properties are pinned:
//!  1. At t=0 (reset), `visible_cells ⊆ explored_cells` AND covers
//!     each agent unit's spawn cell (the unit always sees its own
//!     cell — `reveal_disc` of radius ≥ 0 includes the centre).
//!  2. After a unit moves AWAY from its starting cell, that starting
//!     cell stays in `explored_cells` (sticky) but DROPS OUT of
//!     `visible_cells` (active sight only).
//!  3. With zero live agent actors (no agent placed), `visible_cells`
//!     is empty.

use openra_train::{Command, Env};
use std::io::Write;
use std::path::PathBuf;

fn write_scenario(name: &str, body: &str) -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let dir = PathBuf::from(&home).join("Projects/OpenRA-RL-Training/scenarios/discovery");
    if !dir.join("rush-hour.yaml").exists() {
        return None;
    }
    let p = dir.join(name);
    std::fs::File::create(&p).ok()?.write_all(body.as_bytes()).ok()?;
    Some(p)
}

fn one_unit_scenario() -> String {
    // One agent jeep mid-map, no enemy. `spawn_mcvs: false` so the
    // engine doesn't auto-place an MCV (no extra sight discs to
    // reason about).
    r#"name: VisibleCellsOne
base_map: ../maps/rush-hour-arena.oramap
spawn_mcvs: false
starting_cash: 0
agent:
  faction: allies
enemy:
  faction: soviet
actors:
- type: jeep
  owner: agent
  position:
  - 20
  - 20
termination:
  max_ticks: 20000
  enemy_units_killed: false
"#
    .to_string()
}

fn no_agent_scenario() -> String {
    // ZERO agent actors. `spawn_mcvs: false` so the engine truly has
    // no agent unit on the map. A lone enemy keeps the world alive
    // long enough to read an observation. Without
    // `agent_units_killed: false` the engine would auto-`done` the
    // moment it sees the empty agent side.
    r#"name: VisibleCellsNoAgent
base_map: ../maps/rush-hour-arena.oramap
spawn_mcvs: false
starting_cash: 0
agent:
  faction: allies
enemy:
  faction: soviet
actors:
- type: e1
  owner: enemy
  position:
  - 60
  - 20
termination:
  max_ticks: 20000
  agent_units_killed: false
  enemy_units_killed: false
"#
    .to_string()
}

#[test]
fn visible_cells_at_reset_is_subset_of_explored_and_covers_unit() {
    let Some(path) = write_scenario("_visible_cells_one.yaml", &one_unit_scenario()) else {
        eprintln!("skip: RL-Training scenarios tree not present");
        return;
    };
    let mut env = Env::new(path.to_str().unwrap(), 9).expect("Env::new");
    let o = env.reset();
    assert!(!o.unit_positions.is_empty(), "agent jeep must load");

    let explored: std::collections::HashSet<(i32, i32)> =
        o.explored_cells.iter().copied().collect();
    let visible: std::collections::HashSet<(i32, i32)> =
        o.visible_cells.iter().copied().collect();

    // Property 1a: visible ⊆ explored (a cell is at minimum
    // explored the tick it becomes visible).
    for cell in &visible {
        assert!(
            explored.contains(cell),
            "visible cell {:?} must also be in explored set (visible ⊆ explored)",
            cell
        );
    }

    // Property 1b: visible covers every live agent unit's spawn cell.
    // A unit's own cell is always inside its sight disc (radius ≥ 0).
    for (_id, pos) in &o.unit_positions {
        let cell = (pos.cell_x, pos.cell_y);
        assert!(
            visible.contains(&cell),
            "agent unit's own cell {:?} must be in visible_cells (every actor sees its own cell)",
            cell
        );
    }

    // visible_cells is non-empty when ≥1 agent unit is alive.
    assert!(
        !visible.is_empty(),
        "visible_cells must be non-empty with a live agent actor"
    );
}

#[test]
fn visible_cells_drops_old_position_after_move() {
    let Some(path) = write_scenario("_visible_cells_move.yaml", &one_unit_scenario()) else {
        eprintln!("skip: RL-Training scenarios tree not present");
        return;
    };
    let mut env = Env::new(path.to_str().unwrap(), 11).expect("Env::new");
    let initial = env.reset();
    let (uid, start) = initial
        .unit_positions
        .first()
        .map(|(id, p)| (id.clone(), (p.cell_x, p.cell_y)))
        .expect("≥ 1 agent unit at reset");

    // Confirm the start cell IS visible AND explored on reset.
    let start_visible_t0: std::collections::HashSet<(i32, i32)> =
        initial.visible_cells.iter().copied().collect();
    assert!(
        start_visible_t0.contains(&start),
        "start cell {:?} visible at t=0",
        start
    );

    // Drive the jeep ~30 cells away — well past its sight radius
    // (vendor jeep sight ≈ 5-8 cells). Multiple steps so the unit
    // has time to actually traverse.
    let target_x = if start.0 > 64 { start.0 - 30 } else { start.0 + 30 };
    let target_y = start.1.clamp(3, 36);
    let mut moved_pos = start;
    for _ in 0..40 {
        let r = env.step(&[Command::MoveUnits {
            unit_ids: vec![uid.clone()],
            target_x,
            target_y,
        }]);
        if let Some((_, p)) = r.obs.unit_positions.iter().find(|(id, _)| id == &uid) {
            moved_pos = (p.cell_x, p.cell_y);
        }
        if moved_pos == (target_x, target_y) {
            break;
        }
    }

    assert_ne!(
        moved_pos, start,
        "jeep must have left its spawn cell after 40 steps (start={:?})",
        start
    );
    // The Chebyshev distance from start must exceed any plausible
    // sight radius (jeep ≈ 8 cells in vendor RA).
    let dx = (moved_pos.0 - start.0).abs();
    let dy = (moved_pos.1 - start.1).abs();
    assert!(
        dx.max(dy) > 10,
        "jeep must have moved well outside sight range of its spawn (moved={:?}, start={:?})",
        moved_pos,
        start
    );

    let post = env.last_observation();
    let explored_post: std::collections::HashSet<(i32, i32)> =
        post.explored_cells.iter().copied().collect();
    let visible_post: std::collections::HashSet<(i32, i32)> =
        post.visible_cells.iter().copied().collect();

    // Property 2a: start cell is STILL explored (sticky).
    assert!(
        explored_post.contains(&start),
        "start cell {:?} must stay in explored_cells (sticky)",
        start
    );
    // Property 2b: start cell is NO LONGER actively visible.
    assert!(
        !visible_post.contains(&start),
        "start cell {:?} must NOT be in visible_cells after the unit moved away",
        start
    );
    // Property 2c: visible ⊆ explored still holds after movement.
    for cell in &visible_post {
        assert!(
            explored_post.contains(cell),
            "visible cell {:?} must also be in explored set after move",
            cell
        );
    }
    // Property 2d: the new position IS in visible_cells.
    assert!(
        visible_post.contains(&moved_pos),
        "current unit cell {:?} must be in visible_cells",
        moved_pos
    );
}

#[test]
fn visible_cells_empty_with_no_agent_actors() {
    let Some(path) = write_scenario("_visible_cells_noagent.yaml", &no_agent_scenario()) else {
        eprintln!("skip: RL-Training scenarios tree not present");
        return;
    };
    let mut env = Env::new(path.to_str().unwrap(), 13).expect("Env::new");
    let o = env.reset();

    assert!(
        o.unit_positions.is_empty(),
        "scenario must place zero agent actors (got {} units)",
        o.unit_positions.len()
    );

    // Property 3: no live agent actor ⇒ no cells visible to the
    // agent player this tick. The shroud's per-tick visible mask is
    // cleared at the start of each recompute and only re-set by
    // sight discs around own actors, so an agent-less player sees
    // nothing actively.
    assert!(
        o.visible_cells.is_empty(),
        "visible_cells must be empty with no agent actor, got {} cells",
        o.visible_cells.len()
    );
}
