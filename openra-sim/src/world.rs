//! Game world state — actors, players, RNG.
//!
//! This module builds the world from map data and replay metadata,
//! then computes per-tick SyncHash to verify determinism against
//! the hashes recorded in .orarep files.

use crate::math::{CPos, WPos};
use crate::rng::MersenneTwister;
use crate::sync;

/// Bool hash values matching C# Sync.cs EmitSyncOpcodes:
///
/// Due to an IL generation quirk in OpenRA's Sync.cs, the Brtrue instruction
/// always pops the pushed 0xaaa constant (which is always truthy), leaving
/// the raw bool value (0 or 1) on the stack for XOR. So:
/// true → 1, false → 0 (NOT 0xaaa/0x555 as the code appears to intend).
fn hash_bool(b: bool) -> i32 {
    if b { 1 } else { 0 }
}

/// Compute the XOR hash of N bool fields.
fn xor_bools(values: &[bool]) -> i32 {
    let mut h = 0i32;
    for &v in values {
        h ^= hash_bool(v);
    }
    h
}

/// Lobby information extracted from the replay's SyncInfo orders.
/// This is needed to determine player count, starting cash, slots, etc.
#[derive(Debug, Clone)]
pub struct LobbyInfo {
    /// Starting cash for all players
    pub starting_cash: i32,
    /// Whether spectators are allowed (determines "Everyone" player creation)
    pub allow_spectators: bool,
    /// Slot definitions in order (e.g., ["Multi0", "Multi1"])
    /// Only slots with a client assigned are included.
    pub occupied_slots: Vec<SlotInfo>,
}

#[derive(Debug, Clone)]
pub struct SlotInfo {
    pub player_reference: String,
    pub faction: String,
}

impl Default for LobbyInfo {
    fn default() -> Self {
        LobbyInfo {
            starting_cash: 5000,
            allow_spectators: true,
            occupied_slots: Vec::new(),
        }
    }
}

/// The game world state.
pub struct World {
    /// All actors in creation order (ActorID order).
    /// Includes world actor (ID=0), player actors, and map actors.
    all_actor_ids: Vec<u32>,
    /// Actors with ISync traits, in ActorID order.
    sync_actors: Vec<sync::ActorSync>,
    /// Synced effects (projectiles etc.) — empty at tick 0.
    synced_effects: Vec<i32>,
    /// The shared RNG.
    pub rng: MersenneTwister,
    /// Player actor IDs with UnlockedRenderPlayer = true.
    unlocked_render_player_ids: Vec<u32>,
}

impl World {
    /// Compute World.SyncHash() matching the C# algorithm exactly.
    pub fn sync_hash(&self) -> i32 {
        sync::compute_world_sync_hash(
            &self.all_actor_ids,
            &self.sync_actors,
            &self.synced_effects,
            self.rng.last,
            &self.unlocked_render_player_ids,
        )
    }

    /// Compute SyncHash components separately for debugging.
    pub fn sync_hash_debug(&self) -> SyncHashDebug {
        let identity = sync::compute_world_sync_hash(
            &self.all_actor_ids, &[], &[], 0, &[],
        );
        let full_no_rng = sync::compute_world_sync_hash(
            &self.all_actor_ids, &self.sync_actors, &[], 0, &[],
        );
        let traits = full_no_rng.wrapping_sub(identity);
        let rng_last = self.rng.last;
        let full = self.sync_hash();
        SyncHashDebug { full, identity, traits, rng_last }
    }

    /// Dump per-actor, per-trait contributions for debugging.
    pub fn dump_sync_details(&self) {
        let mut n: i32 = 0;
        let mut ret: i32 = 0;

        // Identity hashes
        for &actor_id in &self.all_actor_ids {
            let contrib = n.wrapping_mul((1i32).wrapping_add(actor_id as i32))
                .wrapping_mul(sync::hash_actor(actor_id));
            eprintln!("IDENTITY n={} actor_id={} contrib={} running={}", n, actor_id, contrib, ret.wrapping_add(contrib));
            ret = ret.wrapping_add(contrib);
            n += 1;
        }
        eprintln!("AFTER_IDENTITY ret={} n={}", ret, n);

        // Trait hashes
        for actor_sync in &self.sync_actors {
            for (ti, &trait_hash) in actor_sync.trait_hashes.iter().enumerate() {
                let contrib = n.wrapping_mul((1i32).wrapping_add(actor_sync.actor_id as i32))
                    .wrapping_mul(trait_hash);
                eprintln!("TRAIT n={} actor_id={} trait_idx={} hash={} contrib={} running={}",
                    n, actor_sync.actor_id, ti, trait_hash, contrib, ret.wrapping_add(contrib));
                ret = ret.wrapping_add(contrib);
                n += 1;
            }
        }
        eprintln!("AFTER_TRAITS ret={} n={}", ret, n);
        eprintln!("RNG_LAST={}", self.rng.last);
        ret = ret.wrapping_add(self.rng.last);
        eprintln!("FINAL ret={}", ret);
    }
}

pub struct SyncHashDebug {
    pub full: i32,
    pub identity: i32,
    pub traits: i32,
    pub rng_last: i32,
}

/// Convert a cell position to world position (rectangular grid).
/// CenterOfCell(cell) = WPos(1024*x + 512, 1024*y + 512, 0)
pub fn center_of_cell(x: i32, y: i32) -> WPos {
    WPos::new(1024 * x + 512, 1024 * y + 512, 0)
}

// === Trait sync hash helpers ===

/// Building trait: [VerifySync] CPos TopLeft
fn building_sync_hash(top_left: CPos) -> i32 {
    sync::hash_cpos(top_left)
}

/// Immobile trait: [VerifySync] CPos TopLeft, [VerifySync] WPos CenterPosition
fn immobile_sync_hash(top_left: CPos, center_pos: WPos) -> i32 {
    sync::hash_cpos(top_left) ^ sync::hash_wpos(center_pos)
}

/// Health trait: [VerifySync] int HP
fn health_sync_hash(hp: i32) -> i32 {
    hp
}

// === Player trait sync hashes at tick 0 ===
//
// The Player actor ("Player" in player.yaml) has these ISync traits.
// Construction order is resolved by dependency topological sort.
//
// YAML order:
//   Shroud, PlayerResources, TechTree, ClassicPQ×6, PlaceBuilding,
//   SupportPowerManager, ScriptTriggers, MissionObjectives, ConquestVictoryConditions,
//   PowerManager, AllyRepair, PlayerResources(override), DeveloperMode, GpsWatcher,
//   Shroud(override), ..., FrozenActorLayer, ..., PlayerExperience, ...
//
// Dependencies:
//   ClassicProductionQueue requires TechTree + PlayerResources (both before it → OK)
//   PowerManager requires DeveloperMode (DeveloperMode is AFTER PowerManager in YAML)
//     → PowerManager must be constructed after DeveloperMode
//
// ISync traits only (in construction order):
//   1. Shroud           - 1 bool field: disabled=false
//   2. PlayerResources  - 3 int fields: Cash, Resources, ResourceCapacity (XOR'd)
//   3. ClassicPQ@Building  - 2 bool props: Enabled, IsValidFaction
//   4. ClassicPQ@Defense   - 2 bool props: Enabled, IsValidFaction
//   5. ClassicPQ@Vehicle   - 2 bool props: Enabled, IsValidFaction
//   6. ClassicPQ@Infantry  - 2 bool props: Enabled, IsValidFaction
//   7. ClassicPQ@Ship      - 2 bool props: Enabled, IsValidFaction
//   8. ClassicPQ@Aircraft  - 2 bool props: Enabled, IsValidFaction
//   9. MissionObjectives   - 1 int prop: ObjectivesHash=0
//  10. DeveloperMode       - 7 bool fields: all false at tick 0
//  11. PowerManager        - 2 int props: PowerProvided=0, PowerDrained=0
//  12. GpsWatcher          - 3 bool props + 1 bool field: all false
//  13. FrozenActorLayer    - 2 int fields: VisibilityHash=0, FrozenHash=0
//  14. PlayerExperience    - 1 int prop: Experience=0

/// Compute the trait hash for Shroud at tick 0.
/// [VerifySync] bool disabled = false
fn shroud_sync_hash() -> i32 {
    hash_bool(false)
}

/// Compute the trait hash for PlayerResources at tick 0.
/// [VerifySync] int Cash, [VerifySync] int Resources, [VerifySync] int ResourceCapacity
/// Fields are XOR'd. At tick 0: Cash=startingCash, Resources=0, ResourceCapacity=0
fn player_resources_sync_hash(starting_cash: i32) -> i32 {
    starting_cash ^ 0 ^ 0
}

/// Compute the trait hash for a single ClassicProductionQueue at tick 0.
/// [VerifySync] bool Enabled (property), [VerifySync] bool IsValidFaction (property)
///
/// The GenerateHashFunc iterates fields first, then properties.
/// Both Enabled and IsValidFaction are properties.
///
/// At tick 0 for a playable player with a non-locked "Random" faction:
///   IsValidFaction is set based on whether the queue's configured Factions
///   match the player's resolved faction. For RA, production queues have no
///   Factions restriction by default, so IsValidFaction = true.
///   Enabled starts as IsValidFaction value.
///
/// For non-playable players (Neutral, Creeps): they also get the Player actor
/// and production queues, but IsValidFaction depends on faction matching.
fn production_queue_sync_hash(enabled: bool, is_valid_faction: bool) -> i32 {
    // Properties only, no fields with [VerifySync]
    hash_bool(enabled) ^ hash_bool(is_valid_faction)
}

/// Compute the trait hash for MissionObjectives at tick 0.
/// [VerifySync] int ObjectivesHash = 0 (property)
fn mission_objectives_sync_hash() -> i32 {
    0
}

/// Compute the trait hash for DeveloperMode at tick 0.
/// 7 [VerifySync] bool fields, all false at tick 0.
/// XOR of 7 copies of 0x555 = 0x555 (odd count)
fn developer_mode_sync_hash() -> i32 {
    xor_bools(&[false; 7])
}

/// Compute the trait hash for PowerManager at tick 0.
/// [VerifySync] int PowerProvided = 0, [VerifySync] int PowerDrained = 0
fn power_manager_sync_hash() -> i32 {
    0 ^ 0
}

/// Compute the trait hash for GpsWatcher at tick 0.
/// Fields: [VerifySync] bool explored = false
/// Properties: [VerifySync] bool Launched = false, GrantedAllies = false, Granted = false
/// Fields first, then properties, all XOR'd.
fn gps_watcher_sync_hash() -> i32 {
    // 1 field + 3 properties = 4 bools, all false
    xor_bools(&[false; 4])
}

/// Compute the trait hash for FrozenActorLayer at tick 0.
/// [VerifySync] int VisibilityHash = 0, [VerifySync] int FrozenHash = 0
fn frozen_actor_layer_sync_hash() -> i32 {
    0 ^ 0
}

/// Compute the trait hash for PlayerExperience at tick 0.
/// [VerifySync] int Experience = 0
fn player_experience_sync_hash() -> i32 {
    0
}

/// Compute the trait hash for Mobile at tick 0.
/// [VerifySync] WAngle Facing, CPos FromCell, CPos ToCell, WPos CenterPosition
/// All properties (no fields). At tick 0: FromCell == ToCell (stationary).
/// Facing is randomized via SharedRandom.Next(1024) during SpawnStartingUnits.
fn mobile_sync_hash(cell_x: i32, cell_y: i32, facing: i32) -> i32 {
    let center = center_of_cell(cell_x, cell_y);
    // hash = Facing XOR FromCell.Bits XOR ToCell.Bits XOR WPos.hash
    // FromCell == ToCell, so they cancel: hash = Facing XOR WPos.hash
    facing ^ center.sync_hash()
}

/// Build the ISync trait hashes for an MCV actor at tick 0.
///
/// MCV traits (in construction order):
/// 1. ClassicFacingBodyOrientation: QuantizedFacings = 32
/// 2. Mobile: position-dependent, facing from RNG
/// 3. Chronoshiftable: CPos Origin=Zero, int ReturnTicks=0 → hash=0
/// 4. Health: HP = 60000
/// 5. RevealsShroud (AffectsShroud): cachedLocation=Zero, cachedRange=Zero,
///    CachedTraitDisabled=false → hash=0
fn mcv_trait_hashes(spawn_x: i32, spawn_y: i32, facing: i32) -> Vec<i32> {
    // RevealsShroud hash = 0: private [VerifySync] fields on AffectsShroud base class
    // are NOT visible to reflection on RevealsShroud derived type.
    vec![
        32,                                          // ClassicFacingBodyOrientation: QuantizedFacings
        mobile_sync_hash(spawn_x, spawn_y, facing),  // Mobile
        0,                                           // Chronoshiftable
        60000,                                       // Health: HP
        0,                                           // RevealsShroud (base class fields invisible)
    ]
}

/// Assign spawn points to playable players using the playerRandom sequence.
///
/// This replicates the C# CreateMapPlayers + MapStartingLocations logic:
/// 1. Non-playable players: ResolveFaction (no RNG if faction has no RandomFactionMembers)
/// 2. Playable players: ResolveFaction (consumes RNG) + AssignHomeLocation
/// 3. "Everyone": ResolveFaction (consumes RNG)
///
/// Returns spawn locations for each playable player in lobby slot order.
fn assign_spawn_points(
    spawn_locations: &[(i32, i32)],
    num_playable: usize,
    seed: i32,
    map_players: &[openra_data::oramap::PlayerDef],
) -> Vec<(i32, i32)> {
    let mut player_rng = MersenneTwister::new(seed);

    // Non-playable players: ResolveFaction for each
    // In RA mod, non-playable players have faction "allies" which has no
    // RandomFactionMembers → no RNG consumption.
    // (allies is defined with Selectable: False and no RandomFactionMembers)
    for p in map_players {
        if !p.playable {
            // ResolveFaction("allies", ..., requireSelectable=false)
            // "allies" has no RandomFactionMembers → no RNG call
            // (nothing to do)
        }
    }

    // Playable players: ResolveFaction + AssignHomeLocation
    // Each "Random" faction resolution: Random→{RandomAllies|RandomSoviet}→{specific}
    // = 2 RNG calls per player for faction
    // First player: 1 RNG call for spawn point (random from available)
    // Subsequent: 0 calls (separateTeamSpawns uses MaxBy = deterministic)
    let mut available_spawns: Vec<usize> = (0..spawn_locations.len()).collect();
    let mut assignments = Vec::new();

    for i in 0..num_playable {
        // ResolveFaction("Random"): 2 playerRandom calls
        // Call 1: Random.RandomFactionMembers = [RandomAllies, RandomSoviet] → Next(2)
        let meta_faction = player_rng.next_range(0, 2);
        eprintln!("playerRNG[{}]: meta_faction={} (0=RandomAllies, 1=RandomSoviet) rng.last={} total={}",
            i, meta_faction, player_rng.last, player_rng.total_count);

        // Call 2: pick specific faction from meta-faction
        // RandomAllies: [england, france, germany] → Next(3)
        // RandomSoviet: [russia, ukraine] → Next(2)
        if meta_faction == 0 {
            let specific = player_rng.next_range(0, 3);
            eprintln!("playerRNG[{}]: specific_allies={} rng.last={} total={}",
                i, specific, player_rng.last, player_rng.total_count);
        } else {
            let specific = player_rng.next_range(0, 2);
            eprintln!("playerRNG[{}]: specific_soviet={} rng.last={} total={}",
                i, specific, player_rng.last, player_rng.total_count);
        }

        // AssignHomeLocation
        if i == 0 {
            // First player: random from available spawn points
            let idx = player_rng.next_range(0, available_spawns.len() as i32) as usize;
            eprintln!("playerRNG[{}]: spawn_idx={} from {} available, rng.last={} total={}",
                i, idx, available_spawns.len(), player_rng.last, player_rng.total_count);
            let spawn_idx = available_spawns.remove(idx);
            assignments.push(spawn_locations[spawn_idx]);
        } else {
            // Subsequent: separateTeamSpawns=true → pick most distant (deterministic)
            let spawn_idx = available_spawns.remove(0);
            assignments.push(spawn_locations[spawn_idx]);
        }
    }

    // "Everyone" player: ResolveFaction("Random", ..., requireSelectable=false)
    // With requireSelectable=false, selectableFactions includes ALL factions.
    // "Random" is found → RandomFactionMembers = [RandomAllies, RandomSoviet] → 2 calls
    // (We don't need to actually compute this since it doesn't affect spawn assignments)

    assignments
}

/// Build the ISync trait hashes for a player actor at tick 0.
///
/// `is_playable` determines production queue Enabled/IsValidFaction defaults.
fn player_trait_hashes(starting_cash: i32, is_playable: bool) -> Vec<i32> {
    // Production queues: for playable players, Enabled=true, IsValidFaction=true
    // For non-playable (Neutral, Creeps), they don't have valid factions for
    // production, but the trait still exists. IsValidFaction depends on faction config.
    // In practice, non-playable players have faction "allies"/"Random" and
    // the production queues have no faction filter → IsValidFaction = true, Enabled = true.
    let pq_enabled = true;
    let pq_valid_faction = true;

    let mut hashes = Vec::new();

    // ISync traits in TraitsInConstructOrder() — dependency-resolved order:
    //
    // Pass 1 (no dependencies): traits in YAML order
    //   Shroud, PlayerResources, MissionObjectives, DeveloperMode,
    //   GpsWatcher, FrozenActorLayer, PlayerExperience
    //
    // Pass 2 (dependencies now met):
    //   ClassicProductionQueue×6 (requires TechTree + PlayerResources)
    //
    // Pass 3:
    //   PowerManager (requires DeveloperMode)

    // 1. Shroud
    hashes.push(shroud_sync_hash());

    // 2. PlayerResources
    hashes.push(player_resources_sync_hash(starting_cash));

    // 3. MissionObjectives
    hashes.push(mission_objectives_sync_hash());

    // 4. DeveloperMode
    hashes.push(developer_mode_sync_hash());

    // 5. GpsWatcher
    hashes.push(gps_watcher_sync_hash());

    // 6. FrozenActorLayer
    hashes.push(frozen_actor_layer_sync_hash());

    // 7. PlayerExperience
    hashes.push(player_experience_sync_hash());

    // 8-13. ClassicProductionQueue × 6 (resolved in pass 2)
    for _ in 0..6 {
        hashes.push(production_queue_sync_hash(pq_enabled, pq_valid_faction));
    }

    // 14. PowerManager (resolved in pass 3)
    hashes.push(power_manager_sync_hash());

    let _ = is_playable; // May be used for future differentiation

    hashes
}

/// Build a World from parsed map data, game seed, and lobby info.
///
/// This is the initial world state (tick 0). Actor IDs are assigned:
/// 0 = World actor, 1-N = player actors, N+1.. = map actors.
pub fn build_world(
    map: &openra_data::oramap::OraMap,
    random_seed: i32,
    lobby: &LobbyInfo,
) -> World {
    let mut rng = MersenneTwister::new(random_seed);
    let mut all_actor_ids: Vec<u32> = Vec::new();
    let mut sync_actors: Vec<sync::ActorSync> = Vec::new();
    let mut next_id: u32 = 0;

    // === Actor ID 0: World actor ===
    // ISync traits: DebugPauseState (1 bool: IsWorldPaused = false at tick 0)
    all_actor_ids.push(next_id);
    sync_actors.push(sync::ActorSync {
        actor_id: next_id,
        trait_hashes: vec![hash_bool(true)], // DebugPauseState: IsWorldPaused=true at tick 0
    });
    next_id += 1;

    // === Player actors ===
    // Order: non-playable first (in map YAML order), then playable (slot order),
    // then "Everyone" spectator.

    // Non-playable map players (Neutral, Creeps, etc.)
    let non_playable: Vec<_> = map.players.iter()
        .filter(|p| !p.playable)
        .collect();

    for _p in &non_playable {
        let id = next_id;
        all_actor_ids.push(id);
        sync_actors.push(sync::ActorSync {
            actor_id: id,
            trait_hashes: player_trait_hashes(lobby.starting_cash, false),
        });
        next_id += 1;
    }

    // Playable players (one per occupied lobby slot)
    for _slot in &lobby.occupied_slots {
        let id = next_id;
        all_actor_ids.push(id);
        sync_actors.push(sync::ActorSync {
            actor_id: id,
            trait_hashes: player_trait_hashes(lobby.starting_cash, true),
        });
        next_id += 1;
    }

    // "Everyone" spectator player — always created by CreateMapPlayers
    // after non-playable and playable players.
    {
        let id = next_id;
        all_actor_ids.push(id);
        sync_actors.push(sync::ActorSync {
            actor_id: id,
            trait_hashes: player_trait_hashes(lobby.starting_cash, false),
        });
        next_id += 1;
    }

    // === Map actors ===
    // Collect spawn point locations for MCV placement.
    let mut spawn_locations: Vec<(i32, i32)> = Vec::new();

    for actor in &map.actors {
        let id = next_id;
        all_actor_ids.push(id);
        next_id += 1;

        let mut trait_hashes = Vec::new();

        let is_tree = actor.actor_type.starts_with('t')
            && (actor.actor_type.len() == 3 || actor.actor_type.starts_with("tc"));
        let is_mine = actor.actor_type == "mine";
        let is_spawn = actor.actor_type == "mpspawn";

        let top_left = CPos::new(actor.location.0, actor.location.1);

        if is_tree {
            trait_hashes.push(1); // BodyOrientation: QuantizedFacings
            trait_hashes.push(building_sync_hash(top_left));
            trait_hashes.push(health_sync_hash(50000));
        } else if is_mine {
            trait_hashes.push(1); // BodyOrientation: QuantizedFacings
            trait_hashes.push(building_sync_hash(top_left));
        } else if is_spawn {
            spawn_locations.push(actor.location);
            // Construction order: Immobile before BodyOrientation
            let center = center_of_cell(actor.location.0, actor.location.1);
            trait_hashes.push(immobile_sync_hash(top_left, center));
            trait_hashes.push(1); // BodyOrientation: QuantizedFacings
        }

        if !trait_hashes.is_empty() {
            sync_actors.push(sync::ActorSync {
                actor_id: id,
                trait_hashes,
            });
        }
    }

    // === Starting units (MCVs) ===
    // "startingunits=none" in OpenRA actually means the "mcv-only" class,
    // which spawns one MCV per playable player at their assigned spawn point.
    // MCVs are created after all map actors.
    //
    // Spawn point assignment uses a separate playerRandom (same seed as SharedRandom).
    // The playerRandom sequence during CreateMapPlayers:
    //   - Non-playable players: ResolveFaction for each (allies faction has no
    //     RandomFactionMembers, so no RNG consumption)
    //   - Playable players: ResolveFaction (2 calls each for Random→RandomAllies/
    //     RandomSoviet→specific) + AssignHomeLocation (1 call for first player,
    //     0 for second since separateTeamSpawns uses MaxBy)
    //   - "Everyone": ResolveFaction (2 calls)
    //
    // MCV facing is randomized via SharedRandom.Next(1024) per player
    // in SpawnStartingUnits.WorldLoaded().
    let player_spawn_assignments = assign_spawn_points(
        &spawn_locations,
        lobby.occupied_slots.len(),
        random_seed,
        &map.players,
    );

    // MCV facing: BaseActorFacing defaults to WAngle(512) in StartingUnitsInfo,
    // and the @mcvonly entry doesn't override it. So facing=512, NOT random.
    // SharedRandom is NOT consumed for MCV facing.
    let facing = 512;
    for (pi, &(spawn_x, spawn_y)) in player_spawn_assignments.iter().enumerate() {
        eprintln!("MCV[{}] spawn=({},{}) facing={}", pi, spawn_x, spawn_y, facing);
        let id = next_id;
        all_actor_ids.push(id);
        sync_actors.push(sync::ActorSync {
            actor_id: id,
            trait_hashes: mcv_trait_hashes(spawn_x, spawn_y, facing),
        });
        next_id += 1;
    }

    World {
        all_actor_ids,
        sync_actors,
        synced_effects: Vec::new(),
        rng,
        unlocked_render_player_ids: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn center_of_cell_values() {
        let pos = center_of_cell(0, 0);
        assert_eq!(pos, WPos::new(512, 512, 0));

        let pos = center_of_cell(10, 20);
        assert_eq!(pos, WPos::new(10 * 1024 + 512, 20 * 1024 + 512, 0));
    }

    #[test]
    fn building_hash_matches_cpos_bits() {
        let top_left = CPos::new(5, 10);
        assert_eq!(building_sync_hash(top_left), top_left.bits);
    }

    #[test]
    fn immobile_hash_xors_topleft_and_center() {
        let top_left = CPos::new(5, 10);
        let center = center_of_cell(5, 10);
        let hash = immobile_sync_hash(top_left, center);
        assert_eq!(hash, top_left.bits ^ center.sync_hash());
    }

    #[test]
    fn bool_hash_values() {
        // Due to IL quirk in C# Sync.cs: false→0, true→1
        assert_eq!(hash_bool(true), 1);
        assert_eq!(hash_bool(false), 0);
    }

    #[test]
    fn developer_mode_7_false_bools() {
        // 7 false bools XOR'd: all 0 → 0
        assert_eq!(developer_mode_sync_hash(), 0);
    }

    #[test]
    fn gps_watcher_4_false_bools() {
        // 4 false bools XOR'd: all 0 → 0
        assert_eq!(gps_watcher_sync_hash(), 0);
    }

    #[test]
    fn production_queue_both_true() {
        // true XOR true = 1 ^ 1 = 0
        assert_eq!(production_queue_sync_hash(true, true), 0);
    }

    #[test]
    fn shroud_false_disabled() {
        assert_eq!(shroud_sync_hash(), 0);
    }

    #[test]
    fn player_resources_5000_cash() {
        assert_eq!(player_resources_sync_hash(5000), 5000);
    }
}
