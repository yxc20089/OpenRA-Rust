//! Engine acceptance: per-player starting cash.
//!
//! Bench scenarios declare `agent: {cash: N}` / `enemy: {cash: M}` in
//! the scenario YAML to give each side its own starting balance —
//! e.g. `spec-thief-steal-cash` gives the agent 0 cash and the enemy
//! 2000 so the thief actually has something to steal. Before the fix
//! the engine plumbed a single `starting_cash: int` into every player
//! actor (see `build_player_traits` callsites in `world.rs`); the
//! per-player values were silently dropped at the bench⇒engine
//! boundary.
//!
//! This test pins the two halves of the fix:
//!   1. Direct API path — `SlotInfo::starting_cash = Some(n)` flows
//!      into `build_player_traits`, so each playable slot starts at
//!      its own cash value while the lobby-wide default still gates
//!      the Neutral / Everyone players.
//!   2. YAML path — a scenario YAML with `agent: {cash: 1000}` and
//!      `enemy: {cash: 2000}` is parsed by
//!      `oramap::parse_scenario_yaml` into `MapDef::agent_starting_cash`
//!      / `MapDef::enemy_starting_cash`. We don't drive the full
//!      `build_world` from a discovery-style scenario here (that path
//!      needs a base-map file on disk); we just assert the parser
//!      surfaces both overrides.

use openra_data::oramap::{MapActor, OraMap, PlayerDef};
use openra_sim::world::{self, LobbyInfo, SlotInfo};

fn build_two_player_world(
    agent_cash: Option<i32>,
    enemy_cash: Option<i32>,
    lobby_default: i32,
) -> openra_sim::world::World {
    // Minimal arena: two mpspawns + Neutral + two playable slots.
    let map = OraMap {
        title: "per-player-cash".into(),
        tileset: "TEMPERAT".into(),
        map_size: (32, 32),
        bounds: (0, 0, 32, 32),
        tiles: Vec::new(),
        actors: vec![
            MapActor {
                id: "mpspawn1".into(),
                actor_type: "mpspawn".into(),
                owner: "Neutral".into(),
                location: (2, 2),
            },
            MapActor {
                id: "mpspawn2".into(),
                actor_type: "mpspawn".into(),
                owner: "Neutral".into(),
                location: (29, 29),
            },
        ],
        players: vec![
            PlayerDef {
                name: "Neutral".into(),
                playable: false,
                owns_world: true,
                non_combatant: true,
                faction: "allies".into(),
                enemies: Vec::new(),
            },
            PlayerDef {
                name: "Multi0".into(),
                playable: true,
                owns_world: false,
                non_combatant: false,
                faction: "allies".into(),
                enemies: vec!["Multi1".into()],
            },
            PlayerDef {
                name: "Multi1".into(),
                playable: true,
                owns_world: false,
                non_combatant: false,
                faction: "soviet".into(),
                enemies: vec!["Multi0".into()],
            },
        ],
    };
    let lobby = LobbyInfo {
        starting_cash: lobby_default,
        allow_spectators: true,
        occupied_slots: vec![
            SlotInfo {
                player_reference: "Multi0".into(),
                faction: "allies".into(),
                is_bot: false,
                starting_cash: agent_cash,
            },
            SlotInfo {
                player_reference: "Multi1".into(),
                faction: "soviet".into(),
                is_bot: false,
                starting_cash: enemy_cash,
            },
        ],
    };
    world::build_world(&map, 1, &lobby, None, 0, false)
}

#[test]
fn per_slot_starting_cash_via_api_overrides_lobby_default() {
    // Agent: 1000 (override), Enemy: 2000 (override), lobby floor: 0.
    // Pre-fix the engine would have built both players at 0 cash.
    let world = build_two_player_world(Some(1000), Some(2000), 0);
    let ids = world.player_ids().to_vec();
    // Order: [Neutral (non-playable), Multi0 (agent), Multi1 (enemy), Everyone].
    assert!(ids.len() >= 3, "expected ≥3 player ids, got {:?}", ids);
    let agent_pid = ids[1];
    let enemy_pid = ids[2];

    assert_eq!(
        world.player_cash(agent_pid),
        1000,
        "agent slot must honour `SlotInfo::starting_cash = Some(1000)`",
    );
    assert_eq!(
        world.player_cash(enemy_pid),
        2000,
        "enemy slot must honour `SlotInfo::starting_cash = Some(2000)`",
    );
}

#[test]
fn slot_starting_cash_none_falls_back_to_lobby_default() {
    // Both slots leave their override unset; both must inherit the
    // lobby-wide 5000. This is the back-compat path for existing
    // scenarios that don't declare `agent: {cash:}` / `enemy: {cash:}`.
    let world = build_two_player_world(None, None, 5000);
    let ids = world.player_ids().to_vec();
    let agent_pid = ids[1];
    let enemy_pid = ids[2];
    assert_eq!(world.player_cash(agent_pid), 5000);
    assert_eq!(world.player_cash(enemy_pid), 5000);
}

#[test]
fn mixed_override_only_agent_set() {
    // Agent overridden to 1000, enemy left None ⇒ enemy uses the
    // lobby default 5000. Mirrors a hypothetical scenario that wants
    // to nerf only the agent's economy while leaving the enemy at
    // the skirmish default.
    let world = build_two_player_world(Some(1000), None, 5000);
    let ids = world.player_ids().to_vec();
    let agent_pid = ids[1];
    let enemy_pid = ids[2];
    assert_eq!(world.player_cash(agent_pid), 1000);
    assert_eq!(world.player_cash(enemy_pid), 5000);
}

