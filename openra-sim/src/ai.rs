//! Simple bot AI for Red Alert.
//!
//! Generates orders each tick: deploys MCV, builds power, barracks, war factory,
//! produces units, sends them to attack. Roughly based on OpenRA's
//! HackyAI bot modules.

use crate::actor::ActorKind;
use crate::world::{GameOrder, World};

/// AI difficulty level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Difficulty {
    Easy = 0,
    Medium = 1,
    Hard = 2,
}

impl Difficulty {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Difficulty::Easy,
            1 => Difficulty::Medium,
            _ => Difficulty::Hard,
        }
    }
}

/// AI state for one bot player.
#[derive(Debug)]
pub struct Bot {
    /// The player actor ID this bot controls.
    pub player_id: u32,
    /// Ticks since last action.
    ticks_idle: u32,
    /// Current strategic state.
    state: BotState,
    /// Whether we've deployed our MCV.
    mcv_deployed: bool,
    /// Whether we've placed our first power plant.
    has_power: bool,
    /// Whether we've queued barracks.
    has_barracks: bool,
    /// Whether we've queued war factory.
    has_war_factory: bool,
    /// Whether we've queued refinery.
    has_refinery: bool,
    /// Whether we've queued radar dome.
    has_radar: bool,
    /// Whether we've queued defenses.
    defenses_built: u32,
    /// Number of units produced.
    units_produced: u32,
    /// Attack wave threshold.
    attack_threshold: u32,
    /// AI difficulty.
    difficulty: Difficulty,
    /// Tick interval between AI actions.
    tick_interval: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BotState {
    /// Deploy MCV first.
    DeployMcv,
    /// Building up base infrastructure.
    BuildUp,
    /// Producing army units.
    Producing,
    /// Sending units to attack.
    Attacking,
}

impl Bot {
    pub fn new(player_id: u32) -> Self {
        Self::new_with_difficulty(player_id, Difficulty::Easy)
    }

    pub fn new_with_difficulty(player_id: u32, difficulty: Difficulty) -> Self {
        let (tick_interval, attack_threshold) = match difficulty {
            Difficulty::Easy => (45, 4),
            Difficulty::Medium => (30, 3),
            Difficulty::Hard => (15, 3),
        };
        Bot {
            player_id,
            ticks_idle: 0,
            state: BotState::DeployMcv,
            mcv_deployed: false,
            has_power: false,
            has_barracks: false,
            has_war_factory: false,
            has_refinery: false,
            has_radar: false,
            defenses_built: 0,
            units_produced: 0,
            attack_threshold,
            difficulty,
            tick_interval,
        }
    }

    /// Generate orders for this tick.
    pub fn tick(&mut self, world: &World) -> Vec<GameOrder> {
        self.ticks_idle += 1;
        let mut orders = Vec::new();

        if self.ticks_idle < self.tick_interval {
            return orders;
        }
        self.ticks_idle = 0;

        // Always try to place ready buildings first
        self.try_place_buildings(world, &mut orders);

        match self.state {
            BotState::DeployMcv => {
                self.do_deploy_mcv(world, &mut orders);
            }
            BotState::BuildUp => {
                self.do_build_up(world, &mut orders);
            }
            BotState::Producing => {
                self.do_produce(world, &mut orders);
            }
            BotState::Attacking => {
                self.do_attack(world, &mut orders);
            }
        }

        orders
    }

    fn do_deploy_mcv(&mut self, world: &World, orders: &mut Vec<GameOrder>) {
        // Check if we already have a conyard
        if world.player_has_conyard(self.player_id) {
            self.mcv_deployed = true;
            self.state = BotState::BuildUp;
            return;
        }

        // Find our MCV and deploy it
        if let Some(mcv_id) = world.player_mcv(self.player_id) {
            orders.push(GameOrder {
                order_string: "DeployTransform".to_string(),
                subject_id: Some(mcv_id),
                target_string: None,
                extra_data: None,
            });
            self.mcv_deployed = true;
            // Wait a few ticks then transition to BuildUp
            self.state = BotState::BuildUp;
        }
    }

    fn try_place_buildings(&mut self, world: &World, orders: &mut Vec<GameOrder>) {
        // Check if there's a completed building waiting to be placed
        if let Some(building_type) = world.player_ready_building(self.player_id) {
            if let Some((x, y)) = world.find_placement_location(self.player_id, &building_type) {
                orders.push(GameOrder {
                    order_string: "PlaceBuilding".to_string(),
                    subject_id: Some(self.player_id),
                    target_string: Some(format!("{},{},{}", building_type, x, y)),
                    extra_data: None,
                });
            }
        }
    }

    fn do_build_up(&mut self, world: &World, orders: &mut Vec<GameOrder>) {
        let cash = self.player_cash(world);

        // Wait for conyard before building
        if !world.player_has_conyard(self.player_id) {
            if world.player_mcv(self.player_id).is_some() {
                self.state = BotState::DeployMcv;
            }
            return;
        }

        // Don't queue if there's already something in production or waiting to be placed
        if world.player_ready_building(self.player_id).is_some() {
            return;
        }
        if world.player_has_pending_production(self.player_id) {
            return;
        }

        // Check what buildings we actually have placed
        self.survey_buildings(world);

        // Build order: powr → tent → proc → weap → then produce units
        if !self.has_power && cash >= 300 {
            self.queue_building(orders, "powr");
            return;
        }

        if self.has_power && !self.has_barracks && cash >= 400 {
            self.queue_building(orders, "tent");
            return;
        }

        if self.has_power && !self.has_refinery && cash >= 1500 {
            self.queue_building(orders, "proc");
            return;
        }

        if self.has_refinery && !self.has_war_factory && cash >= 2000 {
            self.queue_building(orders, "weap");
            return;
        }

        // Once we have barracks, switch to producing units
        if self.has_power && self.has_barracks {
            self.state = BotState::Producing;
        }
    }

    fn queue_building(&self, orders: &mut Vec<GameOrder>, building_type: &str) {
        orders.push(GameOrder {
            order_string: "StartProduction".to_string(),
            subject_id: Some(self.player_id),
            target_string: Some(building_type.to_string()),
            extra_data: None,
        });
    }

    fn do_produce(&mut self, world: &World, orders: &mut Vec<GameOrder>) {
        let cash = self.player_cash(world);

        // Don't produce if something is already in production
        if world.player_has_pending_production(self.player_id) {
            return;
        }

        // Re-survey buildings in case they were destroyed
        self.survey_buildings(world);

        // Build war factory if we can afford it, have refinery prereq, and don't have one
        if !self.has_war_factory && self.has_refinery && cash >= 2000 {
            if world.player_ready_building(self.player_id).is_none() {
                self.queue_building(orders, "weap");
                return;
            }
        }

        // Produce units — prioritize cheap infantry to get attacking quickly
        let unit = if self.has_war_factory && cash >= 800 && self.units_produced % 3 == 0 {
            "2tnk"
        } else if self.has_barracks && cash >= 300 && self.units_produced % 2 == 1 {
            "e3"
        } else if self.has_barracks && cash >= 100 {
            "e1"
        } else {
            return;
        };

        orders.push(GameOrder {
            order_string: "StartProduction".to_string(),
            subject_id: Some(self.player_id),
            target_string: Some(unit.to_string()),
            extra_data: None,
        });
        self.units_produced += 1;

        // Medium/Hard: repair damaged buildings
        if self.difficulty != Difficulty::Easy {
            let damaged = world.player_damaged_buildings(self.player_id);
            for building_id in damaged {
                orders.push(GameOrder {
                    order_string: "RepairBuilding".to_string(),
                    subject_id: Some(building_id),
                    target_string: None,
                    extra_data: None,
                });
            }
        }

        // Once we have enough units, switch to attacking
        if self.units_produced >= self.attack_threshold {
            self.state = BotState::Attacking;
        }
    }

    fn do_attack(&mut self, world: &World, orders: &mut Vec<GameOrder>) {
        // Continue producing while attacking
        self.do_produce_units_only(world, orders);

        // Find enemy actors to attack
        let enemies = world.find_enemy_actors(self.player_id);
        if enemies.is_empty() {
            // No enemies found, go back to producing
            self.state = BotState::Producing;
            return;
        }

        // Get combat units (exclude harvesters)
        let our_units: Vec<u32> = world.actor_ids_for_player(self.player_id)
            .into_iter()
            .filter(|&id| {
                let is_combat = world.actor_kind(id).map(|k| {
                    matches!(k, ActorKind::Infantry | ActorKind::Vehicle)
                }).unwrap_or(false);
                let is_harv = world.actor_type_name(id).map(|t| t == "harv").unwrap_or(false);
                is_combat && !is_harv
            })
            .collect();

        // Move units toward enemy base
        if let Some(enemy_loc) = world.find_enemy_location(self.player_id) {
            for &unit_id in &our_units {
                // Check if unit is close to any enemy — if so, attack directly
                let unit_loc = world.actor_location(unit_id).unwrap_or((0, 0));
                let mut attacked = false;
                for &(enemy_id, enemy_x, enemy_y) in &enemies {
                    let dist = (unit_loc.0 - enemy_x).abs() + (unit_loc.1 - enemy_y).abs();
                    if dist <= 8 {
                        orders.push(GameOrder {
                            order_string: "Attack".to_string(),
                            subject_id: Some(unit_id),
                            target_string: None,
                            extra_data: Some(enemy_id),
                        });
                        attacked = true;
                        break;
                    }
                }
                if !attacked {
                    orders.push(GameOrder {
                        order_string: "Move".to_string(),
                        subject_id: Some(unit_id),
                        target_string: Some(format!("{},{}", enemy_loc.0, enemy_loc.1)),
                        extra_data: None,
                    });
                }
            }
        }

        // Check if our combat units are all idle (attack wave done)
        let all_idle = our_units.iter().all(|&id| {
            world.actor_activity(id).map(|a| a == "idle").unwrap_or(true)
        });
        if all_idle && !our_units.is_empty() {
            self.units_produced = 0;
            let wave_increase = match self.difficulty {
                Difficulty::Easy => 2,
                Difficulty::Medium => 2,
                Difficulty::Hard => 1,
            };
            self.attack_threshold += wave_increase;
            self.state = BotState::Producing;
        }
    }

    /// Produce units without state transitions (used during Attacking).
    fn do_produce_units_only(&mut self, world: &World, orders: &mut Vec<GameOrder>) {
        let cash = self.player_cash(world);
        if world.player_has_pending_production(self.player_id) {
            return;
        }
        self.survey_buildings(world);

        let unit = if self.has_war_factory && cash >= 800 && self.units_produced % 3 == 0 {
            "2tnk"
        } else if self.has_barracks && cash >= 300 && self.units_produced % 2 == 1 {
            "e3"
        } else if self.has_barracks && cash >= 100 {
            "e1"
        } else {
            return;
        };

        orders.push(GameOrder {
            order_string: "StartProduction".to_string(),
            subject_id: Some(self.player_id),
            target_string: Some(unit.to_string()),
            extra_data: None,
        });
        self.units_produced += 1;
    }

    fn player_cash(&self, world: &World) -> i32 {
        world.player_cash(self.player_id)
    }

    fn survey_buildings(&mut self, world: &World) {
        let building_types = world.player_building_types(self.player_id);
        self.has_power = building_types.iter().any(|t| t == "powr" || t == "apwr");
        self.has_barracks = building_types.iter().any(|t| t == "tent" || t == "barr");
        self.has_war_factory = building_types.iter().any(|t| t == "weap" || t == "weap.ukraine");
        self.has_refinery = building_types.iter().any(|t| t == "proc");
        self.has_radar = building_types.iter().any(|t| t == "dome");
    }

    fn find_enemy_target(&self, world: &World) -> Option<(i32, i32)> {
        world.find_enemy_location(self.player_id)
    }
}
