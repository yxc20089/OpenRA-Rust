# Phase 8 Status — Specialist Infantry + Versus + Env Components

Branch: `agent/phase-8-specialist-infantry` (off `agent/phase-7-static-defenses`).

## What shipped

- **`openra-sim/src/projectile.rs` (new, ~250 LoC)** — `Projectile` typed
  component with origin / target / integer-fixed-point velocity, per-tick
  `advance(target_pos)`, `apply_versus(damage, armor_class, table)`
  helper, integer `isqrt_i64` for sqrt-free distance compares. 6 unit
  tests.
- **`openra-sim/src/traits/melee.rs` (new, ~115 LoC)** — `MeleeAttack`
  wrapper around `Armament` that clamps range to 1 cell and zeroes
  `projectile_speed` / `splash_radius`. `melee_weapon_for("dog")` →
  `"DogJaw"`. 3 unit tests.
- **`openra-sim/src/world.rs`** — three new fields on `World`:
  `pending_projectiles: BTreeMap<u32, Projectile>`,
  `next_projectile_id`, `typed_components: BTreeMap<u32,
  ActorTypedComponents>`. New `tick_projectiles()` runs after
  `tick_actors()` each inner tick: advances every projectile, resolves
  impacts (sorted-distance for splash, deterministic id-tiebreak for
  ties), applies `Versus` multiplier and integer linear falloff
  (100 % at center → 50 % at radius), credits kills and clears stale
  Activity::Attack pointing at corpses. The data-driven Attack path
  now spawns a `Projectile` when the attacker's primary weapon has
  `projectile_speed > 0`, otherwise applies instant damage with the
  `Versus` multiplier.
- **`openra-data/src/rules.rs`** — `WeaponStats` extended with
  `projectile_speed: WDist`, `splash_radius: WDist`, `versus:
  BTreeMap<String, i32>`; `UnitInfo` extended with `armor_class`,
  `locomotor`, `turret_turn_speed` (all read from the resolved YAML).
  `Default` impl added so existing test fixtures keep compiling with
  `..Default::default()`.
- **`openra-sim/src/gamerules.rs`** — `WeaponStats` mirrors
  `projectile_speed` and `splash_radius`; `parse_projectile_speed` and
  `parse_splash_radius` walk the warhead / projectile child blocks;
  `parse_damage_from_warheads` now also accepts `Warhead@*:
  TargetDamage` (for `DogJaw`). New `parse_wdist_text` helper avoids
  the legacy `parse_range` `* 1024` quirk for fields whose YAML value
  is already in raw fixed-point units (`Speed:`, `Spread:`).
- **`openra-train/src/env.rs`** — `load_rules_with_fallback` now
  returns both `GameRules` (for the simulator) and
  `data_rules::Rules` (typed view). New `attach_typed_components`
  inserts a `Vehicle { locomotor, has_turret, initial_facing }` and
  optional `Turret { facing, turn_speed }` for every wheeled / tracked
  / heavy-tracked actor injected from the scenario YAML, closing the
  Phase 6 carry-forward TODO. Foot infantry is explicitly excluded.

## Versus formula

```
damage_dealt = base_damage × versus[target_armor_class] / 100
```

Lower-cased armor-class lookup, missing class defaults to 100 % (no
modifier), empty class string → `"none"` lookup, negative result clamps
to 0.

## Test results

`cargo test --workspace --exclude openra-data`: **all green** (138 lib
tests + every integration test). Pre-existing
`openra-data::validate_sprites` baseline failure is unchanged.

New Phase-8 integration tests:

- `tests/rocket_projectile_flies.rs` — e3 fires `RedEye`, asserts ≥1
  projectile in flight after the firing tick and zero damage to the
  target until impact.
- `tests/rocket_splash.rs` — direct-hit > west-splash > 0, west-splash
  ≈ 3 350 dmg at 1 cell inside 1.5-cell radius (exact integer math),
  plus an end-to-end world-tick splash variant.
- `tests/dog_melee.rs` — DogJaw is `Projectile: InstantHit` (no
  projectile spawned), kills `e1` in one hit, MeleeAttack helper
  clamps range to 1 cell.
- `tests/versus_damage.rs` — M1Carbine vs Heavy = 10 %, vs None =
  150 %; 25 mm vs Heavy = 48 % (1200 dealt), vs Light = 116 % (2900);
  90 mm with absent class falls back to 100 %.
- `tests/env_attaches_turret.rs` — Vehicle + Turret components
  attached for tank-class actors after `Env::reset()`; foot infantry
  carries no Vehicle component.

Phase 6/7 regression tests (`tank_duel`, `building_takes_damage`,
`building_fires_back`, `building_blocks_path`, `scout_maginot_smoke`)
all pass on the new combat path.

## Deviations / known limits

- **MiniYAML resolver only inherits `^Abstract` parents.** Concrete
  parents (e.g. `RedEye: Inherits: Nike`) are NOT followed, so RedEye
  inherits from `^AntiGroundMissile` (Range 5c0) rather than from
  Nike (Range 7c512). Affects `e3` engagement range only — pre-existing
  Phase 6 behaviour, not introduced here.
- **Medic heal deferred** to Phase 9 (`TODO(P9)` left in
  `melee_weapon_for`). Strategy scenarios are playable without a
  medic.
- **`Projectile.Inaccuracy` not modelled.** RedEye/Dragon both list a
  small inaccuracy (`Inaccuracy: 0` for RedEye, `128` for the parent);
  we always aim at the target's last-known cell center.
- **Splash falloff** uses `f64::sqrt` to compute distance for the
  linear falloff curve. Result rounded to `i32` — deterministic on
  IEEE-754 platforms (x86_64 / aarch64) but not bit-exact with C#'s
  integer `WDist.Length`. Documented in `world.rs::tick_projectiles`.
- **No `parity_rocket.rs` cross-impl test.** Driving prod OpenRA via
  gRPC at port 8033 was out of scope for this branch — the existing
  `parity_combat.rs` flow covers instant-hit weapons; rocket parity
  vs C# can land in Phase 9 alongside transport.

## Files

- `openra-sim/src/{projectile.rs (new), lib.rs, world.rs, gamerules.rs}`
- `openra-sim/src/traits/{melee.rs (new), mod.rs, armament.rs, structure.rs}`
- `openra-sim/src/activities/attack.rs` (test fixture only)
- `openra-sim/tests/{rocket_projectile_flies, rocket_splash, dog_melee,
  versus_damage}.rs` (new) + existing test fixtures updated for
  `..Default::default()`
- `openra-data/src/rules.rs`
- `openra-train/src/env.rs`
- `openra-train/tests/env_attaches_turret.rs` (new)
- `scripts/smoke_strategy_phase8.py` (new)

## What's left (Phase 9, optional)

- `apc` / `jeep` `Cargo: MaxWeight` load/unload orders. Strategy
  scenarios place an APC on `dilemma` and `maginot` but the winning
  policy never has to use it.
- Cross-impl rocket parity test (`parity_rocket.rs`).
- Concrete-parent inheritance in the MiniYAML resolver (so RedEye
  inherits Nike's 7c512 range). Cosmetic / tactical depth, not
  required for any of the four scout scenarios to play.
