//! Compiled game rules — transforms parsed YAML Ruleset into fast lookup structs.
//!
//! Bridges the gap between `openra_data::rules::Ruleset` (parsed MiniYAML)
//! and the simulation's runtime needs (costs, stats, weapons).

use std::collections::BTreeMap;
use std::path::PathBuf;
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

            // Ground units carry `Mobile.Speed`; aircraft carry
            // `Aircraft.Speed` (they never have a Mobile trait). Read both
            // so heli/hind/mig/yak pick up their real cruise speed from
            // the vendored YAML instead of falling to 0 (which would
            // freeze every aircraft on the map).
            let speed = info.trait_info("Mobile")
                .and_then(|t| t.get_i32("Speed"))
                .or_else(|| info.trait_info("Aircraft").and_then(|t| t.get_i32("Speed")))
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

        // Bench-friendly alias: the C# RA YAML registers the Allied
        // commando hero as `E7`, but every bench scenario (and the
        // `world.rs::order_c4_detonate` + `env.rs` C4 validator) keys
        // the actor by the canonical name `tanya`. Register `tanya` as
        // a clone of `e7` in the ruleset path so scenarios using
        // `type: tanya` resolve to the real E7 stats (HP 10000, Colt45
        // weapon) instead of falling back to the no-stats default
        // (max_hp=50000, weapons=[], → 100-dps "default" weapon — the
        // root cause of `combat-tanya-vs-rush` being unsolvable).
        // Without this alias, `world.rules.actor("tanya")` returns
        // None in production (since only `defaults()` registers
        // "tanya"), and the scenario actor is spawned weaponless.
        if !actors.contains_key("tanya") {
            if let Some(e7) = actors.get("e7").cloned() {
                actors.insert("tanya".to_string(), e7);
            }
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

    /// Load GameRules from the canonical RA YAML data. By default this
    /// reads the YAML bytes embedded inside the binary (originally
    /// seeded from OpenRA `0938a27` — see
    /// `openra-data/src/embedded.rs`); callers can override this by
    /// setting `OPENRA_VENDOR_DIR` to point at an alternative vendor
    /// snapshot for testing.
    ///
    /// Override search order (only consulted if `OPENRA_VENDOR_DIR` is
    /// set, or the in-tree vendor checkout still exists for tests):
    /// 1. `OPENRA_VENDOR_DIR` env var.
    /// 2. `$CARGO_MANIFEST_DIR/../vendor/OpenRA/mods/ra` (legacy in-tree
    ///    checkout — present in older clones, ignored when absent).
    /// 3. `$HOME/Projects/OpenRA-Rust/vendor/OpenRA/mods/ra`.
    /// 4. `$HOME/workspace/OpenRA-Rust/vendor/OpenRA/mods/ra`.
    ///
    /// If nothing in the override chain matches, the embedded data is
    /// used. This function therefore CANNOT FAIL — the previous panic
    /// path (and the `try_from_vendor` Result return) was kept for API
    /// compatibility but only fires when an EXPLICITLY-specified
    /// `OPENRA_VENDOR_DIR` is broken.
    pub fn from_vendor() -> Self {
        match Self::try_from_vendor() {
            Ok(r) => r,
            Err(e) => panic!(
                "GameRules::from_vendor: explicit override failed: {e}\n\
                 (Unset OPENRA_VENDOR_DIR to fall back to the embedded \
                  RA data baked into the binary.)"
            ),
        }
    }

    /// Fallible vendor loader. Returns the embedded ruleset when no
    /// override is configured; honours `OPENRA_VENDOR_DIR` (and the
    /// legacy in-tree paths) when present. Returns `Err` only when an
    /// explicit override is set but unreadable.
    pub fn try_from_vendor() -> Result<Self, String> {
        // Explicit override — if the env var is set, it MUST resolve to
        // a parseable ruleset. We don't silently fall back to embedded
        // for a broken override, because that would hide configuration
        // bugs from power users intentionally pointing at a different
        // snapshot.
        if let Ok(p) = std::env::var("OPENRA_VENDOR_DIR") {
            let path = PathBuf::from(&p);
            if !path.exists() {
                return Err(format!("OPENRA_VENDOR_DIR points at non-existent path: {p}"));
            }
            return openra_data::rules::load_ruleset(&path)
                .map(|rs| Self::from_ruleset(&rs))
                .map_err(|e| format!("OPENRA_VENDOR_DIR {p}: parse failed: {e}"));
        }

        // Implicit legacy in-tree vendor — if a clone happens to still
        // have `vendor/OpenRA/mods/ra/` populated, honour it. This keeps
        // the parity / determinism tests in `openra-sim/tests/` that
        // build their own ruleset directly via `load_ruleset(mod_dir)`
        // working even before they're switched over to the embedded
        // loader. Absence is NOT an error — fall through to embedded.
        let mut candidates: Vec<PathBuf> = Vec::new();
        let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        candidates.push(crate_dir.join("../vendor/OpenRA/mods/ra"));
        if let Ok(home) = std::env::var("HOME") {
            candidates.push(PathBuf::from(&home).join("Projects/OpenRA-Rust/vendor/OpenRA/mods/ra"));
            candidates.push(PathBuf::from(&home).join("workspace/OpenRA-Rust/vendor/OpenRA/mods/ra"));
        }
        for c in &candidates {
            if c.exists()
                && let Ok(rs) = openra_data::rules::load_ruleset(c)
            {
                return Ok(Self::from_ruleset(&rs));
            }
        }

        // Default path — embedded RA YAML baked into the binary at build
        // time. Cannot fail (parse errors would surface as compile-time
        // panics inside the unit-test for `load_ruleset_embedded`).
        let rs = openra_data::embedded::load_ruleset_embedded();
        Ok(Self::from_ruleset(&rs))
    }

    /// Test-friendly cached vendor loader. The first call parses the
    /// vendor YAML; subsequent calls clone from a process-wide cache.
    /// Tests that build many worlds (e.g. the parity / determinism
    /// sweeps) use this to avoid re-parsing the ruleset thousands of
    /// times.
    pub fn vendor_cached() -> Self {
        use std::sync::OnceLock;
        static CACHE: OnceLock<GameRules> = OnceLock::new();
        CACHE.get_or_init(GameRules::from_vendor).clone()
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
    fn vendor_rules_have_all_common_units() {
        let rules = GameRules::vendor_cached();
        // Buildings — costs match vendor RA YAML.
        assert_eq!(rules.cost("powr"), 300);
        assert_eq!(rules.cost("weap"), 2000);
        assert_eq!(rules.cost("proc"), 1400);
        // Infantry
        assert_eq!(rules.cost("e1"), 100);
        assert_eq!(rules.cost("e3"), 300);
        // Vehicles
        assert_eq!(rules.cost("2tnk"), 850);
        assert_eq!(rules.cost("harv"), 1100);
        assert_eq!(rules.cost("mcv"), 2000);
        // Check stats — mcv HP / speed mirror vendor.
        let mcv = rules.actor("mcv").unwrap();
        assert_eq!(mcv.hp, 60000);
        assert_eq!(mcv.speed, 60);
        assert!(matches!(mcv.kind, ActorKind::Mcv | ActorKind::Vehicle));
        // Buildings — vendor fact carries a 3×4 dimension trait
        // (`Building.Dimensions: 3,4` in `structures.yaml`).
        let fact = rules.actor("fact").unwrap();
        assert!(fact.is_building);
        assert_eq!(fact.footprint, (3, 4));
        assert_eq!(fact.hp, 150000);
        assert_eq!(rules.cost("fact"), 2000);
        // Power — generators are positive, drainers are negative.
        assert_eq!(rules.actor("powr").unwrap().power, 100);
        assert_eq!(rules.actor("apwr").unwrap().power, 200);
        assert!(rules.actor("tsla").unwrap().power < 0);
    }

    #[test]
    fn is_unit_vs_building() {
        let rules = GameRules::vendor_cached();
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
