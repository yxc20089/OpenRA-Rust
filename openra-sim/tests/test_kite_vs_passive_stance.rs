//! Engine regression test for Fix #5 from OpenRA-Bench's
//! ENGINE_FOLLOWUPS Wave-12/13: a stationary stance:1 (ReturnFire)
//! light tank must NOT win 1v1 against an advancing stance:3
//! (AttackAnything) heavy tank — otherwise the kite-and-pull
//! capability collapses (the policy "just hold position" trivially
//! wins, making the "kite to maintain range" verb redundant). The
//! fundamental balance fix is to scale stance:1 auto-fire DPS by 0.6
//! so a kiter that just stands and trades fire LOSES to a slower
//! heavy that gets into its own optimal range.

use openra_data::oramap::{MapActor, OraMap, PlayerDef};
use openra_data::rules as data_rules;
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::gamerules::GameRules;
use openra_sim::math::{CPos, WAngle, WPos};
use openra_sim::traits::TraitState;
use openra_sim::world::{
    self, insert_test_actor, set_actor_stance, set_test_unpaused, LobbyInfo, SlotInfo, World,
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
        title: "kite-test".into(),
        tileset: "TEMPERAT".into(),
        map_size: (64, 40),
        bounds: (0, 0, 64, 40),
        tiles: Vec::new(),
        actors: vec![
            MapActor { id: "mpspawn1".into(), actor_type: "mpspawn".into(),
                       owner: "Neutral".into(), location: (1, 1) },
            MapActor { id: "mpspawn2".into(), actor_type: "mpspawn".into(),
                       owner: "Neutral".into(), location: (62, 38) },
        ],
        players: vec![
            PlayerDef { name: "Neutral".into(), playable: false, owns_world: true,
                        non_combatant: true, faction: "allies".into(), enemies: Vec::new() },
            PlayerDef { name: "P1".into(), playable: true, owns_world: false,
                        non_combatant: false, faction: "allies".into(),
                        enemies: vec!["P2".into()] },
            PlayerDef { name: "P2".into(), playable: true, owns_world: false,
                        non_combatant: false, faction: "soviet".into(),
                        enemies: vec!["P1".into()] },
        ],
    };
    let lobby = LobbyInfo {
        starting_cash: 0, allow_spectators: false,
        occupied_slots: vec![
            SlotInfo { player_reference: "P1".into(), faction: "allies".into(),
                       is_bot: false, starting_cash: None },
            SlotInfo { player_reference: "P2".into(), faction: "soviet".into(),
                       is_bot: false, starting_cash: None },
        ],
    };
    let mut w = world::build_world(&map, seed, &lobby, Some(rules), 0, false);
    set_test_unpaused(&mut w);
    let strip: Vec<u32> = world::all_actor_ids(&w)
        .into_iter()
        .filter(|&id| matches!(
            w.actor_kind(id),
            Some(ActorKind::Mcv) | Some(ActorKind::Spawn)
        ))
        .collect();
    for id in strip {
        world::remove_test_actor(&mut w, id);
    }
    Some(w)
}

fn make_tank(id: u32, owner: u32, actor_type: &str, at: (i32, i32), hp: i32) -> Actor {
    let cell = CPos::new(at.0, at.1);
    let center = WPos::new(at.0 * 1024 + 512, at.1 * 1024 + 512, 0);
    Actor {
        id, kind: ActorKind::Vehicle, owner_id: Some(owner), location: Some(at),
        traits: vec![
            TraitState::BodyOrientation { quantized_facings: 32 },
            TraitState::Mobile {
                facing: WAngle::new(512).angle,
                from_cell: cell, to_cell: cell, center_position: center,
            },
            TraitState::Health { hp },
        ],
        activity: None,
        actor_type: Some(actor_type.into()),
        kills: 0, rank: 0,
    }
}

fn alive(w: &World, id: u32) -> bool {
    if let Some(a) = w.actor(id) {
        a.traits.iter().any(|t| matches!(t, TraitState::Health { hp } if *hp > 0))
    } else {
        false
    }
}

fn hp_of(w: &World, id: u32) -> i32 {
    w.actor(id)
        .and_then(|a| a.traits.iter().find_map(|t| {
            if let TraitState::Health { hp } = t { Some(*hp) } else { None }
        }))
        .unwrap_or(-1)
}

#[test]
fn stationary_stance1_medium_loses_to_advancing_low_hp_stance3_heavy() {
    // 1v1 baseline mirroring the bench's combat-kite-and-pull EASY
    // tier: a 2tnk raider on stance:1 (ReturnFire, stays still) vs a
    // 3tnk chaser on stance:3 (AttackAnything, advances) at REDUCED
    // HP (~35% of full, i.e. 21000 not 60000). The bench docs this
    // tier as "every wrong policy LOSES and the kite WINS". Before
    // the Fix #5 patch, a stationary stance:1 kiter still WINS because
    // its auto-fire deals full DPS once licensed (the heavy fires
    // first into kiter's recently_received_fire window). After the
    // patch, stance:1 auto-fire DPS is scaled by 0.6 so the heavy
    // wins the trade — restoring the bench's kite capability gate.
    let mut w = match build_arena(53) {
        Some(w) => w,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };
    let pids = w.player_ids().to_vec();
    let p_raider = pids[1];
    let p_heavy = pids[2];

    let raider_id = 1001u32;
    let heavy_id = 2001u32;
    // Standard medium tank (full HP).
    insert_test_actor(&mut w, make_tank(raider_id, p_raider, "2tnk", (20, 20), 46000));
    // Heavy at ~35% HP (bench-tier "easy").
    insert_test_actor(&mut w, make_tank(heavy_id, p_heavy, "3tnk", (40, 20), 21000));
    set_actor_stance(&mut w, raider_id, 1);
    set_actor_stance(&mut w, heavy_id, 3);

    let mut tick_at_end = 0u32;
    let mut winner: &'static str = "timeout";
    for tick in 0..1500 {
        let _ = w.tick(&[]);
        tick_at_end = tick;
        if tick % 50 == 0 {
            eprintln!(
                "tick={tick} raider={} heavy={} heavy_loc={:?}",
                hp_of(&w, raider_id), hp_of(&w, heavy_id),
                w.actor(heavy_id).and_then(|a| a.location)
            );
        }
        let r = alive(&w, raider_id);
        let h = alive(&w, heavy_id);
        if !h && !r { winner = "mutual"; break; }
        if !h { winner = "raider"; break; }
        if !r { winner = "heavy"; break; }
    }
    eprintln!(
        "FINAL tick={tick_at_end} winner={winner} raider_hp={} heavy_hp={}",
        hp_of(&w, raider_id), hp_of(&w, heavy_id)
    );
    assert_eq!(
        winner, "heavy",
        "in the bench combat-kite-and-pull EASY tier (1 medium raider \
         on stance:1 vs 1 low-HP heavy on stance:3), a STATIONARY raider \
         must LOSE — otherwise stall and intended-kite are \
         indistinguishable and the kite capability is untestable. \
         Got winner={winner} raider_hp={} heavy_hp={}",
        hp_of(&w, raider_id), hp_of(&w, heavy_id)
    );
}

#[test]
fn three_stationary_stance1_raiders_lose_to_one_advancing_stance3_heavy() {
    // Multi-unit analogue at full-HP heavy: 3 stance:1 raiders sitting
    // still vs 1 stance:3 60k-HP heavy. After Fix #5 the heavy still
    // wins (it already won this matchup before the patch — this case
    // is a regression guard, not the patch's primary signal).
    let mut w = match build_arena(53) {
        Some(w) => w,
        None => {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
    };
    let pids = w.player_ids().to_vec();
    let p_raiders = pids[1];
    let p_heavy = pids[2];

    let raider_ids: [u32; 3] = [1001, 1002, 1003];
    let heavy_id = 2001u32;
    // Three medium raiders staggered north of the kill lane (so they
    // don't block each other's line of fire and so the heavy needs to
    // close in).
    insert_test_actor(&mut w, make_tank(raider_ids[0], p_raiders, "2tnk", (20, 18), 46000));
    insert_test_actor(&mut w, make_tank(raider_ids[1], p_raiders, "2tnk", (20, 20), 46000));
    insert_test_actor(&mut w, make_tank(raider_ids[2], p_raiders, "2tnk", (20, 22), 46000));
    // One heavy on stance:3 advancing west into them.
    insert_test_actor(&mut w, make_tank(heavy_id, p_heavy, "3tnk", (40, 20), 60000));
    for &rid in &raider_ids {
        set_actor_stance(&mut w, rid, 1);
    }
    set_actor_stance(&mut w, heavy_id, 3);

    let mut tick_at_end = 0u32;
    for tick in 0..1500 {
        let _ = w.tick(&[]);
        tick_at_end = tick;
        if tick % 50 == 0 {
            eprintln!(
                "tick={tick} r0={} r1={} r2={} | heavy={} loc={:?}",
                hp_of(&w, raider_ids[0]),
                hp_of(&w, raider_ids[1]),
                hp_of(&w, raider_ids[2]),
                hp_of(&w, heavy_id),
                w.actor(heavy_id).and_then(|a| a.location)
            );
        }
        let heavy_alive = alive(&w, heavy_id);
        let any_raider_alive = raider_ids.iter().any(|&id| alive(&w, id));
        // Win condition for the heavy: heavy still alive AND at
        // least two raiders dead. The bench requires
        // own_units_gte:3 to win, so losing even one raider is a
        // LOSS for the raiders → the heavy "won the engagement".
        let raiders_alive = raider_ids.iter().filter(|&&id| alive(&w, id)).count();
        if !heavy_alive || (heavy_alive && raiders_alive <= 1) {
            // Settled (either heavy died OR raiders collapsed).
            break;
        }
        if !any_raider_alive {
            break;
        }
    }
    let heavy_alive = alive(&w, heavy_id);
    let raiders_alive = raider_ids.iter().filter(|&&id| alive(&w, id)).count();
    eprintln!(
        "FINAL tick={tick_at_end} heavy_alive={heavy_alive} raiders_alive={raiders_alive} heavy_hp={}",
        hp_of(&w, heavy_id)
    );
    // The capability invariant: a stationary stance:1 raider group
    // must NOT all-survive the trade. The bench's win condition is
    // own_units_gte:3 (all three raiders survive); stall must violate
    // this. Equivalently: heavy still alive OR at least one raider
    // dead by the end of the trade.
    assert!(
        heavy_alive || raiders_alive < 3,
        "stationary stance:1 raiders must LOSE the head-on trade \
         (the kite-and-pull pack's stall policy must fail). \
         Got heavy_alive={heavy_alive} raiders_alive={raiders_alive}"
    );
    // Stronger: the heavy itself should still be alive (it should win
    // the engagement outright, not just take a raider down on its way).
    assert!(
        heavy_alive,
        "stationary stance:1 raiders must NOT kill the advancing \
         stance:3 heavy by passive auto-fire — otherwise the kite \
         capability is redundant. heavy_hp={}",
        hp_of(&w, heavy_id)
    );
}
