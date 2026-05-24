//! Engine regression test for Fix #3 from OpenRA-Bench's
//! ENGINE_FOLLOWUPS Wave-12/13: nuclear AoE kills must be credited to
//! the firing player's `kills_per_player` tally. Before the fix
//! `detonate_nuke` removed dead actors directly without bumping the
//! counter, so bench packs scoring on `units_killed_gte` saw zero
//! credit for nuked enemies.

use openra_data::oramap::{MapActor, OraMap, PlayerDef};
use openra_sim::actor::{Actor, ActorKind};
use openra_sim::gamerules::GameRules;
use openra_sim::math::{CPos, WPos};
use openra_sim::superweapon::SuperweaponKind;
use openra_sim::traits::TraitState;
use openra_sim::world::{
    self, insert_test_actor, set_test_unpaused, LobbyInfo, SlotInfo, World,
};

fn build_arena(seed: i32) -> World {
    let rules = GameRules::vendor_cached();
    let map = OraMap {
        title: "nuke-credit-test".into(),
        tileset: "TEMPERAT".into(),
        map_size: (40, 40),
        bounds: (0, 0, 40, 40),
        tiles: Vec::new(),
        actors: vec![
            MapActor { id: "mpspawn1".into(), actor_type: "mpspawn".into(),
                       owner: "Neutral".into(), location: (1, 1) },
            MapActor { id: "mpspawn2".into(), actor_type: "mpspawn".into(),
                       owner: "Neutral".into(), location: (38, 38) },
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
        starting_cash: 0,
        allow_spectators: false,
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
    w
}

fn make_building(id: u32, owner: u32, kind_name: &str, at: (i32, i32), hp: i32) -> Actor {
    let cell = CPos::new(at.0, at.1);
    let center = WPos::new(at.0 * 1024 + 512, at.1 * 1024 + 512, 0);
    Actor {
        id, kind: ActorKind::Building, owner_id: Some(owner), location: Some(at),
        traits: vec![
            TraitState::BodyOrientation { quantized_facings: 1 },
            TraitState::Building { top_left: cell },
            TraitState::Immobile { top_left: cell, center_position: center },
            TraitState::Health { hp },
        ],
        activity: None, actor_type: Some(kind_name.into()), kills: 0, rank: 0,
    }
}

fn make_e1(id: u32, owner: u32, at: (i32, i32), hp: i32) -> Actor {
    Actor {
        id, kind: ActorKind::Infantry, owner_id: Some(owner), location: Some(at),
        traits: vec![TraitState::Health { hp }],
        activity: None, actor_type: Some("e1".into()), kills: 0, rank: 0,
    }
}

fn fully_charge(world: &mut World, kind: SuperweaponKind, owner: u32) {
    for _ in 0..(kind.charge_ticks() + 5) {
        if world.superweapons.is_ready(kind, owner) { return; }
        let _ = world.tick(&[]);
    }
    assert!(world.superweapons.is_ready(kind, owner));
}

#[test]
fn nuke_aoe_kills_are_credited_to_firing_player() {
    let mut w = build_arena(17);
    let pids = w.player_ids().to_vec();
    let agent = pids[1];
    let enemy = pids[2];

    insert_test_actor(&mut w, make_building(1001, agent, "mslo", (5, 5), 100000));
    let cluster_centre = (25, 25);
    let n_enemies = 5;
    for (i, off) in [(0, 0), (1, 0), (0, 1), (-1, 0), (0, -1)].iter().enumerate() {
        let pos = (cluster_centre.0 + off.0, cluster_centre.1 + off.1);
        insert_test_actor(&mut w, make_e1(2000 + i as u32, enemy, pos, 50000));
    }

    let kills_before = w.kills_for_player(agent);
    assert_eq!(kills_before, 0, "agent has no kills before firing");

    fully_charge(&mut w, SuperweaponKind::Nuke, agent);

    let hit = w
        .fire_superweapon(SuperweaponKind::Nuke, agent, Some(cluster_centre), None)
        .expect("nuke should fire");
    assert!(hit as i32 >= n_enemies, "nuke should hit cluster");

    let kills_after = w.kills_for_player(agent);
    assert!(
        kills_after as i32 >= n_enemies,
        "expected at least {} kills credited to agent, got {} (was {})",
        n_enemies, kills_after, kills_before
    );
}
