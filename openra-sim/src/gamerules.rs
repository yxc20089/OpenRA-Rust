//! Compiled game rules — transforms parsed YAML Ruleset into fast lookup structs.
//!
//! Bridges the gap between `openra_data::rules::Ruleset` (parsed MiniYAML)
//! and the simulation's runtime needs (costs, stats, weapons).

use std::collections::BTreeMap;
use crate::actor::ActorKind;

/// Armor type for damage modifier lookups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ArmorType {
    None,
    Light,
    Heavy,
    Wood,
    Concrete,
}

/// A virtual prerequisite that a building provides to its owner.
#[derive(Debug, Clone)]
pub struct ProvidesPrereq {
    /// Factions this applies to (empty = all factions).
    pub factions: Vec<String>,
    /// The prerequisite name provided (e.g., "structures.allies").
    pub prerequisite: String,
    /// Additional prerequisites required for this provision to be active.
    pub requires_prerequisites: Vec<String>,
}

/// Compiled stats for one actor type.
#[derive(Debug, Clone)]
pub struct ActorStats {
    pub kind: ActorKind,
    pub hp: i32,
    pub speed: i32,
    pub cost: i32,
    pub power: i32,
    pub footprint: (i32, i32),
    pub armor_type: ArmorType,
    pub is_building: bool,
    /// Whether the C# `MustBeDestroyed` trait is set (used by victory
    /// detection: only these actors count toward "this side is dead").
    /// Buildings without this flag (defenses, scenery like powr/barr) do
    /// NOT need to be destroyed for the opposing side to win.
    pub must_be_destroyed: bool,
    pub prerequisites: Vec<String>,
    pub weapons: Vec<String>,
    pub sight_range: i32,
    pub provides_prerequisites: Vec<ProvidesPrereq>,
    pub build_palette_order: i32,
}

/// Compiled stats for one weapon type.
#[derive(Debug, Clone, Default)]
pub struct WeaponStats {
    pub damage: i32,
    pub range: i32,
    pub reload_delay: i32,
    pub burst: i32,
    pub versus: BTreeMap<ArmorType, i32>,
    /// Phase 8: `Projectile.Speed` in world units per tick. Zero means
    /// `Projectile: InstantHit` — apply damage immediately at fire
    /// time. Non-zero means spawn a `Projectile` entity that flies to
    /// the target over multiple ticks and applies damage on impact.
    pub projectile_speed: i32,
    /// Phase 8: `Warhead@*: SpreadDamage -> Spread` in world units.
    /// Zero for single-target weapons; ~128 for RedEye.
    pub splash_radius: i32,
}

/// All game rules compiled for fast simulation lookups.
#[derive(Debug, Clone)]
pub struct GameRules {
    pub actors: BTreeMap<String, ActorStats>,
    pub weapons: BTreeMap<String, WeaponStats>,
}

impl GameRules {
    /// Build GameRules from a parsed Ruleset.
    pub fn from_ruleset(ruleset: &openra_data::rules::Ruleset) -> Self {
        let mut actors = BTreeMap::new();
        let mut weapons = BTreeMap::new();

        for (name, info) in &ruleset.actors {
            let key = name.to_lowercase();
            // Skip abstract base types (start with ^)
            if name.starts_with('^') {
                continue;
            }

            let hp = info.trait_info("Health")
                .and_then(|t| t.get_i32("HP"))
                .unwrap_or(0);

            let speed = info.trait_info("Mobile")
                .and_then(|t| t.get_i32("Speed"))
                .unwrap_or(0);

            let cost = info.trait_info("Valued")
                .and_then(|t| t.get_i32("Cost"))
                .unwrap_or(0);

            let power = info.trait_info("Power")
                .and_then(|t| t.get_i32("Amount"))
                .unwrap_or(0);

            let is_building = info.has_trait("Building");
            let must_be_destroyed = info.has_trait("MustBeDestroyed");

            let footprint = if is_building {
                parse_building_dimensions(info)
            } else {
                (1, 1)
            };

            let armor_type = info.trait_info("Armor")
                .and_then(|t| t.get("Type"))
                .map(parse_armor_type)
                .unwrap_or(ArmorType::None);

            let prerequisites = info.trait_info("Buildable")
                .and_then(|t| t.get("Prerequisites"))
                .map(|s| s.split(',').map(|p| p.trim().to_lowercase()).collect())
                .unwrap_or_default();

            let mut weapon_names: Vec<String> = info.traits_of("Armament")
                .iter()
                .filter_map(|t| t.get("Weapon").map(|w| w.to_string()))
                .collect();

            // Engine-side default for garrison-only defenses. RA's `pbox`
            // (pillbox) is a `AttackGarrisoned` defense — its offensive
            // power comes from infantry loaded into its `Cargo`, so the
            // C# YAML carries NO direct `Armament` trait. The engine does
            // not model garrisoning, so the auto-target loop's
            // `weapons.first()` returns `None` and a *built* pbox never
            // fires (it just stands inert). To make the pillbox a real
            // active direct-fire tower we attach its canonical RA
            // anti-infantry weapon `M60mg` (the pillbox machine-gun:
            // Damage 1000 × Burst 5, ReloadDelay 30, Range 4c0, anti-
            // infantry `Versus None:150`) when the actor classifies as a
            // ground turret but has no explicit Armament. This is weaker
            // and shorter-ranged than the `gun` turret's `TurretGun`
            // (Damage 6000, Range 6c512), matching the pbox's role as the
            // cheap anti-infantry pillbox. Defenses that DO carry an
            // explicit `Armament` (gun, ftur, tsla) are untouched.
            if weapon_names.is_empty()
                && matches!(
                    crate::traits::classify_defense(&key),
                    Some(crate::traits::DefenseKind::GroundTurret)
                )
            {
                weapon_names.push("M60mg".to_string());
            }

            // RA YAML stores Range as "Xc0" / "XcY" (cells + sub-cell). The
            // C# WDist parser is lenient: also accepts a bare integer in
            // world units. Use `parse_range` which handles both, then
            // collapse to a cell-count for `sight_range` (cells, not WDist).
            let sight_range = info.trait_info("RevealsShroud")
                .and_then(|t| t.get("Range"))
                .map(|s| parse_range(s) / 1024)
                .unwrap_or(if is_building { 5 } else { 4 });

            let kind = classify_actor(&key, info);

            let build_palette_order = info.trait_info("Buildable")
                .and_then(|t| t.get_i32("BuildPaletteOrder"))
                .unwrap_or(9999);

            // Parse ProvidesPrerequisite traits
            let mut provides_prerequisites = Vec::new();
            for pp in info.traits_of("ProvidesPrerequisite") {
                let prerequisite = pp.get("Prerequisite")
                    .map(|s| s.to_lowercase())
                    .unwrap_or_else(|| key.clone()); // @buildingname: provides own name

                let factions: Vec<String> = pp.get("Factions")
                    .map(|s| s.split(',').map(|f| f.trim().to_lowercase()).collect())
                    .unwrap_or_default();

                let requires_prerequisites: Vec<String> = pp.get("RequiresPrerequisites")
                    .map(|s| s.split(',').map(|p| p.trim().to_lowercase()).collect())
                    .unwrap_or_default();

                provides_prerequisites.push(ProvidesPrereq {
                    factions,
                    prerequisite,
                    requires_prerequisites,
                });
            }

            actors.insert(key, ActorStats {
                kind,
                hp,
                speed,
                cost,
                power,
                footprint,
                armor_type,
                is_building,
                must_be_destroyed,
                prerequisites,
                weapons: weapon_names,
                sight_range,
                provides_prerequisites,
                build_palette_order,
            });
        }

        for (name, info) in &ruleset.weapons {
            // C# OpenRA stores `Damage` inside `Warhead@<n>: SpreadDamage` —
            // walk the warhead children rather than the top-level `Damage:`
            // field (which is empty for almost every weapon). Phase 8 also
            // accepts `Warhead@*: TargetDamage` (e.g. DogJaw).
            let damage = parse_damage_from_warheads(info)
                .or_else(|| info.get_i32("Damage"))
                .unwrap_or(0);
            let range = info.get("Range")
                .map(|s| parse_range(s))
                .unwrap_or(5 * 1024);
            let reload_delay = info.get_i32("ReloadDelay").unwrap_or(15);
            let burst = info.get_i32("Burst").unwrap_or(1);

            // Parse Versus block from warhead children
            let versus = parse_versus(info);

            // Phase 8 — projectile speed & splash radius.
            let projectile_speed = parse_projectile_speed(info);
            let splash_radius = parse_splash_radius(info);

            weapons.insert(name.clone(), WeaponStats {
                damage,
                range,
                reload_delay,
                burst,
                versus,
                projectile_speed,
                splash_radius,
            });
        }

        GameRules { actors, weapons }
    }

    /// Build default GameRules matching the current hardcoded values.
    /// Used when no Ruleset is available (e.g., sync tests).
    pub fn defaults() -> Self {
        let mut actors = BTreeMap::new();
        let mut weapons = BTreeMap::new();

        // Helper to insert actor stats
        macro_rules! actor {
            ($name:expr, $kind:expr, $hp:expr, $speed:expr, $cost:expr, $power:expr,
             $fw:expr, $fh:expr, $building:expr) => {
                actors.insert($name.to_string(), ActorStats {
                    kind: $kind, hp: $hp, speed: $speed, cost: $cost, power: $power,
                    footprint: ($fw, $fh), armor_type: ArmorType::None,
                    is_building: $building,
                    // Defaults treat fact/proc as MustBeDestroyed; everything
                    // else off. The Ruleset path (above) reads the C# trait
                    // directly so this is just for the no-ruleset fallback.
                    must_be_destroyed: $building && matches!($name, "fact" | "proc"),
                    prerequisites: Vec::new(),
                    weapons: Vec::new(), sight_range: if $building { 5 } else { 4 },
                    provides_prerequisites: Vec::new(), build_palette_order: 9999,
                });
            };
        }

        // Buildings
        actor!("powr", ActorKind::Building, 40000, 0, 300, 100, 2, 2, true);
        actor!("apwr", ActorKind::Building, 70000, 0, 500, 200, 2, 2, true);
        actor!("tent", ActorKind::Building, 50000, 0, 400, 0, 2, 2, true);
        actor!("barr", ActorKind::Building, 50000, 0, 400, 0, 2, 2, true);
        actor!("weap", ActorKind::Building, 100000, 0, 2000, 0, 3, 2, true);
        actor!("weap.ukraine", ActorKind::Building, 100000, 0, 2000, 0, 3, 2, true);
        actor!("proc", ActorKind::Building, 90000, 0, 1400, 0, 3, 2, true);
        actor!("fact", ActorKind::Building, 150000, 0, 0, 0, 3, 2, true);
        actor!("fix", ActorKind::Building, 80000, 0, 1200, 0, 3, 2, true);
        actor!("dome", ActorKind::Building, 60000, 0, 2800, -200, 2, 2, true);
        actor!("hpad", ActorKind::Building, 80000, 0, 500, 0, 2, 2, true);
        actor!("afld", ActorKind::Building, 80000, 0, 500, 0, 2, 2, true);
        actor!("spen", ActorKind::Building, 120000, 0, 650, 0, 3, 3, true);
        actor!("syrd", ActorKind::Building, 120000, 0, 650, 0, 3, 3, true);
        actor!("atek", ActorKind::Building, 60000, 0, 2800, -50, 2, 2, true);
        actor!("stek", ActorKind::Building, 60000, 0, 2800, -50, 2, 2, true);
        actor!("tsla", ActorKind::Building, 40000, 0, 1500, -200, 1, 1, true);
        actor!("sam", ActorKind::Building, 40000, 0, 750, -80, 1, 1, true);
        actor!("gap", ActorKind::Building, 40000, 0, 500, -60, 1, 1, true);
        actor!("agun", ActorKind::Building, 40000, 0, 600, -20, 1, 1, true);
        actor!("pbox", ActorKind::Building, 40000, 0, 400, 0, 1, 1, true);
        actor!("hbox", ActorKind::Building, 40000, 0, 600, 0, 1, 1, true);
        actor!("gun", ActorKind::Building, 40000, 0, 600, 0, 1, 1, true);
        actor!("ftur", ActorKind::Building, 40000, 0, 600, 0, 1, 1, true);

        // Infantry
        actor!("e1", ActorKind::Infantry, 50000, 43, 100, 0, 1, 1, false);
        actor!("e2", ActorKind::Infantry, 50000, 43, 160, 0, 1, 1, false);
        actor!("e3", ActorKind::Infantry, 45000, 43, 300, 0, 1, 1, false);
        actor!("e4", ActorKind::Infantry, 60000, 43, 200, 0, 1, 1, false);
        actor!("e6", ActorKind::Infantry, 25000, 43, 500, 0, 1, 1, false);
        actor!("e7", ActorKind::Infantry, 100000, 43, 600, 0, 1, 1, false);
        actor!("shok", ActorKind::Infantry, 80000, 43, 400, 0, 1, 1, false);
        actor!("medi", ActorKind::Infantry, 80000, 43, 600, 0, 1, 1, false);
        actor!("mech", ActorKind::Infantry, 70000, 43, 500, 0, 1, 1, false);
        actor!("dog", ActorKind::Infantry, 20000, 85, 200, 0, 1, 1, false);
        actor!("spy", ActorKind::Infantry, 25000, 56, 500, 0, 1, 1, false);
        actor!("thf", ActorKind::Infantry, 50000, 56, 500, 0, 1, 1, false);
        // Tanya — Allied commando hero. Single high-HP, fast-moving
        // infantry with a strong anti-infantry sidearm. HP ~3x e1,
        // speed ~1.5x e1, damage ~5x M1Carbine, faster reload. The
        // weapon (`TanyaPistol`) is registered below in the weapons
        // table; her actor entry pre-binds it so the auto-engage /
        // best_weapon_against paths resolve correctly when she is
        // placed via `insert_test_actor` (no vendor YAML needed).
        actors.insert("tanya".to_string(), ActorStats {
            kind: ActorKind::Infantry,
            hp: 150000,
            speed: 64,
            cost: 1200,
            power: 0,
            footprint: (1, 1),
            armor_type: ArmorType::None,
            is_building: false,
            must_be_destroyed: false,
            prerequisites: vec!["tent".to_string(), "atek".to_string()],
            weapons: vec!["TanyaPistol".to_string()],
            sight_range: 6,
            provides_prerequisites: Vec::new(),
            build_palette_order: 9999,
        });

        // Vehicles
        actor!("1tnk", ActorKind::Vehicle, 160000, 113, 700, 0, 1, 1, false);
        actor!("2tnk", ActorKind::Vehicle, 260000, 85, 800, 0, 1, 1, false);
        actor!("3tnk", ActorKind::Vehicle, 400000, 71, 1500, 0, 1, 1, false);
        actor!("4tnk", ActorKind::Vehicle, 500000, 56, 1800, 0, 1, 1, false);
        actor!("v2rl", ActorKind::Vehicle, 150000, 71, 700, 0, 1, 1, false);
        actor!("arty", ActorKind::Vehicle, 75000, 85, 600, 0, 1, 1, false);
        actor!("harv", ActorKind::Vehicle, 60000, 56, 1400, 0, 1, 1, false);
        // MCV uses ActorKind::Mcv so the world.rs DeployTransform
        // handler (gated on `actor.kind == ActorKind::Mcv`) fires.
        // The from_ruleset() path also routes MCV through Mcv via the
        // name-based special-case in classify_actor; keeping the
        // hard-coded fallback consistent.
        actor!("mcv", ActorKind::Mcv, 60000, 56, 2500, 0, 1, 1, false);
        actor!("apc", ActorKind::Vehicle, 200000, 113, 800, 0, 1, 1, false);
        actor!("jeep", ActorKind::Vehicle, 150000, 113, 600, 0, 1, 1, false);
        actor!("mnly", ActorKind::Vehicle, 55000, 85, 500, 0, 1, 1, false);
        actor!("ttnk", ActorKind::Vehicle, 100000, 71, 1500, 0, 1, 1, false);
        actor!("ctnk", ActorKind::Vehicle, 100000, 71, 2000, 0, 1, 1, false);

        // Aircraft
        actor!("heli", ActorKind::Aircraft, 100000, 0, 1200, 0, 1, 1, false);
        actor!("hind", ActorKind::Aircraft, 100000, 0, 1200, 0, 1, 1, false);
        actor!("mig", ActorKind::Aircraft, 100000, 0, 2000, 0, 1, 1, false);
        actor!("yak", ActorKind::Aircraft, 100000, 0, 800, 0, 1, 1, false);

        // Naval
        actor!("ss", ActorKind::Ship, 100000, 0, 950, 0, 1, 1, false);
        actor!("msub", ActorKind::Ship, 100000, 0, 1800, 0, 1, 1, false);
        actor!("sub", ActorKind::Ship, 100000, 0, 950, 0, 1, 1, false);
        actor!("dd", ActorKind::Ship, 100000, 0, 1000, 0, 1, 1, false);
        actor!("ca", ActorKind::Ship, 100000, 0, 2000, 0, 1, 1, false);
        actor!("pt", ActorKind::Ship, 100000, 0, 700, 0, 1, 1, false);

        // Set prerequisites for units and buildings (matching OpenRA rules)
        // Infantry require barracks (tent/barr)
        for name in &["e1", "e2", "e3", "e4", "e6", "e7", "shok", "medi", "mech", "dog", "spy", "thf"] {
            if let Some(a) = actors.get_mut(*name) {
                a.prerequisites = vec!["tent".to_string()];
            }
        }
        // Basic vehicles require war factory (weap)
        for name in &["1tnk", "2tnk", "apc", "jeep", "mnly", "harv"] {
            if let Some(a) = actors.get_mut(*name) {
                a.prerequisites = vec!["weap".to_string()];
            }
        }
        // Heavy/advanced vehicles require weap + dome (radar dome)
        for name in &["3tnk", "4tnk", "v2rl", "arty", "ttnk", "ctnk"] {
            if let Some(a) = actors.get_mut(*name) {
                a.prerequisites = vec!["weap".to_string(), "dome".to_string()];
            }
        }
        // Buildings prerequisites (matching OpenRA)
        if let Some(a) = actors.get_mut("tent") { a.prerequisites = vec!["powr".to_string()]; }
        if let Some(a) = actors.get_mut("barr") { a.prerequisites = vec!["powr".to_string()]; }
        if let Some(a) = actors.get_mut("weap") { a.prerequisites = vec!["proc".to_string()]; }
        if let Some(a) = actors.get_mut("proc") { a.prerequisites = vec!["powr".to_string()]; }
        if let Some(a) = actors.get_mut("dome") { a.prerequisites = vec!["proc".to_string()]; }
        if let Some(a) = actors.get_mut("fix") { a.prerequisites = vec!["weap".to_string()]; }
        if let Some(a) = actors.get_mut("hpad") { a.prerequisites = vec!["dome".to_string()]; }
        if let Some(a) = actors.get_mut("afld") { a.prerequisites = vec!["dome".to_string()]; }
        if let Some(a) = actors.get_mut("atek") { a.prerequisites = vec!["weap".to_string(), "dome".to_string()]; }
        if let Some(a) = actors.get_mut("stek") { a.prerequisites = vec!["weap".to_string(), "dome".to_string()]; }

        // ProvidesPrerequisite for buildings (simplified defaults for testing)
        // FACT provides structures.allies / structures.soviet based on faction
        if let Some(a) = actors.get_mut("fact") {
            a.provides_prerequisites = vec![
                ProvidesPrereq { factions: vec!["allies".into(),"england".into(),"france".into(),"germany".into()], prerequisite: "structures.allies".into(), requires_prerequisites: vec![] },
                ProvidesPrereq { factions: vec!["soviet".into(),"russia".into(),"ukraine".into()], prerequisite: "structures.soviet".into(), requires_prerequisites: vec![] },
                ProvidesPrereq { factions: vec![], prerequisite: "fact".into(), requires_prerequisites: vec![] },
            ];
        }
        // POWR/APWR provide anypower
        if let Some(a) = actors.get_mut("powr") { a.provides_prerequisites = vec![ProvidesPrereq { factions: vec![], prerequisite: "anypower".into(), requires_prerequisites: vec![] }]; }
        if let Some(a) = actors.get_mut("apwr") { a.provides_prerequisites = vec![ProvidesPrereq { factions: vec![], prerequisite: "anypower".into(), requires_prerequisites: vec![] }]; }
        // TENT provides barracks + infantry.allies
        if let Some(a) = actors.get_mut("tent") {
            a.provides_prerequisites = vec![
                ProvidesPrereq { factions: vec![], prerequisite: "barracks".into(), requires_prerequisites: vec![] },
                ProvidesPrereq { factions: vec![], prerequisite: "tent".into(), requires_prerequisites: vec![] },
                ProvidesPrereq { factions: vec!["allies".into(),"england".into(),"france".into(),"germany".into()], prerequisite: "infantry.allies".into(), requires_prerequisites: vec![] },
            ];
        }
        // BARR provides barracks + infantry.soviet
        if let Some(a) = actors.get_mut("barr") {
            a.provides_prerequisites = vec![
                ProvidesPrereq { factions: vec![], prerequisite: "barracks".into(), requires_prerequisites: vec![] },
                ProvidesPrereq { factions: vec![], prerequisite: "barr".into(), requires_prerequisites: vec![] },
                ProvidesPrereq { factions: vec!["soviet".into(),"russia".into(),"ukraine".into()], prerequisite: "infantry.soviet".into(), requires_prerequisites: vec![] },
            ];
        }
        // WEAP provides vehicles.allies / vehicles.soviet
        if let Some(a) = actors.get_mut("weap") {
            a.provides_prerequisites = vec![
                ProvidesPrereq { factions: vec![], prerequisite: "weap".into(), requires_prerequisites: vec![] },
                ProvidesPrereq { factions: vec!["allies".into(),"england".into(),"france".into(),"germany".into()], prerequisite: "vehicles.allies".into(), requires_prerequisites: vec![] },
                ProvidesPrereq { factions: vec!["soviet".into(),"russia".into(),"ukraine".into()], prerequisite: "vehicles.soviet".into(), requires_prerequisites: vec![] },
            ];
        }
        // Other buildings provide themselves
        for bname in &["proc","dome","fix","hpad","afld","spen","syrd","atek","stek","sam","agun","gap","tsla","pbox","hbox","gun","ftur"] {
            if let Some(a) = actors.get_mut(*bname) {
                if a.provides_prerequisites.is_empty() {
                    a.provides_prerequisites = vec![ProvidesPrereq { factions: vec![], prerequisite: bname.to_string(), requires_prerequisites: vec![] }];
                }
            }
        }
        // ATEK/STEK also provide techcenter
        if let Some(a) = actors.get_mut("atek") { a.provides_prerequisites.push(ProvidesPrereq { factions: vec![], prerequisite: "techcenter".into(), requires_prerequisites: vec![] }); }
        if let Some(a) = actors.get_mut("stek") { a.provides_prerequisites.push(ProvidesPrereq { factions: vec![], prerequisite: "techcenter".into(), requires_prerequisites: vec![] }); }

        // Default weapon
        weapons.insert("default".to_string(), WeaponStats {
            damage: 100,
            range: 5 * 1024,
            reload_delay: 1,
            burst: 1,
            versus: BTreeMap::new(),
            projectile_speed: 0,
            splash_radius: 0,
        });

        // Tanya's sidearm — strong, fast-firing, single-target anti-
        // infantry. Damage 5000 (5x M1Carbine), reload_delay 10 (2x
        // faster than M1Carbine's 20), range 5 cells, instant-hit. A
        // single tanya outpaces the burst output of multiple e1's at
        // close range, which is the load-bearing "hero" property.
        weapons.insert("TanyaPistol".to_string(), WeaponStats {
            damage: 5000,
            range: 5 * 1024,
            reload_delay: 10,
            burst: 1,
            versus: BTreeMap::new(),
            projectile_speed: 0,
            splash_radius: 0,
        });

        GameRules { actors, weapons }
    }

    /// Look up actor stats, falling back to a generic default.
    pub fn actor(&self, name: &str) -> Option<&ActorStats> {
        self.actors.get(name)
    }

    /// Look up weapon stats.
    pub fn weapon(&self, name: &str) -> Option<&WeaponStats> {
        self.weapons.get(name)
    }

    /// Effective per-hit damage of a weapon against a given armor class,
    /// after applying the weapon's `Versus` multiplier. This is the
    /// armor-class damage model: `damage × versus[armor] / 100`. A
    /// missing `Versus` entry defaults to 100% (no modifier), matching
    /// C# `Warhead.DamageVersus`.
    pub fn effective_damage(weapon: &WeaponStats, target_armor: ArmorType) -> i32 {
        let pct = weapon.versus.get(&target_armor).copied().unwrap_or(100);
        let scaled = (weapon.damage as i64) * (pct as i64) / 100;
        scaled.max(0) as i32
    }

    /// Select the best armament for an attacker firing at a target with
    /// the given `target_armor` class. OpenRA infantry/vehicles can
    /// carry multiple armaments (e.g. e3 has RedEye anti-air PRIMARY +
    /// Dragon anti-ground SECONDARY); the engine must pick the weapon
    /// that deals the most effective damage to *this* target rather
    /// than blindly using `weapons[0]`.
    ///
    /// Effective damage already folds in the per-armor-class `Versus`
    /// multiplier, so an anti-armor warhead (Dragon: 5000 base, Heavy
    /// 100%) is correctly preferred over an anti-air missile (RedEye:
    /// 2400 base, no Heavy entry ⇒ 100%) when shooting a heavy tank,
    /// and the anti-air weapon is preferred against aircraft armor.
    ///
    /// Returns `(weapon_name, &WeaponStats)`. Falls back to the first
    /// listed weapon when no weapon has positive effective damage (so
    /// degenerate rulesets still fire something), and to `None` only
    /// when the attacker has no weapons at all.
    pub fn best_weapon_against(
        &self,
        attacker_type: &str,
        target_armor: ArmorType,
    ) -> Option<(&str, &WeaponStats)> {
        let stats = self.actor(attacker_type)?;
        let mut best: Option<(&str, &WeaponStats, i32)> = None;
        for wname in &stats.weapons {
            let Some(w) = self.weapon(wname) else { continue };
            let eff = Self::effective_damage(w, target_armor);
            match best {
                Some((_, _, beff)) if beff >= eff => {}
                _ => best = Some((wname.as_str(), w, eff)),
            }
        }
        if let Some((n, w, eff)) = best {
            if eff > 0 {
                return Some((n, w));
            }
        }
        // No weapon has positive effective damage — fall back to the
        // first listed weapon so the attacker still behaves.
        stats
            .weapons
            .first()
            .and_then(|n| self.weapon(n).map(|w| (n.as_str(), w)))
    }

    /// Get production cost for an item.
    pub fn cost(&self, name: &str) -> i32 {
        self.actors.get(name).map(|a| a.cost).unwrap_or(0)
    }

    /// Check if an item is a unit (not a building).
    pub fn is_unit(&self, name: &str) -> bool {
        self.actors.get(name).map(|a| !a.is_building).unwrap_or(false)
    }
}

/// Parse OpenRA range format "Xc0" where X is cells and 0 is sub-cell.
/// e.g., "6c0" = 6*1024 = 6144, "5c512" = 5*1024+512 = 5632
fn parse_range(s: &str) -> i32 {
    if let Some(pos) = s.find('c') {
        let cells: i32 = s[..pos].parse().unwrap_or(5);
        let sub: i32 = s[pos + 1..].parse().unwrap_or(0);
        cells * 1024 + sub
    } else {
        s.parse::<i32>().unwrap_or(5) * 1024
    }
}

/// Parse armor type string to enum.
fn parse_armor_type(s: &str) -> ArmorType {
    match s.to_lowercase().as_str() {
        "light" => ArmorType::Light,
        "heavy" => ArmorType::Heavy,
        "wood" => ArmorType::Wood,
        "concrete" => ArmorType::Concrete,
        _ => ArmorType::None,
    }
}

/// Parse building dimensions from Building trait or Footprint.
fn parse_building_dimensions(info: &openra_data::rules::ActorInfo) -> (i32, i32) {
    // Try Building.Dimensions first
    if let Some(building) = info.trait_info("Building") {
        if let Some(dims) = building.get("Dimensions") {
            let parts: Vec<&str> = dims.split(',').collect();
            if parts.len() >= 2 {
                let w = parts[0].trim().parse().unwrap_or(2);
                let h = parts[1].trim().parse().unwrap_or(2);
                return (w, h);
            }
        }
    }
    (2, 2) // Default building size
}

/// Classify an actor into ActorKind based on its traits.
fn classify_actor(name: &str, info: &openra_data::rules::ActorInfo) -> ActorKind {
    // MCV is its own ActorKind (not a generic Vehicle) so the
    // world.rs DeployTransform handler — gated on
    // `actor.kind == ActorKind::Mcv` — fires for scenario-declared
    // MCVs. Without this special-case, classify_actor's locomotor
    // check returns Vehicle for MCV and `Command::deploy` silently
    // no-ops on the scenario MCV (caught in bench MCV-deploy smoke).
    if name.eq_ignore_ascii_case("mcv") {
        return ActorKind::Mcv;
    }
    if info.has_trait("Building") {
        ActorKind::Building
    } else if info.has_trait("Aircraft") {
        ActorKind::Aircraft
    } else if info.has_trait("Mobile") {
        let locomotor = info.trait_info("Mobile")
            .and_then(|m| m.get("Locomotor"))
            .unwrap_or("");
        if locomotor.contains("foot") {
            ActorKind::Infantry
        } else if locomotor.contains("naval") || locomotor.contains("lcraft") {
            ActorKind::Ship
        } else {
            ActorKind::Vehicle
        }
    } else {
        ActorKind::World
    }
}

/// Walk a weapon's warhead children for the first damaging warhead
/// (`Warhead@*: SpreadDamage` or `Warhead@*: TargetDamage`) carrying a
/// `Damage:` field. Returns `None` if no warhead has a damage value
/// (e.g. CreateEffect / LeaveSmudge warheads).
///
/// Reference: `OpenRA.Mods.Common/Warheads/SpreadDamageWarhead.cs` and
/// `TargetDamageWarhead.cs` (used by DogJaw for melee).
fn parse_damage_from_warheads(info: &openra_data::rules::WeaponInfo) -> Option<i32> {
    for child in &info.children {
        if !child.key.starts_with("Warhead") {
            continue;
        }
        // The MiniYAML inheritance resolver leaves `Warhead@1Dam: SpreadDamage`
        // as a node whose `value == "SpreadDamage"`. Other warhead types
        // (CreateEffect, LeaveSmudge, ...) carry no Damage field; skip
        // them by checking the value before peeking at children.
        if child.value != "SpreadDamage" && child.value != "TargetDamage" {
            continue;
        }
        for gc in &child.children {
            if gc.key == "Damage" {
                if let Ok(d) = gc.value.parse::<i32>() {
                    return Some(d);
                }
            }
        }
    }
    None
}

/// Phase 8 — parse `Projectile.Speed` (in world units per tick).
///
/// `Projectile: InstantHit` returns 0; `Projectile: Missile / Bullet`
/// returns the parsed `Speed:` if present. Anything else (e.g.
/// `Projectile: TeslaZap`) returns 0 — those are visual-effect-only
/// projectile classes treated as instant in the sim.
///
/// Speed values use the C# WDist text format. Plain integers (`853`)
/// are raw fixed-point units; `1c682` is `1*1024+682 = 1706` units.
fn parse_projectile_speed(info: &openra_data::rules::WeaponInfo) -> i32 {
    for child in &info.children {
        if child.key != "Projectile" {
            continue;
        }
        let class = child.value.trim();
        if class != "Missile" && class != "Bullet" {
            return 0;
        }
        for gc in &child.children {
            if gc.key == "Speed" {
                return parse_wdist_text(&gc.value);
            }
        }
        return 0;
    }
    0
}

/// Phase 8 — parse splash radius from `Warhead@*: SpreadDamage -> Spread`.
///
/// Returned in world units (1024 = 1 cell). Zero when not specified
/// (single-target weapons). Inheritance is already resolved by the
/// upstream MiniYAML loader. Plain numbers are raw fixed-point units;
/// `Spread: 128` returns `128`, not `128 * 1024`.
fn parse_splash_radius(info: &openra_data::rules::WeaponInfo) -> i32 {
    for child in &info.children {
        if !child.key.starts_with("Warhead") {
            continue;
        }
        if child.value != "SpreadDamage" {
            continue;
        }
        for gc in &child.children {
            if gc.key == "Spread" {
                return parse_wdist_text(&gc.value);
            }
        }
    }
    0
}

/// Parse a C# `WDist` text literal correctly: plain integer = raw
/// fixed-point units (NOT cells), `Xc<sub>` = X*1024+sub. This
/// matches `OpenRA.Game/WDist.cs::TryParse` and the openra-data
/// `parse_wdist` helper. Unlike the Phase-3 `parse_range` helper
/// (which incorrectly multiplies plain integers by 1024 — preserved
/// for backward compatibility), this helper is used for fields that
/// always carry raw wdist values (`Speed`, `Spread`).
fn parse_wdist_text(s: &str) -> i32 {
    let s = s.trim();
    if let Some(idx) = s.find('c') {
        let cells: i32 = s[..idx].trim().parse().unwrap_or(0);
        let sub: i32 = s[idx + 1..].trim().parse().unwrap_or(0);
        let sign = if cells < 0 { -1 } else { 1 };
        cells * 1024 + sign * sub
    } else {
        s.parse::<i32>().unwrap_or(0)
    }
}

/// Parse Versus block from weapon warhead children.
fn parse_versus(info: &openra_data::rules::WeaponInfo) -> BTreeMap<ArmorType, i32> {
    let mut versus = BTreeMap::new();
    // Search warhead children for Versus entries
    for child in &info.children {
        if child.key.starts_with("Warhead") || child.key.contains("SpreadDamage") {
            for grandchild in &child.children {
                if grandchild.key == "Versus" {
                    for entry in &grandchild.children {
                        let armor = parse_armor_type(&entry.key);
                        if let Ok(pct) = entry.value.parse::<i32>() {
                            versus.insert(armor, pct);
                        }
                    }
                }
            }
        }
    }
    versus
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_have_all_common_units() {
        let rules = GameRules::defaults();
        // Buildings
        assert_eq!(rules.cost("powr"), 300);
        assert_eq!(rules.cost("weap"), 2000);
        assert_eq!(rules.cost("proc"), 1400);
        // Infantry
        assert_eq!(rules.cost("e1"), 100);
        assert_eq!(rules.cost("e3"), 300);
        // Vehicles
        assert_eq!(rules.cost("2tnk"), 800);
        assert_eq!(rules.cost("harv"), 1400);
        assert_eq!(rules.cost("mcv"), 2500);
        // Check stats
        let mcv = rules.actor("mcv").unwrap();
        assert_eq!(mcv.hp, 60000);
        assert_eq!(mcv.speed, 56);
        assert_eq!(mcv.kind, ActorKind::Vehicle);
        // Buildings
        let fact = rules.actor("fact").unwrap();
        assert!(fact.is_building);
        assert_eq!(fact.footprint, (3, 2));
        assert_eq!(fact.hp, 150000);
        // Power
        assert_eq!(rules.actor("powr").unwrap().power, 100);
        assert_eq!(rules.actor("apwr").unwrap().power, 200);
        assert_eq!(rules.actor("tsla").unwrap().power, -200);
    }

    #[test]
    fn is_unit_vs_building() {
        let rules = GameRules::defaults();
        assert!(rules.is_unit("e1"));
        assert!(rules.is_unit("2tnk"));
        assert!(rules.is_unit("harv"));
        assert!(!rules.is_unit("powr"));
        assert!(!rules.is_unit("fact"));
        assert!(!rules.is_unit("weap"));
    }

    #[test]
    fn parse_range_format() {
        assert_eq!(parse_range("6c0"), 6144);
        assert_eq!(parse_range("5c512"), 5632);
        assert_eq!(parse_range("3c0"), 3072);
        assert_eq!(parse_range("10"), 10240);
    }

    #[test]
    fn weapon_damage_from_warhead_for_real_yaml() {
        // Load the vendored RA ruleset and check Phase 6's representative
        // weapons end up with the right damage values. Skips silently
        // if the vendor dir is missing (CI without submodules).
        let manifest = env!("CARGO_MANIFEST_DIR");
        let mod_dir =
            std::path::PathBuf::from(format!("{}/../vendor/OpenRA/mods/ra", manifest));
        if !mod_dir.exists() {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
        let ruleset = openra_data::rules::load_ruleset(&mod_dir).unwrap();
        let rules = GameRules::from_ruleset(&ruleset);

        // 25mm cannon (1tnk): SpreadDamage Damage = 2500
        let w = rules.weapon("25mm").expect("25mm not parsed");
        assert_eq!(w.damage, 2500, "25mm damage");
        // 90mm (2tnk): inherits from ^Cannon Damage = 4000
        let w = rules.weapon("90mm").expect("90mm not parsed");
        assert_eq!(w.damage, 4000, "90mm damage");
        // 105mm (3tnk): inherits 4000, burst 2
        let w = rules.weapon("105mm").expect("105mm not parsed");
        assert_eq!(w.damage, 4000, "105mm damage");
        assert_eq!(w.burst, 2, "105mm burst");
        // TurretGun (gun building): explicit Damage 6000
        let w = rules.weapon("TurretGun").expect("TurretGun not parsed");
        assert_eq!(w.damage, 6000, "TurretGun damage");
    }

    #[test]
    fn sight_range_parses_wdist_format() {
        // RA stores RevealsShroud.Range as "Xc0" (cells + sub-cell). The
        // old code called `get_i32` which silently fell through to a
        // 4-cell default for every actor — letting tesla coils
        // out-range scout sight (gun = 6c0 attack vs jeep = 7c0 sight).
        // This test pins the values so the regression can't sneak back.
        let manifest = env!("CARGO_MANIFEST_DIR");
        let mod_dir =
            std::path::PathBuf::from(format!("{}/../vendor/OpenRA/mods/ra", manifest));
        if !mod_dir.exists() {
            eprintln!("skipping: vendored OpenRA mod dir not found");
            return;
        }
        let ruleset = openra_data::rules::load_ruleset(&mod_dir).unwrap();
        let rules = GameRules::from_ruleset(&ruleset);

        // Vehicles: jeep should out-see tesla. Defenses don't get to
        // shoot first if the scout's reveal radius covers them.
        let jeep = rules.actor("jeep").expect("jeep not parsed");
        assert_eq!(jeep.sight_range, 7, "jeep sight (7c0 expected)");
        let dog = rules.actor("dog").expect("dog not parsed");
        assert!(dog.sight_range >= 5, "dog sight (5c0 expected, got {})", dog.sight_range);
        let tnk = rules.actor("2tnk").expect("2tnk not parsed");
        assert!(tnk.sight_range >= 5, "2tnk sight (>=5c0 expected, got {})", tnk.sight_range);
        // Buildings: defenses have RevealsShroud too (Range tied to
        // attack range so the player sees their own field of fire).
        // We only require they parse to a positive value, not the
        // default 5 fallback.
        let tsla = rules.actor("tsla").expect("tsla not parsed");
        assert!(tsla.sight_range > 0, "tsla sight parsed");
    }
}
