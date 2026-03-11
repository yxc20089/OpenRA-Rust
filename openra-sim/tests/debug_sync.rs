//! Debug test: dump detailed SyncHash components to find the mismatch.

use openra_data::{oramap, orarep};
use openra_sim::world::{self, LobbyInfo, SlotInfo};

#[test]
fn debug_sync_components() {
    let replay_data = std::fs::read(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/replays/ra-2026-02-20T001259Z.orarep")
    ).unwrap();
    let replay = orarep::parse(&replay_data).unwrap();
    let settings = replay.lobby_settings().unwrap();

    let map_data = std::fs::read(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/maps/singles.oramap")
    ).unwrap();
    let map = oramap::parse(&map_data).unwrap();

    let lobby = LobbyInfo {
        starting_cash: settings.starting_cash,
        allow_spectators: settings.allow_spectators,
        occupied_slots: settings.occupied_slots.iter().map(|(_, pr, f)| SlotInfo {
            player_reference: pr.clone(),
            faction: f.clone(),
        }).collect(),
    };

    let w = world::build_world(&map, settings.random_seed, &lobby);
    eprintln!("=== Per-actor SyncHash dump ===");
    w.dump_sync_details();
    eprintln!("\nExpected: 605399687");
}
