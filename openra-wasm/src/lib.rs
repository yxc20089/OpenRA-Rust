//! Browser game client — replay viewer + live game mode.
//!
//! Compiles to WASM via wasm-pack. Supports two modes:
//! - ReplayViewer: load .orarep + .oramap, play back recorded game
//! - GameSession: start a new game vs bot AI on a bundled map

use std::collections::HashMap;
use wasm_bindgen::prelude::*;

use openra_data::{oramap, orarep, palette, shp};
use openra_sim::world::{self, GameOrder, LobbyInfo, SlotInfo};

/// Bundled map for quick-start games.
const BUNDLED_MAP: &[u8] = include_bytes!("../../tests/maps/singles.oramap");
/// Bundled palette.
const BUNDLED_PALETTE: &[u8] = include_bytes!("../../vendor/OpenRA/mods/ra/maps/chernobyl/temperat.pal");

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

// ── Sprite Atlas ───────────────────────────────────────────────────────────

/// Bundled SHP sprite files.
const SPRITE_DATA: &[(&str, &[u8])] = &[
    ("fact", include_bytes!("../../vendor/OpenRA/mods/ra/bits/fact.shp")),
    ("harv", include_bytes!("../../vendor/OpenRA/mods/ra/bits/harv.shp")),
    ("heli", include_bytes!("../../vendor/OpenRA/mods/ra/bits/heli.shp")),
    ("hind", include_bytes!("../../vendor/OpenRA/mods/ra/bits/hind.shp")),
    ("yak", include_bytes!("../../vendor/OpenRA/mods/ra/bits/yak.shp")),
    ("e6", include_bytes!("../../vendor/OpenRA/mods/ra/bits/e6.shp")),
    ("gap", include_bytes!("../../vendor/OpenRA/mods/ra/bits/gap.shp")),
    ("truk", include_bytes!("../../vendor/OpenRA/mods/ra/bits/truk.shp")),
    ("ftrk", include_bytes!("../../vendor/OpenRA/mods/ra/bits/ftrk.shp")),
    ("sam2", include_bytes!("../../vendor/OpenRA/mods/ra/bits/sam2.shp")),
    ("proctop", include_bytes!("../../vendor/OpenRA/mods/ra/bits/proctop.shp")),
    ("nopower", include_bytes!("../../vendor/OpenRA/mods/ra/bits/nopower.shp")),
];

/// A decoded sprite: name → list of frames, each with (width, height, rgba_data).
#[wasm_bindgen]
pub struct SpriteAtlas {
    sprites: HashMap<String, Vec<SpriteFrame>>,
}

struct SpriteFrame {
    width: u16,
    height: u16,
    rgba: Vec<u8>,
}

#[wasm_bindgen]
impl SpriteAtlas {
    /// Decode all bundled sprites using the bundled palette.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Result<SpriteAtlas, JsValue> {
        let pal = palette::Palette::from_bytes(BUNDLED_PALETTE)
            .map_err(|e| JsValue::from_str(&format!("Palette error: {}", e)))?;

        let mut sprites = HashMap::new();
        for &(name, data) in SPRITE_DATA {
            match shp::decode(data) {
                Ok(shp_file) => {
                    let frames: Vec<SpriteFrame> = shp_file.frames.iter().map(|f| {
                        let mut rgba = Vec::with_capacity(f.pixels.len() * 4);
                        for &px in &f.pixels {
                            let c = pal.rgba(px);
                            rgba.extend_from_slice(&c);
                        }
                        SpriteFrame { width: f.width, height: f.height, rgba }
                    }).collect();
                    sprites.insert(name.to_string(), frames);
                }
                Err(_) => {} // Skip failed decodes
            }
        }
        Ok(SpriteAtlas { sprites })
    }

    /// Get sprite info as JSON: { name: { width, height, frames } }
    pub fn info_json(&self) -> String {
        let mut info = HashMap::new();
        for (name, frames) in &self.sprites {
            if let Some(f) = frames.first() {
                info.insert(name.clone(), serde_json::json!({
                    "width": f.width,
                    "height": f.height,
                    "frames": frames.len(),
                }));
            }
        }
        serde_json::to_string(&info).unwrap_or_default()
    }

    /// Get RGBA pixel data for a specific sprite frame.
    /// Returns empty vec if not found.
    pub fn frame_rgba(&self, name: &str, frame_index: usize) -> Vec<u8> {
        self.sprites.get(name)
            .and_then(|frames| frames.get(frame_index))
            .map(|f| f.rgba.clone())
            .unwrap_or_default()
    }

    /// Get sprite width.
    pub fn width(&self, name: &str) -> u16 {
        self.sprites.get(name)
            .and_then(|f| f.first())
            .map(|f| f.width)
            .unwrap_or(0)
    }

    /// Get sprite height.
    pub fn height(&self, name: &str) -> u16 {
        self.sprites.get(name)
            .and_then(|f| f.first())
            .map(|f| f.height)
            .unwrap_or(0)
    }

    /// Get frame count for a sprite.
    pub fn frame_count(&self, name: &str) -> usize {
        self.sprites.get(name).map(|f| f.len()).unwrap_or(0)
    }
}
