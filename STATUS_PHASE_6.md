# Phase 6 Status — Vehicles + Turrets

Branch: `agent/phase-6-vehicles-turrets` (off `feat/rust-sim-integration`).

## What shipped

- **`openra-sim/src/traits/turret.rs` (new, 250 LoC)** — `Turret` typed
  component with independent yaw, `tick(desired)` integer rotation matching
  C# `Util.TickFacing`, `yaw_between(from, to)` helper, configurable
  `aim_tolerance`. 9 unit tests.
- **`openra-sim/src/traits/vehicle.rs` (new, 110 LoC)** — `Locomotor` enum
  (`Foot`/`Wheeled`/`Tracked`/`HeavyTracked`/`Naval`/`Aircraft`) parsed from
  YAML strings with safe fallback; `Vehicle` typed component flagging
  `has_turret`. 3 unit tests.
- **`openra-sim/src/traits/armament.rs` extended** — added `MultiArmament`
  + `NamedArmament` for actors with primary/secondary weapons (e3, 4tnk).
  `select_for_target` picks first ready in-range armament, falling back to
  any in-range one. 6 new unit tests.
- **`openra-sim/src/gamerules.rs::parse_damage_from_warheads`** — fixes the
  long-standing weapon parser bug: damage now correctly comes from
  `Warhead@*: SpreadDamage → Damage`, not the empty top-level `Damage:`.
  Added an integration test that loads `vendor/OpenRA/mods/ra/weapons/`
  and asserts 25mm=2500, 90mm=4000, 105mm=4000 burst=2, TurretGun=6000.
- **`openra-sim/tests/tank_duel.rs` (new)** — Phase 6 acceptance test.
  Spawns a 2tnk vs 1tnk arena with real ruleset-loaded weapons; the 2tnk
  kills the 1tnk in 86 outer ticks (≈84 expected, from
  `(1 + 5×reload_delay) / NetFrameInterval=3`).
- **`scripts/smoke_strategy_phase6.py` (new)** — drives `cargo test` for the
  tank duel + Phase 6 unit tests as a single Python entrypoint.
- **`PLAN_STRATEGY_SCENARIOS.md` (new)** — full scope analysis for all four
  strategy scenarios, phase 6→9 roadmap with acceptance criteria.

## Test results

```
$ cargo test --workspace --exclude openra-data
total: 156 passed, 0 failed
```

The one pre-existing failure (`openra-data::validate_sprites`) is in the
baseline branch and unrelated (SHP decoder for 6 sprites returning 0 frames).

```
$ python scripts/smoke_strategy_phase6.py
PASS: tank_duel
PASS: traits::turret
PASS: traits::vehicle
PASS: traits::armament
PASS: weapon damage from warheads
PASS: openra-sim full lib (120 tests)
Phase 6 smoke OK
```

## Deviations from spec

- **`OpenRAEnv` not yet wired to attach `Turret`/`Vehicle` traits at spawn.**
  The env still uses `GameRules::defaults()` which lacks per-weapon Versus
  multipliers and uses a single fallback weapon. The tank-duel parity test
  instead exercises the full chain via `World` directly, with real
  `GameRules::from_ruleset(load_ruleset(vendor/...))`. Wiring the env to
  load real rules + attach typed components is mechanical follow-up;
  separated to avoid changing rush-hour behaviour mid-sprint.
- **Versus armor multipliers** are parsed (`parse_versus`) but not applied
  in the world's combat tick. Tank-duel test bounds [60..=110] outer ticks
  span both with and without the 115% Heavy multiplier. Apply in Phase 8.
- **Turret rotation is animation-only.** I did not gate firing on turret
  facing alignment. C# `AttackTurreted` would refuse to fire until the
  turret has aimed; we accept the simpler "fire instantly when in range"
  semantics and document the assumption in `turret.rs`.
- **`yaw_between` uses `f64::atan2`** rather than C#'s integer
  `WAngle.ArcTan` polynomial. Documented in code; deterministic on
  IEEE-754 platforms; only used for visual aim, not damage / sync.

## What's left for Phases 7-9

- **Phase 7 (static defenses)**: `gun`, `tsla`, `pbox` need Armament +
  `AttackTurreted`/`AttackTesla` so they can fire back. Required to load
  any of the four scout scenarios — they all place enemy `gun` and `tsla`.
- **Phase 8 (specialist infantry)**: `e3` Dragon/RedEye splash, `dog`
  melee leap, Versus armor multipliers in damage application. After this
  all four strategy scenarios can be played end-to-end.
- **Phase 9 (transport)**: `apc`/`jeep` `Cargo` load/unload. Optional —
  the human-readable strategy scenario descriptions never *require* an
  APC; only `dilemma` and `maginot` even spawn one.

`TODO(P6)` markers seeded in `armament.rs::MultiArmament` (no Versus
weighting in `select_for_target` yet) and the env loader (it doesn't
attach `Turret`/`Vehicle` typed components yet — fine for Phase 6 because
combat path is data-driven via `Activity::Attack`).

## Files

- `openra-sim/src/traits/{turret.rs,vehicle.rs,armament.rs,mod.rs}`
- `openra-sim/src/gamerules.rs`
- `openra-sim/tests/tank_duel.rs`
- `scripts/smoke_strategy_phase6.py`
- `PLAN_STRATEGY_SCENARIOS.md`
