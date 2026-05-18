# OpenRA-Rust → Full C# Parity Plan

Goal: make the Rust engine (`openra-sim` + `openra-train`) a faithful
reimplementation of the C# RA engine as exposed by
`openra-rl/proto/rl_bridge.proto` — **all 22 actions, full observation,
economy, tech tree, buildings, sabotage, every unit type** — so the
OpenRA-Bench eval and Training run on Rust alone (C# dropped).

Parity is defined by the proto + the C# handlers
(`ExternalBotBridge.cs`, `ActionHandler.cs`, `ObservationSerializer.cs`).
Every subsystem is TDD'd against concrete C# values (the rules data is
the same RA YAML the C# loads).

## Strict dependency order

Nothing downstream is correct until the layer under it is. Build in
this order; each layer ships with tests before the next starts.

```
F  Foundation
   F1 rules/data: parse cost, build_time, power, prerequisites,
      footprint, queue/produces, versus armor, locomotor terrain
      costs, sellable/refund, repair  (openra-data/rules.rs,
      openra-sim/gamerules.rs)
   F2 generic World::spawn_unit(type, owner, cell)  (world.rs)
   F3 order plumbing: Command variants + env.build_orders +
      world.process_frame dispatch skeleton for all 22 actions
S  Subsystems (each TDD vs C#)
   S1 Economy   : ResourceLayer (ore/gem density), harvester loop,
      refinery→cash, PlayerResources; action HARVEST
   S2 Power     : PowerManager aggregation, low-power build modifier;
      action POWER_DOWN
   S3 Production: tick_production (cost/time/power), queues;
      actions BUILD, TRAIN, CANCEL_PRODUCTION; available_production
   S4 Build/Struct: footprint placement, construction HP ramp;
      actions PLACE_BUILDING, DEPLOY (MCV→cyard), SELL, REPAIR
   S5 Tech tree : ProvidesPrerequisite, queue enable/disable, gating
   S6 Units     : all infantry/vehicle/tank/arty/air/ship/mcv/harv/
      engineer/spy/thief/medic/tanya; multi-armament, burst, versus,
      turret-vs-hull, locomotor terrain, melee, AA
   S7 Commands  : ATTACK_MOVE, STOP, GUARD, SET_STANCE,
      SET_RALLY_POINT, SET_PRIMARY, ENTER_TRANSPORT/UNLOAD (cargo),
      SURRENDER, PATROL
   S8 Sabotage/special: engineer capture/repair, spy infiltrate+
      sabotage+disguise, thief steal cash, C4/demolition, dog detects
      spy, MAD/Chrono/Iron-Curtain, superweapons (nuke, paras…)
   S9 Observation parity: full RlEconomy/RlMilitary/RlUnitInfo/
      RlBuildingInfo/RlProductionInfo/RlMapInfo/RlKillEvent, 9-channel
      spatial tensor, done/result, fast-advance + interrupt signals,
      kill_events drain
   S10 Map      : generic .oramap binary terrain + arbitrary scenario
      actor loading (unblocks custom maps for the bench)
```

S9 is incremental: every subsystem adds its own observation fields as
it lands (don't defer all serialization to the end).

## Parity spec anchors (from code, not assumed)

- Actions: 22 enum values, field usage per action — `ActionHandler.cs`.
  PATROL defined but unimplemented in C# (parity = no-op/dropped).
  FAST_ADVANCE handled in bridge, not an order.
- Observation: spatial tensor = H×W×**9** float32, row-major
  channels-last; ch0 terrain, 1 height, 2 resource density, 3
  passability{0,1}, 4 fog{0,0.5,1}, 5 own bldg, 6 own unit density,
  7 enemy bldg, 8 enemy unit density (`ObservationSerializer.cs`).
- Economy: cost=`ValuedInfo.Cost`; build_time=`BuildableInfo.BuildDuration`
  (−1⇒use cost) × actor + queue modifiers; low-power ⇒ ×LowPowerModifier.
  Power = per-building `Power.Amount` (+provide/−drain); player aggregates.
- Prerequisites: `BuildableInfo.Prerequisites` (`~` hidden, `!` inverted),
  granted by `ProvidesPrerequisite` on buildings; enforced by TechTree.
- Kill events drained every advance; interrupt signals: game_over,
  enemy_spotted, unit_destroyed, under_attack, building_discovered,
  enemy_building_destroyed, own_building_destroyed, unit_arrived,
  production_complete, exploration_milestone.

## Extension points (from architecture map)

- New action: `openra-train/src/command.rs` (enum + PyCommand) →
  `env.rs::build_orders()` (→GameOrder) →
  `world.rs::process_frame()` (dispatch) → Activity in
  `actor.rs`/`activities/` + tick in `tick_actors()` or new subsystem fn.
- New obs field: `observation.rs::Observation` → `env.rs::observation()`
  → `to_pydict()`; update deterministic hash if load-bearing.
- New system: parse in `openra-data/rules.rs`, expose via
  `openra-sim/gamerules.rs`, add `TraitState` variant +
  `sync_hash()` if state is sync-critical, tick in `process_frame()`.
- Determinism: every sync-critical field must enter `SyncHash`; tests
  assert tick-exact behavior vs C# reference numbers.

## Test strategy

- `openra-data/tests`: rules values vs known C# (E1 hp 5000, costs,
  build times, power, prerequisites, footprints, versus).
- `openra-sim/tests`: per-subsystem integration (build empty world,
  insert actors, drive orders, assert tick-exact outcomes).
- `openra-train/tests`: `Env` step/obs parity per action.
- Determinism gate: same (scenario, seed, orders) ⇒ identical SyncHash.

## Honest scoping

Full RA parity incl. spies/sabotage/superweapons is a large,
multi-iteration effort. Order above is value-and-dependency optimal:
F→S5 unlocks economy/production/tech (the bench's missing scenario
families); S6–S8 broaden unit/ability coverage; S10 unlocks arbitrary
maps. Progress is shipped per-subsystem with green tests, not big-bang.

## Progress log (per-subsystem, green-committed)

- **S7 — FAITHFUL HoldFire** (task #10) ✅
  C# AttackBase/AutoTarget parity. `Activity::Attack` now carries an
  `auto_acquired` flag (true = idle auto-engage / defensive-building
  scan; false = explicit agent/player "Attack" order). Three gates:
  (1) `order_attack` refuses an auto-acquired engagement under
  HoldFire(0); (2) every `tick_actors` an *abandonment* pass drops any
  auto-acquired Attack whose owner is now on HoldFire (handles the env
  reset-warmup frame that auto-engages before the agent can SET_STANCE
  — the documented root cause); (3) defensive-building auto-scan
  skips HoldFire owners. Explicit `attack_unit` always overrides
  stance (player intent wins). Scenario YAML per-actor `stance:` is
  now parsed (was discarded) and applied to the world before the
  warmup frame via `world::set_actor_stance`.
  Integ test: `openra-train/tests/env_holdfire.rs` — HoldFire deals
  ZERO damage to an adjacent enemy over 200 steps; a no-stance control
  DOES damage it; explicit attack_unit under HoldFire STILL attacks.
  Honest gap: faithful ReturnFire(1) retaliation is not asserted by a
  dedicated test (documented in the test header; ReturnFire/Defend/
  AttackAnything behaviour otherwise unchanged).

- **S7 — GUARD** (task #10 cont.) ✅
  C# `Guard`/`GuardActivity` follow subset. New
  `Activity::Guard{target_id,leash,speed}`; the guard steps one
  cell/tick toward a cell adjacent to the guarded actor whenever the
  Chebyshev gap exceeds `leash` (2), and goes idle if the guarded
  actor disappears. Command + Python shim + env order plumbing with
  target/ownership validation. Bench: `guard` tool schema +
  `_to_commands` dispatch, congruence test kept 1:1 (wildcard count
  17→18). Integ test `openra-train/tests/env_guard.rs`.
  Honest gap: C# Guard layers AttackFollow (re-engages the guarded
  actor's attackers) on top — not asserted; opportunistic combat
  falls back to normal stance auto-engage (documented in test header).

- **S7 — SET_PRIMARY** (task #10 cont.) ✅
  C# `PrimaryBuilding`. World gains `primary_buildings: HashSet<u32>`;
  `set_primary_building` enforces one primary per (owner, type)
  (designating clears same-type siblings). `find_spawn_location` and
  `find_rally_point_for_unit` now sort candidates primary-first so
  produced units spawn from / rally through the primary. New
  observation field `OwnBuilding.is_primary` (struct + SyncHash +
  PyDict). Command + Python shim + env order plumbing
  (ownership-validated). Bench: `set_primary` tool schema +
  units-only `_to_commands` dispatch, congruence 1:1 (wildcard count
  18→19). Integ test `openra-train/tests/env_set_primary.rs`: with
  two barracks, SET_PRIMARY on the far one routes the next produced
  E1 there and sets the flag; switching clears the old primary;
  non-owned id warns.
  Honest gap: C# also routes the production EXIT cell through the
  primary — we assert spawn-building preference + the flag only.
