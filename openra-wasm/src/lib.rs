//! Browser game client — replay viewer + live game mode.
//!
//! Compiles to WASM via wasm-pack. Supports two modes:
//! - ReplayViewer: load .orarep + .oramap, play back recorded game
//! - GameSession: start a new game vs bot AI on a bundled map

use std::collections::HashMap;
use wasm_bindgen::prelude::*;

use openra_data::{mix, oramap, orarep, palette, shp, tmp};
use openra_sim::world::{self, GameOrder, LobbyInfo, SlotInfo};

/// Bundled map for quick-start games.
const BUNDLED_MAP: &[u8] = include_bytes!("../../tests/maps/singles.oramap");
/// Bundled palette.
const BUNDLED_PALETTE: &[u8] = include_bytes!("../../vendor/OpenRA/mods/ra/maps/chernobyl/temperat.pal");
/// Bundled temperat.mix for terrain tiles.
const TEMPERAT_MIX: &[u8] = include_bytes!("../../vendor/ra-content/temperat.mix");
/// Bundled tileset YAML for template→image mapping.
const TEMPERAT_TILESET: &str = include_str!("../../vendor/OpenRA/mods/ra/tilesets/temperat.yaml");

/// Parse tileset YAML to get template_id → (image_filename, size_w, size_h) mapping.
fn parse_tileset_templates() -> HashMap<u16, (String, u32, u32)> {
    let mut templates = HashMap::new();
    let mut current_id: Option<u16> = None;
    let mut current_image: Option<String> = None;
    let mut current_size = (1u32, 1u32);

    for line in TEMPERAT_TILESET.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Template@") {
            // Flush previous template
            if let (Some(id), Some(img)) = (current_id, &current_image) {
                templates.insert(id, (img.clone(), current_size.0, current_size.1));
            }
            current_id = None;
            current_image = None;
            current_size = (1, 1);
        } else if trimmed.starts_with("Id:") && !trimmed.contains("TEMPERAT") {
            if let Ok(id) = trimmed[3..].trim().parse::<u16>() {
                current_id = Some(id);
            }
        } else if trimmed.starts_with("Images:") {
            current_image = Some(trimmed[7..].trim().to_string());
        } else if trimmed.starts_with("Size:") {
            let parts: Vec<&str> = trimmed[5..].trim().split(',').collect();
            if parts.len() == 2 {
                current_size = (
                    parts[0].trim().parse().unwrap_or(1),
                    parts[1].trim().parse().unwrap_or(1),
                );
            }
        }
    }
    // Flush last template
    if let (Some(id), Some(img)) = (current_id, current_image) {
        templates.insert(id, (img, current_size.0, current_size.1));
    }
    templates
}

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

// ── Replay Viewer ──────────────────────────────────────────────────────────

/// Serialize map tiles to JSON: [[{type_id, index}, ...], ...]
fn map_tiles_to_json(map: &oramap::OraMap) -> String {
    let mut rows = Vec::new();
    for row in &map.tiles {
        let cells: Vec<_> = row.iter().map(|t| {
            serde_json::json!([t.type_id, t.index])
        }).collect();
        rows.push(serde_json::Value::Array(cells));
    }
    serde_json::to_string(&rows).unwrap_or_else(|_| "[]".to_string())
}

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
    map_tiles_json: String,
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
        let map_tiles_json = map_tiles_to_json(&map);
        let world = world::build_world(&map, settings.random_seed, &lobby, None);
        let max_frame = replay.sync_hashes.last().map(|sh| sh.frame).unwrap_or(0);
        Ok(ReplayViewer {
            world, orders: replay.orders, sync_hashes: replay.sync_hashes,
            current_frame: 0, max_frame, map_tiles_json,
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
    pub fn map_tiles_json(&self) -> String { self.map_tiles_json.clone() }
}

// ── Game Session (vs Bot) ──────────────────────────────────────────────────

#[wasm_bindgen]
pub struct GameSession {
    world: world::World,
    human_player_id: u32,
    pending_orders: Vec<GameOrder>,
    frame: u32,
    map_tiles_json: String,
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

        let map_tiles_json = map_tiles_to_json(&map);
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
            map_tiles_json,
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

    pub fn map_tiles_json(&self) -> String { self.map_tiles_json.clone() }

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
/// bits/ overrides take priority over MIX-extracted versions.
const SPRITE_DATA: &[(&str, &[u8])] = &[
    // ── Vehicles (MIX-extracted, bits/ overrides where available) ──
    ("1tnk", include_bytes!("../../vendor/extracted-sprites/1tnk.shp")),
    ("2tnk", include_bytes!("../../vendor/extracted-sprites/2tnk.shp")),
    ("3tnk", include_bytes!("../../vendor/extracted-sprites/3tnk.shp")),
    ("4tnk", include_bytes!("../../vendor/extracted-sprites/4tnk.shp")),
    ("harv", include_bytes!("../../vendor/OpenRA/mods/ra/bits/harv.shp")),       // bits/ override
    ("mcv", include_bytes!("../../vendor/extracted-sprites/mcv.shp")),
    ("jeep", include_bytes!("../../vendor/extracted-sprites/jeep.shp")),
    ("apc", include_bytes!("../../vendor/extracted-sprites/apc.shp")),
    ("v2rl", include_bytes!("../../vendor/extracted-sprites/v2rl.shp")),
    ("arty", include_bytes!("../../vendor/extracted-sprites/arty.shp")),
    ("mnly", include_bytes!("../../vendor/extracted-sprites/mnly.shp")),
    ("mrj", include_bytes!("../../vendor/extracted-sprites/mrj.shp")),
    ("truk", include_bytes!("../../vendor/OpenRA/mods/ra/bits/truk.shp")),       // bits/ override
    ("stnk", include_bytes!("../../vendor/extracted-sprites/stnk.shp")),
    ("dog", include_bytes!("../../vendor/extracted-sprites/dog.shp")),
    ("mgg", include_bytes!("../../vendor/extracted-sprites/mgg.shp")),
    ("ftrk", include_bytes!("../../vendor/OpenRA/mods/ra/bits/ftrk.shp")),
    // Vehicle variants & husks (bits/)
    ("harvempty", include_bytes!("../../vendor/OpenRA/mods/ra/bits/harvempty.shp")),
    ("harvhalf", include_bytes!("../../vendor/OpenRA/mods/ra/bits/harvhalf.shp")),
    ("mcvhusk", include_bytes!("../../vendor/OpenRA/mods/ra/bits/mcvhusk.shp")),
    ("hhusk", include_bytes!("../../vendor/OpenRA/mods/ra/bits/hhusk.shp")),
    ("hhusk2", include_bytes!("../../vendor/OpenRA/mods/ra/bits/hhusk2.shp")),
    ("tire", include_bytes!("../../vendor/OpenRA/mods/ra/bits/tire.shp")),
    // ── Buildings (MIX-extracted, bits/ overrides where available) ──
    ("powr", include_bytes!("../../vendor/extracted-sprites/powr.shp")),
    ("apwr", include_bytes!("../../vendor/extracted-sprites/apwr.shp")),
    ("barr", include_bytes!("../../vendor/extracted-sprites/barr.shp")),
    ("fact", include_bytes!("../../vendor/OpenRA/mods/ra/bits/fact.shp")),        // bits/ override
    ("proc", include_bytes!("../../vendor/extracted-sprites/proc.shp")),
    ("weap", include_bytes!("../../vendor/extracted-sprites/weap.shp")),
    ("dome", include_bytes!("../../vendor/extracted-sprites/dome.shp")),
    ("gun", include_bytes!("../../vendor/extracted-sprites/gun.shp")),
    ("tent", include_bytes!("../../vendor/extracted-sprites/tent.shp")),
    ("tsla", include_bytes!("../../vendor/extracted-sprites/tsla.shp")),
    ("pbox", include_bytes!("../../vendor/extracted-sprites/pbox.shp")),
    ("gap", include_bytes!("../../vendor/OpenRA/mods/ra/bits/gap.shp")),          // bits/ override
    ("iron", include_bytes!("../../vendor/extracted-sprites/iron.shp")),
    ("fix", include_bytes!("../../vendor/extracted-sprites/fix.shp")),
    ("silo", include_bytes!("../../vendor/extracted-sprites/silo.shp")),
    ("atek", include_bytes!("../../vendor/extracted-sprites/atek.shp")),
    ("stek", include_bytes!("../../vendor/extracted-sprites/stek.shp")),
    ("ftur", include_bytes!("../../vendor/extracted-sprites/ftur.shp")),
    ("sam", include_bytes!("../../vendor/extracted-sprites/sam.shp")),
    ("weap2", include_bytes!("../../vendor/extracted-sprites/weap2.shp")),
    // Newly extracted buildings (from conquer.mix)
    ("hpad", include_bytes!("../../vendor/extracted-sprites/hpad.shp")),
    ("spen", include_bytes!("../../vendor/extracted-sprites/spen.shp")),
    ("syrd", include_bytes!("../../vendor/extracted-sprites/syrd.shp")),
    ("agun", include_bytes!("../../vendor/extracted-sprites/agun.shp")),
    ("kenn", include_bytes!("../../vendor/extracted-sprites/kenn.shp")),
    ("miss", include_bytes!("../../vendor/extracted-sprites/miss.shp")),
    ("pdox", include_bytes!("../../vendor/extracted-sprites/pdox.shp")),
    ("fcom", include_bytes!("../../vendor/extracted-sprites/fcom.shp")),
    ("hosp", include_bytes!("../../vendor/extracted-sprites/hosp.shp")),
    ("bio", include_bytes!("../../vendor/extracted-sprites/bio.shp")),
    ("afld", include_bytes!("../../vendor/extracted-sprites/afld.shp")),
    // Walls (from conquer.mix)
    ("brik", include_bytes!("../../vendor/extracted-sprites/brik.shp")),
    ("sbag", include_bytes!("../../vendor/extracted-sprites/sbag.shp")),
    ("fenc", include_bytes!("../../vendor/extracted-sprites/fenc.shp")),
    ("cycl", include_bytes!("../../vendor/extracted-sprites/cycl.shp")),
    ("barb", include_bytes!("../../vendor/extracted-sprites/barb.shp")),
    // Building overlays & variants (bits/)
    ("sam2", include_bytes!("../../vendor/OpenRA/mods/ra/bits/sam2.shp")),
    ("proctop", include_bytes!("../../vendor/OpenRA/mods/ra/bits/proctop.shp")),
    ("nopower", include_bytes!("../../vendor/OpenRA/mods/ra/bits/nopower.shp")),
    ("silo2", include_bytes!("../../vendor/OpenRA/mods/ra/bits/silo2.shp")),
    ("weap3", include_bytes!("../../vendor/OpenRA/mods/ra/bits/weap3.shp")),
    ("oilb", include_bytes!("../../vendor/OpenRA/mods/ra/bits/oilb.shp")),
    ("afldidle", include_bytes!("../../vendor/OpenRA/mods/ra/bits/afldidle.shp")),
    // Building construction animations (from conquer.mix)
    ("factmake", include_bytes!("../../vendor/extracted-sprites/factmake.shp")),
    ("procmake", include_bytes!("../../vendor/extracted-sprites/procmake.shp")),
    ("powrmake", include_bytes!("../../vendor/extracted-sprites/powrmake.shp")),
    ("apwrmake", include_bytes!("../../vendor/extracted-sprites/apwrmake.shp")),
    ("barrmake", include_bytes!("../../vendor/extracted-sprites/barrmake.shp")),
    ("domemake", include_bytes!("../../vendor/extracted-sprites/domemake.shp")),
    ("weapmake", include_bytes!("../../vendor/extracted-sprites/weapmake.shp")),
    ("gunmake", include_bytes!("../../vendor/extracted-sprites/gunmake.shp")),
    ("agunmake", include_bytes!("../../vendor/extracted-sprites/agunmake.shp")),
    ("sammake", include_bytes!("../../vendor/extracted-sprites/sammake.shp")),
    ("fturmake", include_bytes!("../../vendor/extracted-sprites/fturmake.shp")),
    ("tslamake", include_bytes!("../../vendor/extracted-sprites/tslamake.shp")),
    ("pboxmake", include_bytes!("../../vendor/extracted-sprites/pboxmake.shp")),
    ("stekmake", include_bytes!("../../vendor/extracted-sprites/stekmake.shp")),
    ("atekmake", include_bytes!("../../vendor/extracted-sprites/atekmake.shp")),
    ("hpadmake", include_bytes!("../../vendor/extracted-sprites/hpadmake.shp")),
    ("fixmake", include_bytes!("../../vendor/extracted-sprites/fixmake.shp")),
    ("gapmake", include_bytes!("../../vendor/extracted-sprites/gapmake.shp")),
    ("ironmake", include_bytes!("../../vendor/extracted-sprites/ironmake.shp")),
    ("spenmake", include_bytes!("../../vendor/extracted-sprites/spenmake.shp")),
    ("syrdmake", include_bytes!("../../vendor/extracted-sprites/syrdmake.shp")),
    ("afldmake", include_bytes!("../../vendor/extracted-sprites/afldmake.shp")),
    ("silomake", include_bytes!("../../vendor/extracted-sprites/silomake.shp")),
    ("tentmake", include_bytes!("../../vendor/extracted-sprites/tentmake.shp")),
    ("kennmake", include_bytes!("../../vendor/extracted-sprites/kennmake.shp")),
    ("pdoxmake", include_bytes!("../../vendor/extracted-sprites/pdoxmake.shp")),
    ("hospmake", include_bytes!("../../vendor/extracted-sprites/hospmake.shp")),
    ("biomake", include_bytes!("../../vendor/extracted-sprites/biomake.shp")),
    // Building construction/death from bits/
    ("factmake_b", include_bytes!("../../vendor/OpenRA/mods/ra/bits/factmake.shp")),
    ("factdead", include_bytes!("../../vendor/OpenRA/mods/ra/bits/factdead.shp")),
    ("powrdead", include_bytes!("../../vendor/OpenRA/mods/ra/bits/powrdead.shp")),
    ("apwrdead", include_bytes!("../../vendor/OpenRA/mods/ra/bits/apwrdead.shp")),
    ("procdead", include_bytes!("../../vendor/OpenRA/mods/ra/bits/procdead.shp")),
    ("fcommake", include_bytes!("../../vendor/OpenRA/mods/ra/bits/fcommake.shp")),
    ("gapmake_b", include_bytes!("../../vendor/OpenRA/mods/ra/bits/gapmake.shp")),
    ("missmake", include_bytes!("../../vendor/OpenRA/mods/ra/bits/missmake.shp")),
    ("silomake_b", include_bytes!("../../vendor/OpenRA/mods/ra/bits/silomake.shp")),
    // ── Aircraft (MIX-extracted, bits/ overrides) ──
    ("heli", include_bytes!("../../vendor/OpenRA/mods/ra/bits/heli.shp")),        // bits/ override
    ("hind", include_bytes!("../../vendor/OpenRA/mods/ra/bits/hind.shp")),        // bits/ override
    ("yak", include_bytes!("../../vendor/OpenRA/mods/ra/bits/yak.shp")),          // bits/ override
    ("mig", include_bytes!("../../vendor/extracted-sprites/mig.shp")),
    ("tran", include_bytes!("../../vendor/extracted-sprites/tran.shp")),
    ("tran2", include_bytes!("../../vendor/OpenRA/mods/ra/bits/tran2.shp")),
    ("tran1husk", include_bytes!("../../vendor/OpenRA/mods/ra/bits/tran1husk.shp")),
    ("tran2husk", include_bytes!("../../vendor/OpenRA/mods/ra/bits/tran2husk.shp")),
    ("badr", include_bytes!("../../vendor/extracted-sprites/badr.shp")),
    ("u2", include_bytes!("../../vendor/extracted-sprites/u2.shp")),
    ("mh60", include_bytes!("../../vendor/OpenRA/mods/ra/bits/mh60.shp")),
    // Rotors
    ("lrotor", include_bytes!("../../vendor/extracted-sprites/lrotor.shp")),
    ("rrotor", include_bytes!("../../vendor/extracted-sprites/rrotor.shp")),
    ("lrotorlg", include_bytes!("../../vendor/OpenRA/mods/ra/bits/lrotorlg.shp")),
    ("yrotorlg", include_bytes!("../../vendor/OpenRA/mods/ra/bits/yrotorlg.shp")),
    // ── Naval (from conquer.mix) ──
    ("ss", include_bytes!("../../vendor/extracted-sprites/ss.shp")),
    ("dd", include_bytes!("../../vendor/extracted-sprites/dd.shp")),
    ("ca", include_bytes!("../../vendor/extracted-sprites/ca.shp")),
    ("pt", include_bytes!("../../vendor/extracted-sprites/pt.shp")),
    ("lst", include_bytes!("../../vendor/extracted-sprites/lst.shp")),
    ("turr", include_bytes!("../../vendor/extracted-sprites/turr.shp")),
    ("mgun", include_bytes!("../../vendor/extracted-sprites/mgun.shp")),
    ("ssam", include_bytes!("../../vendor/extracted-sprites/ssam.shp")),
    // ── Infantry (bits/) ──
    ("e6", include_bytes!("../../vendor/OpenRA/mods/ra/bits/e6.shp")),
    ("zombie", include_bytes!("../../vendor/OpenRA/mods/ra/bits/zombie.shp")),
    ("c11", include_bytes!("../../vendor/OpenRA/mods/ra/bits/c11.shp")),
    ("chan", include_bytes!("../../vendor/OpenRA/mods/ra/bits/chan.shp")),
    ("einstein", include_bytes!("../../vendor/OpenRA/mods/ra/bits/einstein.shp")),
    ("jmin", include_bytes!("../../vendor/OpenRA/mods/ra/bits/jmin.shp")),
    // ── Weapon overlays ──
    ("minigun", include_bytes!("../../vendor/extracted-sprites/minigun.shp")),
    ("gunfire2", include_bytes!("../../vendor/OpenRA/mods/ra/bits/gunfire2.shp")),
    ("samfire", include_bytes!("../../vendor/extracted-sprites/samfire.shp")),
    // ── Effects & explosions (from conquer.mix) ──
    ("piff", include_bytes!("../../vendor/extracted-sprites/piff.shp")),
    ("piffpiff", include_bytes!("../../vendor/extracted-sprites/piffpiff.shp")),
    ("veh-hit1", include_bytes!("../../vendor/extracted-sprites/veh-hit1.shp")),
    ("veh-hit2", include_bytes!("../../vendor/extracted-sprites/veh-hit2.shp")),
    ("veh-hit3", include_bytes!("../../vendor/extracted-sprites/veh-hit3.shp")),
    ("flak", include_bytes!("../../vendor/extracted-sprites/flak.shp")),
    ("h2o_exp1", include_bytes!("../../vendor/extracted-sprites/h2o_exp1.shp")),
    ("h2o_exp2", include_bytes!("../../vendor/extracted-sprites/h2o_exp2.shp")),
    ("h2o_exp3", include_bytes!("../../vendor/extracted-sprites/h2o_exp3.shp")),
    ("art-exp1", include_bytes!("../../vendor/extracted-sprites/art-exp1.shp")),
    ("fball1", include_bytes!("../../vendor/extracted-sprites/fball1.shp")),
    ("frag1", include_bytes!("../../vendor/extracted-sprites/frag1.shp")),
    ("smoke_m", include_bytes!("../../vendor/extracted-sprites/smoke_m.shp")),
    ("burn-l", include_bytes!("../../vendor/extracted-sprites/burn-l.shp")),
    ("burn-m", include_bytes!("../../vendor/extracted-sprites/burn-m.shp")),
    ("burn-s", include_bytes!("../../vendor/extracted-sprites/burn-s.shp")),
    ("fire1", include_bytes!("../../vendor/extracted-sprites/fire1.shp")),
    ("fire2", include_bytes!("../../vendor/extracted-sprites/fire2.shp")),
    ("fire3", include_bytes!("../../vendor/extracted-sprites/fire3.shp")),
    ("fire4", include_bytes!("../../vendor/extracted-sprites/fire4.shp")),
    ("speed", include_bytes!("../../vendor/extracted-sprites/speed.shp")),
    ("120mm", include_bytes!("../../vendor/extracted-sprites/120mm.shp")),
    ("50cal", include_bytes!("../../vendor/extracted-sprites/50cal.shp")),
    ("v2", include_bytes!("../../vendor/extracted-sprites/v2.shp")),
    ("litning", include_bytes!("../../vendor/extracted-sprites/litning.shp")),
    ("bomblet", include_bytes!("../../vendor/extracted-sprites/bomblet.shp")),
    ("napalm1", include_bytes!("../../vendor/OpenRA/mods/ra/bits/napalm1.shp")),
    ("fb3", include_bytes!("../../vendor/OpenRA/mods/ra/bits/fb3.shp")),
    ("fb4", include_bytes!("../../vendor/OpenRA/mods/ra/bits/fb4.shp")),
    ("wpiff", include_bytes!("../../vendor/OpenRA/mods/ra/bits/wpiff.shp")),
    ("wpifpif", include_bytes!("../../vendor/OpenRA/mods/ra/bits/wpifpif.shp")),
    ("invun", include_bytes!("../../vendor/OpenRA/mods/ra/bits/invun.shp")),
    ("bubbles", include_bytes!("../../vendor/OpenRA/mods/ra/bits/bubbles.shp")),
    ("playersmoke", include_bytes!("../../vendor/OpenRA/mods/ra/bits/playersmoke.shp")),
    // ── UI indicators & misc (bits/) ──
    ("select", include_bytes!("../../vendor/extracted-sprites/select.shp")),
    ("flagfly", include_bytes!("../../vendor/extracted-sprites/flagfly.shp")),
    ("parach", include_bytes!("../../vendor/extracted-sprites/parach.shp")),
    ("rank", include_bytes!("../../vendor/OpenRA/mods/ra/bits/rank.shp")),
    ("pips2", include_bytes!("../../vendor/OpenRA/mods/ra/bits/pips2.shp")),
    ("poweroff", include_bytes!("../../vendor/OpenRA/mods/ra/bits/poweroff.shp")),
    ("allyrepair", include_bytes!("../../vendor/OpenRA/mods/ra/bits/allyrepair.shp")),
    ("attackmove", include_bytes!("../../vendor/OpenRA/mods/ra/bits/attackmove.shp")),
    ("beaconclock", include_bytes!("../../vendor/OpenRA/mods/ra/bits/beaconclock.shp")),
    ("iconchevrons", include_bytes!("../../vendor/OpenRA/mods/ra/bits/iconchevrons.shp")),
    ("levelup", include_bytes!("../../vendor/OpenRA/mods/ra/bits/levelup.shp")),
    ("pip-disguise", include_bytes!("../../vendor/OpenRA/mods/ra/bits/pip-disguise.shp")),
    ("tag-spy", include_bytes!("../../vendor/OpenRA/mods/ra/bits/tag-spy.shp")),
    ("camera", include_bytes!("../../vendor/OpenRA/mods/ra/bits/camera.shp")),
    ("gpsdot", include_bytes!("../../vendor/OpenRA/mods/ra/bits/gpsdot.shp")),
    ("mpspawn", include_bytes!("../../vendor/OpenRA/mods/ra/bits/mpspawn.shp")),
    ("waypoint", include_bytes!("../../vendor/OpenRA/mods/ra/bits/waypoint.shp")),
    ("parach-shadow", include_bytes!("../../vendor/OpenRA/mods/ra/bits/parach-shadow.shp")),
    ("ctflag", include_bytes!("../../vendor/OpenRA/mods/ra/bits/ctflag.shp")),
    // ── Icons (bits/) ──
    ("ateficon", include_bytes!("../../vendor/OpenRA/mods/ra/bits/ateficon.shp")),
    ("fapwicon", include_bytes!("../../vendor/OpenRA/mods/ra/bits/fapwicon.shp")),
    ("fixficon", include_bytes!("../../vendor/OpenRA/mods/ra/bits/fixficon.shp")),
    ("fpwricon", include_bytes!("../../vendor/OpenRA/mods/ra/bits/fpwricon.shp")),
    ("ftrkicon", include_bytes!("../../vendor/OpenRA/mods/ra/bits/ftrkicon.shp")),
    ("hospicon", include_bytes!("../../vendor/OpenRA/mods/ra/bits/hospicon.shp")),
    ("mh60icon", include_bytes!("../../vendor/OpenRA/mods/ra/bits/mh60icon.shp")),
    ("mslficon", include_bytes!("../../vendor/OpenRA/mods/ra/bits/mslficon.shp")),
    ("msloicon2", include_bytes!("../../vendor/OpenRA/mods/ra/bits/msloicon2.shp")),
    ("pdoficon", include_bytes!("../../vendor/OpenRA/mods/ra/bits/pdoficon.shp")),
    ("stnkicon", include_bytes!("../../vendor/OpenRA/mods/ra/bits/stnkicon.shp")),
    ("tenficon", include_bytes!("../../vendor/OpenRA/mods/ra/bits/tenficon.shp")),
    ("zombicon", include_bytes!("../../vendor/OpenRA/mods/ra/bits/zombicon.shp")),
    ("anticon", include_bytes!("../../vendor/OpenRA/mods/ra/bits/anticon.shp")),
    // ── Crates (bits/) ──
    ("scrate", include_bytes!("../../vendor/extracted-sprites/scrate.shp")),
    ("wcrate", include_bytes!("../../vendor/extracted-sprites/wcrate.shp")),
    ("xcratea", include_bytes!("../../vendor/OpenRA/mods/ra/bits/xcratea.shp")),
    ("xcrateb", include_bytes!("../../vendor/OpenRA/mods/ra/bits/xcrateb.shp")),
    ("xcratec", include_bytes!("../../vendor/OpenRA/mods/ra/bits/xcratec.shp")),
    ("xcrated", include_bytes!("../../vendor/OpenRA/mods/ra/bits/xcrated.shp")),
    // ── Decorative (bits/) ──
    ("asianhut", include_bytes!("../../vendor/OpenRA/mods/ra/bits/asianhut.shp")),
    ("rushouse", include_bytes!("../../vendor/OpenRA/mods/ra/bits/rushouse.shp")),
    ("snowhut", include_bytes!("../../vendor/OpenRA/mods/ra/bits/snowhut.shp")),
    ("windmill", include_bytes!("../../vendor/OpenRA/mods/ra/bits/windmill.shp")),
    ("lhus", include_bytes!("../../vendor/OpenRA/mods/ra/bits/lhus.shp")),
    ("utilpol1", include_bytes!("../../vendor/OpenRA/mods/ra/bits/utilpol1.shp")),
    ("utilpol2", include_bytes!("../../vendor/OpenRA/mods/ra/bits/utilpol2.shp")),
    ("tanktrap1", include_bytes!("../../vendor/OpenRA/mods/ra/bits/tanktrap1.shp")),
    ("tanktrap2", include_bytes!("../../vendor/OpenRA/mods/ra/bits/tanktrap2.shp")),
    ("ammobox1", include_bytes!("../../vendor/OpenRA/mods/ra/bits/ammobox1.shp")),
    ("ammobox2", include_bytes!("../../vendor/OpenRA/mods/ra/bits/ammobox2.shp")),
    ("ammobox3", include_bytes!("../../vendor/OpenRA/mods/ra/bits/ammobox3.shp")),
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
    indexed: Vec<u8>, // Raw palette indices for recoloring
}

#[wasm_bindgen]
impl SpriteAtlas {
    /// Decode all bundled sprites using the bundled palette.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Result<SpriteAtlas, JsValue> {
        let pal = palette::Palette::from_bytes(BUNDLED_PALETTE)
            .map_err(|e| JsValue::from_str(&format!("Palette error: {}", e)))?;

        let mut sprites = HashMap::new();

        // Decode SHP sprites
        for &(name, data) in SPRITE_DATA {
            match shp::decode(data) {
                Ok(shp_file) => {
                    let frames: Vec<SpriteFrame> = shp_file.frames.iter().map(|f| {
                        let mut rgba = Vec::with_capacity(f.pixels.len() * 4);
                        for &px in &f.pixels {
                            if px == 0 {
                                rgba.extend_from_slice(&[0, 0, 0, 0]); // transparent
                            } else if px == 4 {
                                rgba.extend_from_slice(&[0, 0, 0, 128]); // shadow
                            } else {
                                let c = pal.rgba(px);
                                rgba.extend_from_slice(&c);
                            }
                        }
                        SpriteFrame { width: f.width, height: f.height, rgba, indexed: f.pixels.clone() }
                    }).collect();
                    sprites.insert(name.to_string(), frames);
                }
                Err(_) => {}
            }
        }

        // Decode terrain tiles from temperat.mix using tileset templates
        let templates = parse_tileset_templates();
        if let Ok(tmix) = mix::MixArchive::parse(TEMPERAT_MIX.to_vec()) {
            // Collect unique .tem filenames
            let mut loaded: std::collections::HashSet<String> = std::collections::HashSet::new();
            for (_id, (filename, _sw, _sh)) in &templates {
                if loaded.contains(filename) { continue; }
                loaded.insert(filename.clone());
                if let Some(tem_data) = tmix.get(filename) {
                    if let Ok(tmp_file) = tmp::decode(tem_data) {
                        let sprite_name = format!("ter:{}", filename);
                        let frames: Vec<SpriteFrame> = tmp_file.tiles.iter().map(|tile| {
                            match tile {
                                Some(pixels) => {
                                    let mut rgba = Vec::with_capacity(pixels.len() * 4);
                                    for &px in pixels {
                                        if px == 0 {
                                            rgba.extend_from_slice(&[0, 0, 0, 255]);
                                        } else {
                                            let c = pal.rgba(px);
                                            rgba.extend_from_slice(&c);
                                        }
                                    }
                                    SpriteFrame {
                                        width: tmp_file.width,
                                        height: tmp_file.height,
                                        rgba,
                                        indexed: pixels.clone(),
                                    }
                                }
                                None => {
                                    // Empty tile — transparent
                                    let len = tmp_file.width as usize * tmp_file.height as usize;
                                    SpriteFrame {
                                        width: tmp_file.width,
                                        height: tmp_file.height,
                                        rgba: vec![0; len * 4],
                                        indexed: vec![0; len],
                                    }
                                }
                            }
                        }).collect();
                        sprites.insert(sprite_name, frames);
                    }
                }
            }

            // Also load decoration/resource .tem sprites not in tileset templates
            // (trees, mines, gems)
            let extra_tems = [
                "mine.tem", "gmine.tem",
                "t01.tem", "t02.tem", "t03.tem", "t04.tem", "t05.tem",
                "t06.tem", "t07.tem", "t08.tem", "t09.tem", "t10.tem",
                "t11.tem", "t12.tem", "t13.tem", "t14.tem", "t15.tem",
                "t16.tem", "t17.tem",
                "tc01.tem", "tc02.tem", "tc03.tem", "tc04.tem", "tc05.tem",
            ];
            for filename in &extra_tems {
                let sprite_name = format!("ter:{}", filename);
                if sprites.contains_key(&sprite_name) { continue; }
                if let Some(tem_data) = tmix.get(filename) {
                    if let Ok(tmp_file) = tmp::decode(tem_data) {
                        let frames: Vec<SpriteFrame> = tmp_file.tiles.iter().map(|tile| {
                            match tile {
                                Some(pixels) => {
                                    let mut rgba = Vec::with_capacity(pixels.len() * 4);
                                    for &px in pixels {
                                        if px == 0 {
                                            rgba.extend_from_slice(&[0, 0, 0, 255]);
                                        } else {
                                            let c = pal.rgba(px);
                                            rgba.extend_from_slice(&c);
                                        }
                                    }
                                    SpriteFrame {
                                        width: tmp_file.width,
                                        height: tmp_file.height,
                                        rgba,
                                        indexed: pixels.clone(),
                                    }
                                }
                                None => {
                                    let len = tmp_file.width as usize * tmp_file.height as usize;
                                    SpriteFrame {
                                        width: tmp_file.width,
                                        height: tmp_file.height,
                                        rgba: vec![0; len * 4],
                                        indexed: vec![0; len],
                                    }
                                }
                            }
                        }).collect();
                        sprites.insert(sprite_name, frames);
                    }
                }
            }
        }

        // Load building foundation .tem sprites from bits/
        let bits_tems: &[(&str, &[u8])] = &[
            ("mbAGUN.tem", include_bytes!("../../vendor/OpenRA/mods/ra/bits/mbAGUN.tem")),
            ("mbFIX.tem", include_bytes!("../../vendor/OpenRA/mods/ra/bits/mbFIX.tem")),
            ("mbFTUR.tem", include_bytes!("../../vendor/OpenRA/mods/ra/bits/mbFTUR.tem")),
            ("mbGAP.tem", include_bytes!("../../vendor/OpenRA/mods/ra/bits/mbGAP.tem")),
            ("mbGUN.tem", include_bytes!("../../vendor/OpenRA/mods/ra/bits/mbGUN.tem")),
            ("mbHOSP.tem", include_bytes!("../../vendor/OpenRA/mods/ra/bits/mbHOSP.tem")),
            ("mbIRON.tem", include_bytes!("../../vendor/OpenRA/mods/ra/bits/mbIRON.tem")),
            ("mbPBOX.tem", include_bytes!("../../vendor/OpenRA/mods/ra/bits/mbPBOX.tem")),
            ("mbPDOX.tem", include_bytes!("../../vendor/OpenRA/mods/ra/bits/mbPDOX.tem")),
            ("mbSAM.tem", include_bytes!("../../vendor/OpenRA/mods/ra/bits/mbSAM.tem")),
            ("mbSILO.tem", include_bytes!("../../vendor/OpenRA/mods/ra/bits/mbSILO.tem")),
            ("mbTSLA.tem", include_bytes!("../../vendor/OpenRA/mods/ra/bits/mbTSLA.tem")),
        ];
        for &(filename, data) in bits_tems {
            let sprite_name = format!("ter:{}", filename);
            if sprites.contains_key(&sprite_name) { continue; }
            if let Ok(tmp_file) = tmp::decode(data) {
                let frames: Vec<SpriteFrame> = tmp_file.tiles.iter().map(|tile| {
                    match tile {
                        Some(pixels) => {
                            let mut rgba = Vec::with_capacity(pixels.len() * 4);
                            for &px in pixels {
                                if px == 0 {
                                    rgba.extend_from_slice(&[0, 0, 0, 255]);
                                } else {
                                    let c = pal.rgba(px);
                                    rgba.extend_from_slice(&c);
                                }
                            }
                            SpriteFrame { width: tmp_file.width, height: tmp_file.height, rgba, indexed: pixels.clone() }
                        }
                        None => {
                            let len = tmp_file.width as usize * tmp_file.height as usize;
                            SpriteFrame { width: tmp_file.width, height: tmp_file.height, rgba: vec![0; len * 4], indexed: vec![0; len] }
                        }
                    }
                }).collect();
                sprites.insert(sprite_name, frames);
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

    /// Get raw palette indices for a sprite frame (for player color remapping).
    pub fn frame_indexed(&self, name: &str, frame_index: usize) -> Vec<u8> {
        self.sprites.get(name)
            .and_then(|frames| frames.get(frame_index))
            .map(|f| f.indexed.clone())
            .unwrap_or_default()
    }

    /// Get the full 256-entry palette as flat RGB array (768 bytes).
    pub fn palette_rgb(&self) -> Vec<u8> {
        let pal = palette::Palette::from_bytes(BUNDLED_PALETTE).unwrap();
        let mut data = Vec::with_capacity(768);
        for color in &pal.colors {
            data.extend_from_slice(color);
        }
        data
    }

    /// Get tileset template mapping as JSON: { "template_id": { "image": "file.tem", "sw": 1, "sh": 1 }, ... }
    pub fn tileset_json(&self) -> String {
        let templates = parse_tileset_templates();
        let mut map = serde_json::Map::new();
        for (id, (image, sw, sh)) in &templates {
            map.insert(id.to_string(), serde_json::json!({
                "image": image,
                "sw": sw,
                "sh": sh,
            }));
        }
        serde_json::Value::Object(map).to_string()
    }
}
