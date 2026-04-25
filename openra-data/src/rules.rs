//! Game rules loader — parses OpenRA's YAML rules into structured data.
//!
//! Loads actor definitions, weapon definitions, and other game rules from
//! the mod's rules/*.yaml files using the MiniYAML parser.
//!
//! Reference: OpenRA.Game/GameRules/Ruleset.cs, ActorInfo.cs

use crate::miniyaml::{self, MiniYamlNode};
use std::collections::BTreeMap;
use std::path::Path;

/// 1D world distance in OpenRA fixed-point units (1024 units = 1 cell).
///
/// This mirrors `openra_sim::math::WDist` exactly (same `i32` representation).
/// We define a local copy here to keep `openra-data` free of a `openra-sim`
/// dependency (the dep arrow already runs sim → data). The simulator's loader
/// constructs proper `WDist` values from the raw `i32` field, no scaling
/// required.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct WDist {
    pub length: i32,
}

impl WDist {
    pub const ZERO: WDist = WDist { length: 0 };

    pub const fn new(length: i32) -> Self {
        WDist { length }
    }

    pub const fn from_cells(cells: i32) -> Self {
        WDist { length: 1024 * cells }
    }
}

/// Parse an OpenRA range/distance literal.
///
/// Accepts:
/// - `"5c0"`     → 5 cells + 0 → WDist(5120)
/// - `"5c512"`   → 5 cells + 512 → WDist(5632)
/// - `"2c512"`   → 2 cells + 512
/// - `"1024"`    → raw fixed-point units (no `c`)
///
/// Reference: `OpenRA.Game/WDist.cs::TryParse`.
pub fn parse_wdist(s: &str) -> Option<WDist> {
    let s = s.trim();
    if let Some(idx) = s.find('c') {
        let cells: i32 = s[..idx].trim().parse().ok()?;
        let sub: i32 = s[idx + 1..].trim().parse().ok()?;
        // OpenRA convention: negative cells negate the sub-component too.
        let sign = if cells < 0 { -1 } else { 1 };
        Some(WDist::new(cells * 1024 + sign * sub))
    } else {
        Some(WDist::new(s.parse().ok()?))
    }
}

/// A trait definition on an actor (e.g., Health, Mobile, Armament@PRIMARY).
#[derive(Debug, Clone)]
pub struct TraitInfo {
    /// Trait type name (e.g., "Health", "Armament", "Mobile").
    pub trait_name: String,
    /// Instance name for named traits (e.g., "PRIMARY" for Armament@PRIMARY).
    pub instance_name: Option<String>,
    /// Key-value parameters (e.g., "HP" -> "150000").
    pub params: BTreeMap<String, String>,
    /// Nested child nodes (for complex params like warhead definitions).
    pub children: Vec<MiniYamlNode>,
}

impl TraitInfo {
    /// Get a parameter value by key.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.params.get(key).map(|s| s.as_str())
    }

    /// Get a parameter as i32.
    pub fn get_i32(&self, key: &str) -> Option<i32> {
        self.get(key).and_then(|v| v.parse().ok())
    }

    /// Get a parameter as bool.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get(key).map(|v| v.eq_ignore_ascii_case("true") || v == "True")
    }
}

/// An actor definition (e.g., E1, FACT, MCV).
#[derive(Debug, Clone)]
pub struct ActorInfo {
    pub name: String,
    pub traits: Vec<TraitInfo>,
}

impl ActorInfo {
    /// Find a trait by type name (first match).
    pub fn trait_info(&self, name: &str) -> Option<&TraitInfo> {
        self.traits.iter().find(|t| t.trait_name == name)
    }

    /// Find a trait by type name and instance (e.g., "Armament", "PRIMARY").
    pub fn trait_instance(&self, name: &str, instance: &str) -> Option<&TraitInfo> {
        self.traits.iter().find(|t| {
            t.trait_name == name && t.instance_name.as_deref() == Some(instance)
        })
    }

    /// Get all traits of a given type.
    pub fn traits_of(&self, name: &str) -> Vec<&TraitInfo> {
        self.traits.iter().filter(|t| t.trait_name == name).collect()
    }

    /// Check if this actor has a specific trait.
    pub fn has_trait(&self, name: &str) -> bool {
        self.traits.iter().any(|t| t.trait_name == name)
    }
}

/// A weapon definition.
#[derive(Debug, Clone)]
pub struct WeaponInfo {
    pub name: String,
    pub params: BTreeMap<String, String>,
    pub children: Vec<MiniYamlNode>,
}

impl WeaponInfo {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.params.get(key).map(|s| s.as_str())
    }

    pub fn get_i32(&self, key: &str) -> Option<i32> {
        self.get(key).and_then(|v| v.parse().ok())
    }
}

/// The complete game ruleset.
#[derive(Debug, Clone)]
pub struct Ruleset {
    pub actors: BTreeMap<String, ActorInfo>,
    pub weapons: BTreeMap<String, WeaponInfo>,
}

impl Ruleset {
    /// Get an actor definition by name.
    pub fn actor(&self, name: &str) -> Option<&ActorInfo> {
        self.actors.get(name)
    }

    /// Get a weapon definition by name.
    pub fn weapon(&self, name: &str) -> Option<&WeaponInfo> {
        self.weapons.get(name)
    }
}

/// Convert a resolved MiniYamlNode into an ActorInfo.
fn node_to_actor(node: &MiniYamlNode) -> ActorInfo {
    let mut traits = Vec::new();

    for child in &node.children {
        let (trait_name, instance_name) = if let Some(at_pos) = child.key.find('@') {
            (
                child.key[..at_pos].to_string(),
                Some(child.key[at_pos + 1..].to_string()),
            )
        } else {
            (child.key.clone(), None)
        };

        let mut params = BTreeMap::new();
        for grandchild in &child.children {
            if grandchild.children.is_empty() {
                params.insert(grandchild.key.clone(), grandchild.value.clone());
            }
        }

        traits.push(TraitInfo {
            trait_name,
            instance_name,
            params,
            children: child.children.clone(),
        });
    }

    ActorInfo {
        name: node.key.clone(),
        traits,
    }
}

/// Convert a resolved MiniYamlNode into a WeaponInfo.
fn node_to_weapon(node: &MiniYamlNode) -> WeaponInfo {
    let mut params = BTreeMap::new();
    for child in &node.children {
        if child.children.is_empty() {
            params.insert(child.key.clone(), child.value.clone());
        }
    }

    WeaponInfo {
        name: node.key.clone(),
        params,
        children: node.children.clone(),
    }
}

/// Build a Ruleset from pre-loaded YAML source strings.
/// Works in WASM (no filesystem access needed).
pub fn load_ruleset_from_strings(rule_sources: &[&str], weapon_sources: &[&str]) -> Ruleset {
    let merged = miniyaml::parse_and_merge(rule_sources);
    let resolved = miniyaml::resolve_inherits(merged);

    let mut actors = BTreeMap::new();
    for node in &resolved {
        actors.insert(node.key.clone(), node_to_actor(node));
    }

    let mut weapons = BTreeMap::new();
    if !weapon_sources.is_empty() {
        let weapon_merged = miniyaml::parse_and_merge(weapon_sources);
        let weapon_resolved = miniyaml::resolve_inherits(weapon_merged);
        for node in &weapon_resolved {
            weapons.insert(node.key.clone(), node_to_weapon(node));
        }
    }

    Ruleset { actors, weapons }
}

/// Load a ruleset from a mod directory (e.g., "/path/to/OpenRA/mods/ra").
///
/// Reads rules/*.yaml and weapons/*.yaml, merges them with defaults,
/// resolves inheritance, and returns a complete Ruleset.
pub fn load_ruleset(mod_dir: &Path) -> std::io::Result<Ruleset> {
    let rules_dir = mod_dir.join("rules");
    let weapons_dir = mod_dir.join("weapons");

    let rule_files = &[
        "defaults.yaml",
        "player.yaml",
        "world.yaml",
        "infantry.yaml",
        "vehicles.yaml",
        "aircraft.yaml",
        "ships.yaml",
        "structures.yaml",
        "decoration.yaml",
        "misc.yaml",
        "civilian.yaml",
        "fakes.yaml",
        "husks.yaml",
    ];

    let mut rule_sources = Vec::new();
    for filename in rule_files {
        let path = rules_dir.join(filename);
        if path.exists() {
            rule_sources.push(std::fs::read_to_string(&path)?);
        }
    }

    let source_refs: Vec<&str> = rule_sources.iter().map(|s| s.as_str()).collect();

    // Load weapons
    let mut weapon_sources_owned = Vec::new();
    if weapons_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&weapons_dir) {
            let mut filenames: Vec<_> = entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().map_or(false, |ext| ext == "yaml"))
                .map(|e| e.path())
                .collect();
            filenames.sort();
            for path in &filenames {
                weapon_sources_owned.push(std::fs::read_to_string(path)?);
            }
        }
    }
    let weapon_refs: Vec<&str> = weapon_sources_owned.iter().map(|s| s.as_str()).collect();

    Ok(load_ruleset_from_strings(&source_refs, &weapon_refs))
}

// ---------------------------------------------------------------------------
// Typed convenience layer (sim-facing API).
// ---------------------------------------------------------------------------

/// Per-unit stats consumed by the simulator.
///
/// Values are pulled from the resolved (inheritance-applied) `ActorInfo`. All
/// distance fields are stored as fixed-point `WDist` (1024 units per cell) so
/// the sim does not need to scale on load.
#[derive(Debug, Clone)]
pub struct UnitInfo {
    /// Actor type name (e.g. `"E1"`).
    pub name: String,
    /// `Health.HP`. C# uses raw integer HP (not WDist). For e1 this is `5000`.
    pub hp: i32,
    /// `Mobile.Speed` in OpenRA tick-units. For e1 (inherits ^Infantry) this
    /// is `54`. Higher = faster. Returns `None` if the actor is immobile.
    pub speed: Option<i32>,
    /// `RevealsShroud.Range`. For e1 this is `4c0` = WDist(4096).
    pub reveal_range: Option<WDist>,
    /// Primary weapon name, e.g. `"M1Carbine"`. Pulled from
    /// `Armament.Weapon` or `Armament@PRIMARY.Weapon` (first match wins).
    pub primary_weapon: Option<String>,
    /// Whether this actor has the `MustBeDestroyed` trait (counts toward the
    /// kill-all win condition).
    pub must_be_destroyed: bool,
    /// `Armor.Type` string lower-cased (`"none"`, `"light"`, `"heavy"`,
    /// `"wood"`, `"concrete"`). Used by `Versus` damage multipliers.
    /// Defaults to `"none"` when the actor has no `Armor` trait.
    pub armor_class: String,
    /// `Mobile.Locomotor` lower-cased (`"foot"`, `"wheeled"`, `"tracked"`,
    /// `"heavytracked"`). Empty when the unit has no Mobile trait.
    pub locomotor: String,
    /// `Turreted.TurnSpeed` (WAngle units / tick). `None` if the actor
    /// has no turret. Phase 8: surfaced for env loader's typed-component
    /// attachment.
    pub turret_turn_speed: Option<i32>,
}

/// Per-weapon stats consumed by the simulator's combat loop.
///
/// Phase 8 extends Phase 6's view to include projectile-flight metadata
/// and per-armor-class damage multipliers (`Versus`).
#[derive(Debug, Clone, Default)]
pub struct WeaponStats {
    /// Weapon name (e.g. `"M1Carbine"`).
    pub name: String,
    /// `Range`. For M1Carbine this is `5c0` = WDist(5120).
    pub range: WDist,
    /// `ReloadDelay` in ticks (cooldown between shots). For M1Carbine = 20.
    pub reload_delay: i32,
    /// Damage from `Warhead@*: SpreadDamage` (or `TargetDamage`) -> `Damage`.
    /// Inherited from `^LightMG` for M1Carbine = 1000.
    pub damage: i32,
    /// `Projectile.Speed` (world units / tick). Zero for weapons with
    /// `Projectile: InstantHit` (M1Carbine, DogJaw, TurretGun, TeslaZap)
    /// — the world's combat tick still applies damage instantly for
    /// these. Non-zero for `Projectile: Missile` / `Projectile: Bullet`
    /// (RedEye, Dragon, Hellfire, Maverick, Stinger), which spawn a
    /// `Projectile` entity that flies to the target over multiple ticks.
    pub projectile_speed: WDist,
    /// `Warhead@*: SpreadDamage -> Spread`. Radius in world units across
    /// which damage is applied at impact. For non-splash weapons this is
    /// `0` (single-target hit). For RedEye/Dragon (`Spread: 128`) this
    /// becomes 128 units (~1/8 cell) — small but non-zero.
    pub splash_radius: WDist,
    /// `Warhead@*: SpreadDamage -> Versus: <Class>: <pct>`. Per-armor-class
    /// damage multiplier in percent. Missing classes default to 100% (no
    /// modifier). Keys are lower-cased armor-class names (`"heavy"`,
    /// `"light"`, `"wood"`, `"concrete"`, `"none"`).
    pub versus: BTreeMap<String, i32>,
}

/// Phase-7 building info — typed view onto the subset of `ActorInfo`
/// that the simulator needs for static structures (footprint, primary
/// weapon, MustBeDestroyed flag).
///
/// Buildings are also reachable via `UnitInfo::must_be_destroyed`, but
/// `BuildingInfo` carries the additional fields the static-defense
/// path needs (footprint, weapon name) without forcing every `unit`
/// caller to learn about building-only fields.
#[derive(Debug, Clone)]
pub struct BuildingInfo {
    /// Actor type (e.g. `"GUN"`, `"PBOX"`).
    pub name: String,
    /// `Health.HP`. Same units as `UnitInfo::hp`.
    pub hp: i32,
    /// Footprint (width, height) parsed from `Building.Dimensions:`.
    /// Defaults to `(2, 2)` when unspecified.
    pub footprint: (i32, i32),
    /// Primary armament weapon name (e.g. `"TurretGun"` for `gun`,
    /// `"TeslaZap"` for `tsla`). `None` for cosmetic buildings.
    pub primary_weapon: Option<String>,
    /// Whether this building counts toward the kill-all win condition.
    pub must_be_destroyed: bool,
}

/// Build a `BuildingInfo` from a resolved `ActorInfo`.
///
/// Returns `None` if the actor lacks `Building` (i.e. is not a building).
pub fn building_info_from_actor(actor: &ActorInfo) -> Option<BuildingInfo> {
    if !actor.has_trait("Building") {
        return None;
    }
    let hp = actor
        .trait_info("Health")
        .and_then(|t| t.get_i32("HP"))
        .unwrap_or(0);
    // Footprint from Building.Dimensions: "W,H" — fall back to 2,2.
    let footprint = actor
        .trait_info("Building")
        .and_then(|b| b.get("Dimensions"))
        .and_then(|s| {
            let mut parts = s.split(',');
            let w: i32 = parts.next()?.trim().parse().ok()?;
            let h: i32 = parts.next()?.trim().parse().ok()?;
            Some((w, h))
        })
        .unwrap_or((2, 2));

    let primary_weapon = actor
        .trait_instance("Armament", "PRIMARY")
        .or_else(|| actor.trait_info("Armament"))
        .and_then(|t| t.get("Weapon"))
        .map(|s| s.to_string());

    let must_be_destroyed = actor.has_trait("MustBeDestroyed");

    Some(BuildingInfo {
        name: actor.name.clone(),
        hp,
        footprint,
        primary_weapon,
        must_be_destroyed,
    })
}

/// Sim-facing typed view over a ruleset. Built from a `Ruleset` via
/// `Rules::from_ruleset`. Lookups are by uppercase actor/weapon name.
#[derive(Debug, Clone)]
pub struct Rules {
    pub units: BTreeMap<String, UnitInfo>,
    pub weapons: BTreeMap<String, WeaponStats>,
    /// Phase-7 — typed view of static buildings keyed by their uppercase
    /// actor name (`"GUN"`, `"PBOX"`, `"TSLA"`, `"FACT"`, …).
    pub buildings: BTreeMap<String, BuildingInfo>,
}

impl Rules {
    /// Build the typed `Rules` view from a parsed `Ruleset`.
    ///
    /// Failures during typed extraction are logged via `eprintln!` and the
    /// affected unit/weapon is skipped (we do not want to abort the whole
    /// load on a single missing field). All distance values pass through
    /// `parse_wdist`.
    pub fn from_ruleset(ruleset: &Ruleset) -> Self {
        let mut units = BTreeMap::new();
        let mut buildings = BTreeMap::new();
        for (name, actor) in &ruleset.actors {
            if let Some(unit) = unit_info_from_actor(actor) {
                units.insert(name.clone(), unit);
            }
            if let Some(b) = building_info_from_actor(actor) {
                buildings.insert(name.clone(), b);
            }
        }
        let mut weapons = BTreeMap::new();
        for (name, weapon) in &ruleset.weapons {
            if let Some(stats) = weapon_stats_from_weapon(weapon) {
                weapons.insert(name.clone(), stats);
            }
        }
        Rules { units, weapons, buildings }
    }

    pub fn unit(&self, name: &str) -> Option<&UnitInfo> {
        self.units.get(name)
    }

    pub fn weapon(&self, name: &str) -> Option<&WeaponStats> {
        self.weapons.get(name)
    }

    /// Phase-7 — look up a building by uppercase actor name.
    pub fn building(&self, name: &str) -> Option<&BuildingInfo> {
        self.buildings.get(name)
    }

    /// Convenience: load a Rules from a mod directory (e.g.
    /// `"vendor/OpenRA/mods/ra"`).
    pub fn load(mod_dir: &Path) -> std::io::Result<Self> {
        let ruleset = load_ruleset(mod_dir)?;
        Ok(Self::from_ruleset(&ruleset))
    }
}

/// Build a `UnitInfo` from a resolved `ActorInfo`. Returns `None` if the
/// actor lacks a `Health` block (e.g. for abstract `^...` parents that
/// somehow leaked through, or for purely-decorative actors).
fn unit_info_from_actor(actor: &ActorInfo) -> Option<UnitInfo> {
    let health = actor.trait_info("Health")?;
    let hp = health.get_i32("HP")?;

    let speed = actor.trait_info("Mobile").and_then(|t| t.get_i32("Speed"));
    let reveal_range = actor
        .trait_info("RevealsShroud")
        .and_then(|t| t.get("Range"))
        .and_then(parse_wdist);

    // Find the primary armament. C# uses Armament@PRIMARY when present,
    // else falls back to a bare `Armament:` block.
    let primary_weapon = actor
        .trait_instance("Armament", "PRIMARY")
        .or_else(|| actor.trait_info("Armament"))
        .and_then(|t| t.get("Weapon"))
        .map(|s| s.to_string());

    let must_be_destroyed = actor.has_trait("MustBeDestroyed");

    // Phase 8 — armor class, locomotor, turret turn-speed for Versus
    // multipliers and typed-component attachment in the env loader.
    let armor_class = actor
        .trait_info("Armor")
        .and_then(|t| t.get("Type"))
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "none".to_string());
    let locomotor = actor
        .trait_info("Mobile")
        .and_then(|t| t.get("Locomotor"))
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_default();
    let turret_turn_speed = actor
        .trait_info("Turreted")
        .and_then(|t| t.get_i32("TurnSpeed"));

    Some(UnitInfo {
        name: actor.name.clone(),
        hp,
        speed,
        reveal_range,
        primary_weapon,
        must_be_destroyed,
        armor_class,
        locomotor,
        turret_turn_speed,
    })
}

/// Build a `WeaponStats` from a resolved `WeaponInfo`. Walks the children
/// list (not `params`) for the `Warhead@*: SpreadDamage` (or
/// `TargetDamage`) block. Returns `None` if `Range`, `ReloadDelay`, or a
/// damaging warhead is missing.
///
/// Phase 8 also pulls:
/// * `Projectile.Speed`     → `projectile_speed` (zero for InstantHit).
/// * `Warhead@*.Spread`     → `splash_radius`    (zero when absent).
/// * `Warhead@*.Versus.*`   → `versus` per-armor-class multipliers.
fn weapon_stats_from_weapon(weapon: &WeaponInfo) -> Option<WeaponStats> {
    let range = weapon.get("Range").and_then(parse_wdist)?;
    let reload_delay = weapon.get_i32("ReloadDelay")?;

    // Warheads live in `weapon.children`, not in `params` (because they have
    // their own children block). Find the first damaging warhead — both
    // `SpreadDamage` (e1, tank cannons, missiles) and `TargetDamage` (dog
    // melee) carry `Damage`.
    let mut damage = None;
    let mut splash_radius = WDist::ZERO;
    let mut versus: BTreeMap<String, i32> = BTreeMap::new();
    for child in &weapon.children {
        if !child.key.starts_with("Warhead") {
            continue;
        }
        let is_damaging = matches!(child.value.as_str(), "SpreadDamage" | "TargetDamage");
        if !is_damaging {
            continue;
        }
        // First damaging warhead wins.
        if damage.is_none()
            && let Some(d_node) = child.child("Damage")
            && let Ok(d) = d_node.value.parse::<i32>()
        {
            damage = Some(d);
            // Splash spread is per-warhead — accept zero for `TargetDamage`.
            if let Some(s_node) = child.child("Spread")
                && let Some(wd) = parse_wdist(&s_node.value)
            {
                splash_radius = wd;
            }
            // Versus multipliers live as a child block under the warhead.
            if let Some(v_node) = child.child("Versus") {
                for entry in &v_node.children {
                    if let Ok(pct) = entry.value.parse::<i32>() {
                        versus.insert(entry.key.trim().to_ascii_lowercase(), pct);
                    }
                }
            }
            break;
        }
    }
    let damage = damage?;

    // Projectile speed — only meaningful for `Projectile: Missile` /
    // `Projectile: Bullet`. `Projectile: InstantHit` carries no Speed
    // (or a meaningless one). Treat anything missing as instant hit.
    let mut projectile_speed = WDist::ZERO;
    for child in &weapon.children {
        if child.key != "Projectile" {
            continue;
        }
        // value is the projectile-class name: "Missile", "Bullet",
        // "InstantHit", "TeslaZap". Only Missile/Bullet fly.
        let class = child.value.trim();
        if class != "Missile" && class != "Bullet" {
            break;
        }
        if let Some(s_node) = child.child("Speed")
            && let Some(wd) = parse_wdist(&s_node.value)
        {
            projectile_speed = wd;
        }
        break;
    }

    Some(WeaponStats {
        name: weapon.name.clone(),
        range,
        reload_delay,
        damage,
        projectile_speed,
        splash_radius,
        versus,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_ra_ruleset() {
        let manifest = env!("CARGO_MANIFEST_DIR");
        let vendored = format!("{}/../vendor/OpenRA/mods/ra", manifest);
        let mod_dir = Path::new(&vendored);
        if !mod_dir.exists() {
            eprintln!("Skipping: vendored OpenRA mod dir not found at {}", vendored);
            return;
        }
        let ruleset = load_ruleset(mod_dir).unwrap();

        // Basic sanity checks
        assert!(ruleset.actors.len() > 50, "Expected 50+ actors, got {}", ruleset.actors.len());
        eprintln!("Loaded {} actors, {} weapons", ruleset.actors.len(), ruleset.weapons.len());

        // Check MCV
        let mcv = ruleset.actor("MCV").expect("MCV not found");
        assert!(mcv.has_trait("Mobile"), "MCV should have Mobile");
        assert!(mcv.has_trait("Health"), "MCV should have Health");
        let hp = mcv.trait_info("Health").unwrap().get_i32("HP");
        assert_eq!(hp, Some(60000), "MCV HP should be 60000");

        // Check FACT
        let fact = ruleset.actor("FACT").expect("FACT not found");
        assert!(fact.has_trait("Building"), "FACT should have Building");
        let hp = fact.trait_info("Health").unwrap().get_i32("HP");
        assert_eq!(hp, Some(150000), "FACT HP should be 150000");

        // Check POWR cost
        let powr = ruleset.actor("POWR").expect("POWR not found");
        let cost = powr.trait_info("Valued").unwrap().get_i32("Cost");
        assert_eq!(cost, Some(300), "POWR cost should be 300");

        // Check E1 (Rifleman)
        let e1 = ruleset.actor("E1").expect("E1 not found");
        assert!(e1.has_trait("Health"), "E1 should have Health");
        assert!(e1.has_trait("Mobile"), "E1 should have Mobile");
    }

    #[test]
    fn load_ra_weapons() {
        let manifest = env!("CARGO_MANIFEST_DIR");
        let vendored = format!("{}/../vendor/OpenRA/mods/ra", manifest);
        let mod_dir = Path::new(&vendored);
        if !mod_dir.exists() {
            eprintln!("Skipping: vendored OpenRA mod dir not found at {}", vendored);
            return;
        }
        let ruleset = load_ruleset(mod_dir).unwrap();
        assert!(ruleset.weapons.len() > 10, "Expected 10+ weapons, got {}", ruleset.weapons.len());
        eprintln!("Weapons: {:?}", ruleset.weapons.keys().collect::<Vec<_>>());
    }
}
