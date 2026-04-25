//! Phase-3 shroud basic visibility test.
//!
//! 1 own actor at (10, 10) with `RevealsShroud.Range = 4 cells`.
//! Assert cells (10±4, 10±4) visible, (15, 15) not visible. After
//! moving the actor to (20, 10), old cells go back to fog (unless
//! another own unit is there), new cells become visible.
//!
//! Note: the typed `Shroud` uses Euclidean distance (`dx² + dy² ≤ r²`)
//! so the corners of a 4-cell box are at radius √32 ≈ 5.66 — outside
//! the disc. We probe along the cardinal axes (which are inside the
//! disc) and at the (15, 15) corner (outside both Euclidean and
//! Chebyshev radius-4 visibility).

use openra_data::rules::WDist;
use openra_sim::actor::ActorKind;
use openra_sim::math::CPos;
use openra_sim::traits::{update_from_actors, Shroud};

#[test]
fn shroud_reveals_disc_around_own_actor() {
    let mut shroud = Shroud::new(40, 40);
    let actors = vec![(1, ActorKind::Infantry, CPos::new(10, 10), Some(WDist::from_cells(4)))];
    update_from_actors(&mut shroud, actors, 1);

    // Cardinals are inside the disc (Manhattan = 4 ≤ Euclidean radius 4).
    assert!(shroud.is_visible(10, 10), "centre cell visible");
    assert!(shroud.is_visible(14, 10), "+4 east visible");
    assert!(shroud.is_visible(6, 10), "-4 west visible");
    assert!(shroud.is_visible(10, 14), "+4 south visible");
    assert!(shroud.is_visible(10, 6), "-4 north visible");
    // Outside the disc: (15, 15) is at d² = 50 > 16.
    assert!(!shroud.is_visible(15, 15), "(15,15) is outside the disc, must be invisible");
    // Far corner: clearly outside.
    assert!(!shroud.is_visible(20, 20));
}

#[test]
fn shroud_old_cells_unfog_when_actor_moves_no_other_unit() {
    let mut shroud = Shroud::new(60, 40);
    // Frame 1: actor at (10, 10), reveal disc r=4.
    let frame1 = vec![(1, ActorKind::Infantry, CPos::new(10, 10), Some(WDist::from_cells(4)))];
    update_from_actors(&mut shroud, frame1, 1);
    assert!(shroud.is_visible(10, 10));
    assert!(shroud.is_visible(14, 10));

    // Frame 2: actor moved to (20, 10). Old cells lose active visibility.
    let frame2 = vec![(1, ActorKind::Infantry, CPos::new(20, 10), Some(WDist::from_cells(4)))];
    update_from_actors(&mut shroud, frame2, 1);

    // (10, 10) is no longer actively visible (only one unit, and it's gone).
    assert!(!shroud.is_visible(10, 10), "old cell loses active visibility after move");
    // But it remains explored (sticky terrain reveal).
    assert!(shroud.is_explored(10, 10), "old cell stays explored");

    // New disc is visible.
    assert!(shroud.is_visible(20, 10));
    assert!(shroud.is_visible(24, 10));
    // Outside the new disc.
    assert!(!shroud.is_visible(28, 10));
}

#[test]
fn shroud_old_cells_remain_visible_when_second_unit_covers() {
    let mut shroud = Shroud::new(60, 40);
    // Two units covering overlapping discs.
    let frame1 = vec![
        (1, ActorKind::Infantry, CPos::new(10, 10), Some(WDist::from_cells(4))),
        (1, ActorKind::Infantry, CPos::new(12, 10), Some(WDist::from_cells(4))),
    ];
    update_from_actors(&mut shroud, frame1, 1);
    assert!(shroud.is_visible(10, 10));

    // Unit 1 moves away to (40, 10) but unit 2 stays at (12, 10).
    let frame2 = vec![
        (1, ActorKind::Infantry, CPos::new(40, 10), Some(WDist::from_cells(4))),
        (1, ActorKind::Infantry, CPos::new(12, 10), Some(WDist::from_cells(4))),
    ];
    update_from_actors(&mut shroud, frame2, 1);

    // (10, 10) is still in unit 2's disc → still visible.
    assert!(shroud.is_visible(10, 10), "second unit's disc keeps cell visible");
}
