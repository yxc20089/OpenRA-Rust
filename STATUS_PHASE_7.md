# Phase 7 Status — Static Defenses

Branch: `agent/phase-7-static-defenses` (off `feat/rust-sim-integration`).

## What shipped

- **`traits/structure.rs` (new, 184 LoC)** — `Structure` component
  (footprint, optional `Armament`, `must_be_destroyed`) and
  `classify_defense`: `gun/pbox/hbox/ftur` → `GroundTurret`, `tsla` →
  `Tesla`, `agun/sam` → `AntiAirOnly`, `gap` → `InertWeapon`.
- **`world.rs` auto-target loop** — idle armed buildings scan
  BTreeMap-ordered actors for nearest in-range hostile (Chebyshev,
  ties → lowest id). Damage / range / reload pulled from
  `rules.actor.weapons[0] → rules.weapon`. Static attackers don't
  chase: out-of-range or dead-target activities clear so auto-target
  re-picks.
- **Pathfinder** — building spawn / removal occupy / clear the full
  `footprint_w × footprint_h` rectangle. A* treats buildings as solid.
- **`openra-data::BuildingInfo`** — typed view (hp, footprint from
  `Building.Dimensions`, primary weapon from `Armament[@PRIMARY]`,
  `MustBeDestroyed`). `Rules.buildings` populated alongside `units`.
- **`oramap.rs`** — strategy scenarios now load: `base_map_ref`
  resolves against sibling `../maps/`; block-form actor lists (indented
  `- `); quoted `type:` scalars; inline `position: [x, y]`; agent
  spawn-point filter skipped when no agent declares one.
- **`env.rs`** — loads vendored RA ruleset so `TurretGun`, `TeslaZap`,
  `M60mg` resolve. `build_scenario_actor` produces `ActorKind::Building`
  for `is_building` actors. New `obs["enemy_buildings_summary"]`
  (cell_x, cell_y, id, type, hp_pct), fog-filtered. Done-check
  requires zero combat units AND zero buildings on a side.

## Test results

`cargo test --workspace`: all green except pre-existing
`validate_sprites` baseline failure. Phase 7 tests:
`traits::structure::*` (4); `building_takes_damage` (pbox dies to 2tnk);
`building_fires_back` (gun auto-targets a tank); `building_blocks_path`
(A* routes around a pbox); `scout_maginot_smoke` (loads real yaml,
parses ≥7 enemy buildings, 50 `Observe` ticks); `tank_duel` (Phase 6
regression).

## Deviations

- **AA-only stubs (`sam`, `agun`) never fire** — no aircraft. Strategy
  scenarios don't place either.
- **No projectile flight** — instant damage on fire frame (Phase 3
  carry-forward). Tank-duel timing still matches; reload dominates.
- **No `Versus` armor multipliers applied**. Parsed but not used.
  `gun → 2tnk` would otherwise scale by Heavy 115%.
- **Tesla three-charge salvo approximated** as plain reload-gated shot.
- **Env still does NOT attach `Vehicle` / `Turret` typed components**
  (Phase 6 carry-forward). Vehicles spawn with `BodyOrientation +
  Mobile + Health` only. Combat is data-driven through
  `Activity::Attack` and doesn't yet read the typed components, so
  this stays open into Phase 8.
- **`MustBeDestroyed` treats all buildings as counting**
  (`building_must_be_destroyed` always `true`). C# distinguishes
  `fact`/`proc` from `powr`/`barr`.

## Cross-impl smoke

`scripts/smoke_strategy_phase7.py` drives `OpenRAEnv` on
`scout-maginot.yaml` (seed 42) through PyO3: asserts
`enemy_buildings_summary` exists, runs 100 `Observe` ticks, prints
per-type building counts + mean HP%. Skips gracefully when the
`openra_train` wheel is missing or pre-dates Phase 7 (rebuild via
`maturin develop --release`).

## What's left

**Phase 8 (specialist infantry)**: `e3` rocket (`RedEye` / `Dragon`
warhead parsing, splash via `Spread`, multi-armament selection); `dog`
melee (`DogJaw` 1c0 leap-then-strike); `medi` heal (skip v1, sits
idle); apply `Versus` multipliers; wire `Vehicle` / `Turret` components
in env loader.

**Phase 9 (APC transport)**: `Cargo: MaxWeight` load/unload for `apc`
(5) and `jeep` (1). Optional — strategy descriptions never require
transports, though maps place them.

## Files touched

- `openra-sim/src/traits/{structure.rs (new), mod.rs}`
- `openra-sim/src/world.rs`
- `openra-sim/tests/{building_takes_damage, building_fires_back, building_blocks_path}.rs` (new)
- `openra-data/src/{oramap.rs, rules.rs}`
- `openra-train/src/{env.rs, observation.rs}`
- `openra-train/tests/scout_maginot_smoke.rs` (new)
- `scripts/smoke_strategy_phase7.py` (new)
