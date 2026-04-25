//! Phase-3 shroud "explored but not currently visible" semantics.
//!
//! Confirms OpenRA's two-state visibility model: cells once seen
//! stay revealed terrain (gray) but enemy actors only show when
//! actively in sight.
//!
//! Reference: vendor/OpenRA/OpenRA.Mods.Common/Traits/World/Shroud.cs.
//! `Shroud.IsVisible` returns true only while a friendly source is
//! revealing the cell *this tick*; `Shroud.IsExplored` returns true
//! once any source has ever revealed it (sticky). Our typed
//! `Shroud` mirrors both flags; observation builders use
//! `is_visible` for actor visibility and `is_explored` for terrain
//! tile rendering.

use openra_data::rules::WDist;
use openra_sim::actor::ActorKind;
use openra_sim::math::CPos;
use openra_sim::traits::{update_from_actors, Shroud};

#[test]
fn explored_remains_after_actor_leaves() {
    let mut shroud = Shroud::new(40, 40);
    // Tick 1: own scout sees (10, 10).
    let frame1 = vec![(1, ActorKind::Infantry, CPos::new(10, 10), Some(WDist::from_cells(4)))];
    update_from_actors(&mut shroud, frame1, 1);
    assert!(shroud.is_visible(10, 10));
    assert!(shroud.is_explored(10, 10));

    // Tick 2: scout walks away.
    let frame2 = vec![(1, ActorKind::Infantry, CPos::new(35, 35), Some(WDist::from_cells(4)))];
    update_from_actors(&mut shroud, frame2, 1);

    // (10, 10) is no longer actively visible …
    assert!(!shroud.is_visible(10, 10));
    // … but stays explored — terrain still rendered.
    assert!(shroud.is_explored(10, 10));
}

#[test]
fn enemy_actor_only_visible_when_in_active_sight() {
    // Simulate the observation pipeline check that uses IsVisible
    // (not IsExplored) when deciding whether to surface an enemy
    // actor in `enemy_positions`. We build a small two-frame
    // scenario:
    //
    //  - frame 1: own unit at (10,10) reveals around it; an enemy
    //    sits at (12, 10), inside the disc → enemy is "visible".
    //  - frame 2: own unit walks to (35, 35); the enemy didn't
    //    move, but is now in fog → not visible (despite (12,10)
    //    being explored).
    //
    // The test directly probes `is_visible` at the enemy's cell.

    let mut shroud = Shroud::new(60, 60);
    let enemy_cell = CPos::new(12, 10);

    // Frame 1: own scout sees the enemy.
    let frame1 = vec![(1, ActorKind::Infantry, CPos::new(10, 10), Some(WDist::from_cells(4)))];
    update_from_actors(&mut shroud, frame1, 1);
    assert!(shroud.is_visible_at(enemy_cell), "enemy visible while in own sight");
    assert!(shroud.is_explored(enemy_cell.x(), enemy_cell.y()));

    // Frame 2: scout leaves. Enemy is in fog: explored, not visible.
    let frame2 = vec![(1, ActorKind::Infantry, CPos::new(35, 35), Some(WDist::from_cells(4)))];
    update_from_actors(&mut shroud, frame2, 1);
    assert!(!shroud.is_visible_at(enemy_cell), "enemy hides in fog of war");
    assert!(shroud.is_explored(enemy_cell.x(), enemy_cell.y()),
            "terrain at enemy cell remains explored");
}

#[test]
fn visibility_re_acquires_when_unit_returns() {
    let mut shroud = Shroud::new(40, 40);
    let target = CPos::new(8, 8);

    // See it.
    update_from_actors(
        &mut shroud,
        [(1, ActorKind::Infantry, CPos::new(10, 10), Some(WDist::from_cells(4)))],
        1,
    );
    assert!(shroud.is_visible_at(target));

    // Walk away.
    update_from_actors(
        &mut shroud,
        [(1, ActorKind::Infantry, CPos::new(30, 30), Some(WDist::from_cells(4)))],
        1,
    );
    assert!(!shroud.is_visible_at(target));
    assert!(shroud.is_explored(target.x(), target.y()));

    // Walk back.
    update_from_actors(
        &mut shroud,
        [(1, ActorKind::Infantry, CPos::new(10, 10), Some(WDist::from_cells(4)))],
        1,
    );
    assert!(shroud.is_visible_at(target));
}
