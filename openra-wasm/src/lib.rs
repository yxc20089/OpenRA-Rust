//! Browser game client — replay viewer + live game mode.
//!
//! Compiles to WASM via wasm-pack. Supports two modes:
//! - ReplayViewer: load .orarep + .oramap, play back recorded game
//! - GameSession: start a new game vs bot AI on a bundled map

use wasm_bindgen::prelude::*;

use openra_data::{oramap, orarep};
use openra_sim::world::{self, GameOrder, LobbyInfo, SlotInfo};

/// Bundled map for quick-start games.
const BUNDLED_MAP: &[u8] = include_bytes!("../../tests/maps/singles.oramap");

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

// ── Replay Viewer ──────────────────────────────────────────────────────────

fn lobby_from_replay(replay: &orarep::Replay) -> LobbyInfo {
    let settings = replay.lobby_settings().expect("No lobby settings in replay");
    let occupied_slots = settings
        .occupied_slots
        .iter()
        .map(|(_, player_ref, faction)| SlotInfo {
            player_reference: player_ref.clone(),
            faction: faction.clone(),
            is_bot: false,
        })
        .collect();
    LobbyInfo {
        starting_cash: settings.starting_cash,
        allow_spectators: settings.allow_spectators,
        occupied_slots,
    }
}

const SKIP_ORDERS: &[&str] = &[
    "SyncInfo", "SyncLobbyClients", "SyncLobbySlots",
    "HandshakeResponse", "HandshakeRequest", "SyncConnectionQuality",
    "FluentMessage", "StartGame",
];

#[wasm_bindgen]
pub struct ReplayViewer {
    world: world::World,
    orders: Vec<(i32, orarep::Order)>,
    #[allow(dead_code)]
    sync_hashes: Vec<orarep::SyncHashEntry>,
    current_frame: usize,
    max_frame: i32,
}

#[wasm_bindgen]
impl ReplayViewer {
    #[wasm_bindgen(constructor)]
    pub fn new(replay_bytes: &[u8], map_bytes: &[u8]) -> Result<ReplayViewer, JsValue> {
        let replay = orarep::parse(replay_bytes)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse replay: {}", e)))?;
        let map = oramap::parse(map_bytes)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse map: {}", e)))?;
        let settings = replay.lobby_settings()
            .ok_or_else(|| JsValue::from_str("No lobby settings in replay"))?;
        let lobby = lobby_from_replay(&replay);
        let world = world::build_world(&map, settings.random_seed, &lobby, None);
        let max_frame = replay.sync_hashes.last().map(|sh| sh.frame).unwrap_or(0);
        Ok(ReplayViewer {
            world, orders: replay.orders, sync_hashes: replay.sync_hashes,
            current_frame: 0, max_frame,
        })
    }

    pub fn tick(&mut self) -> bool {
        self.current_frame += 1;
        let frame = self.current_frame as i32;
        if frame > self.max_frame { return false; }
        let orders: Vec<GameOrder> = self.orders.iter()
            .filter(|(f, o)| *f == frame && !SKIP_ORDERS.contains(&o.order_string.as_str()))
            .map(|(_, o)| GameOrder {
                order_string: o.order_string.clone(),
                subject_id: o.subject_id,
                target_string: o.target_string.clone(),
                extra_data: o.extra_data,
            })
            .collect();
        self.world.process_frame(&orders);
        true
    }

    pub fn snapshot_json(&self) -> String {
        serde_json::to_string(&self.world.snapshot()).unwrap_or_default()
    }

    pub fn current_frame(&self) -> i32 { self.current_frame as i32 }
    pub fn total_frames(&self) -> i32 { self.max_frame }
}

// ── Game Session (vs Bot) ──────────────────────────────────────────────────

#[wasm_bindgen]
pub struct GameSession {
    world: world::World,
    human_player_id: u32,
    pending_orders: Vec<GameOrder>,
    frame: u32,
}

#[wasm_bindgen]
impl GameSession {
    /// Start a new game: human player vs 1 bot AI on the bundled map.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Result<GameSession, JsValue> {
        let map = oramap::parse(BUNDLED_MAP)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse bundled map: {}", e)))?;

        let seed = (js_sys::Math::random() * 2_147_483_647.0) as i32;

        let lobby = LobbyInfo {
            starting_cash: 5000,
            allow_spectators: true,
            occupied_slots: vec![
                SlotInfo {
                    player_reference: "Multi0".to_string(),
                    faction: "soviet".to_string(),
                    is_bot: false,
                },
                SlotInfo {
                    player_reference: "Multi1".to_string(),
                    faction: "soviet".to_string(),
                    is_bot: true,
                },
            ],
        };

        let world = world::build_world(&map, seed, &lobby, None);
        // Human is first playable player (after World=1, Neutral=2, Creeps=... )
        // Player IDs: the first two non-system player actors
        let player_ids = world.player_ids().to_vec();
        // Player IDs: [non-playable..., playable_slot_0, playable_slot_1, Everyone]
        // Human is the first occupied slot = slot count + 1 from the end
        let num_slots = lobby.occupied_slots.len(); // 2
        let human_player_id = player_ids[player_ids.len() - 1 - num_slots]; // skip Everyone, take first

        Ok(GameSession {
            world,
            human_player_id,
            pending_orders: Vec::new(),
            frame: 0,
        })
    }

    /// Advance one network frame. Returns false if game is over.
    pub fn tick(&mut self) -> bool {
        self.frame += 1;
        let orders: Vec<GameOrder> = self.pending_orders.drain(..).collect();
        self.world.process_frame(&orders);
        self.world.game_over().is_none()
    }

    pub fn snapshot_json(&self) -> String {
        serde_json::to_string(&self.world.snapshot()).unwrap_or_default()
    }

    pub fn human_player_id(&self) -> u32 { self.human_player_id }
    pub fn current_frame(&self) -> u32 { self.frame }

    /// Get buildable items for the human player as JSON.
    pub fn buildable_items_json(&self) -> String {
        let items = self.world.buildable_items(self.human_player_id);
        serde_json::to_string(&items).unwrap_or_default()
    }

    /// Check if building can be placed at (x, y).
    pub fn can_place_building(&self, building_type: &str, x: i32, y: i32) -> bool {
        self.world.can_place_building(self.human_player_id, building_type, x, y)
    }

    /// Get game over winner (0 = not over, otherwise winner player ID).
    pub fn winner(&self) -> u32 {
        self.world.game_over().unwrap_or(0)
    }

    // ── Typed order methods ────────────────────────────────────────────

    pub fn order_move(&mut self, unit_id: u32, x: i32, y: i32) {
        self.pending_orders.push(GameOrder {
            order_string: "Move".into(),
            subject_id: Some(unit_id),
            target_string: Some(format!("{},{}", x, y)),
            extra_data: None,
        });
    }

    pub fn order_attack(&mut self, unit_id: u32, target_id: u32) {
        self.pending_orders.push(GameOrder {
            order_string: "Attack".into(),
            subject_id: Some(unit_id),
            target_string: None,
            extra_data: Some(target_id),
        });
    }

    pub fn order_attack_move(&mut self, unit_id: u32, x: i32, y: i32) {
        self.pending_orders.push(GameOrder {
            order_string: "AttackMove".into(),
            subject_id: Some(unit_id),
            target_string: Some(format!("{},{}", x, y)),
            extra_data: None,
        });
    }

    pub fn order_stop(&mut self, unit_id: u32) {
        self.pending_orders.push(GameOrder {
            order_string: "Stop".into(),
            subject_id: Some(unit_id),
            target_string: None,
            extra_data: None,
        });
    }

    pub fn order_start_production(&mut self, item_name: &str) {
        self.pending_orders.push(GameOrder {
            order_string: "StartProduction".into(),
            subject_id: Some(self.human_player_id),
            target_string: Some(item_name.to_string()),
            extra_data: None,
        });
    }

    pub fn order_place_building(&mut self, building_type: &str, x: i32, y: i32) {
        self.pending_orders.push(GameOrder {
            order_string: "PlaceBuilding".into(),
            subject_id: Some(self.human_player_id),
            target_string: Some(format!("{},{},{}", building_type, x, y)),
            extra_data: None,
        });
    }

    pub fn order_deploy(&mut self, unit_id: u32) {
        self.pending_orders.push(GameOrder {
            order_string: "DeployTransform".into(),
            subject_id: Some(unit_id),
            target_string: None,
            extra_data: None,
        });
    }

    pub fn order_sell(&mut self, building_id: u32) {
        self.pending_orders.push(GameOrder {
            order_string: "Sell".into(),
            subject_id: Some(building_id),
            target_string: None,
            extra_data: None,
        });
    }
}
