# Strategy Scenarios — Plan & Phase Roadmap

**Goal**: extend the OpenRA-Rust deterministic simulator so all four strategy
scenarios under `OpenRA-RL-Training/scenarios/strategy/` can be played
end-to-end through `OpenRAEnv` with the same fidelity as the C# game-server.
This plan inventories every actor / weapon / mechanic the scenarios reference
and breaks the work into shippable phases.

Branch base: `feat/rust-sim-integration` (the rush-hour sprint).

## Scope inventory (read from the four scenario YAMLs)

| Scenario | Tools used | Map | Termination |
|---|---|---|---|
| `scout-twobody` | move/attack/stop/observe | `singles-twobody.oramap` | 15000 ticks, kill-all |
| `scout-maginot` | move/attack/stop/observe | `singles-maginot.oramap` | 12000 ticks, kill-all |
| `scout-dilemma` | move/attack/stop/observe | `singles-dilemma.oramap` | 12000 ticks, kill-all |
| `scout-gauntlet` | move/attack/stop/observe | `singles-gauntlet.oramap` | 15000 ticks, kill-all |

### Distinct actor types referenced across all four scenarios

**Agent infantry**: `e1` (rifleman), `e3` (rocket), `medi` (medic), `dog` (melee)
**Agent vehicles**: `1tnk` (Light Tank), `2tnk` (Medium Tank), `3tnk` (Heavy Tank, in dilemma/twobody only via comment), `jeep` (Jeep), `apc` (APC, dilemma+maginot only)
**Enemy infantry**: `e1`, `e3`
**Enemy vehicles**: `harv` (Harvester — placed but `stance: 0` neutral, used as a discoverable resource marker)
**Enemy buildings**:
- `gun` — Allied Turret (1×1, anti-armor turret weapon `TurretGun`, range 6c512, damage 6000 vs heavy 115%)
- `tsla` — Tesla Coil (1×1, weapon `TeslaZap`, instant beam)
- `fact` — Construction Yard (3×4, MustBeDestroyed)
- `proc` — Refinery (3×3, MustBeDestroyed)
- `powr` — Power Plant (2×2, scenery)
- `barr` — Barracks (2×2, scenery)

### Required weapons (from referenced unit Armaments)

| Weapon | Mounted on | Range | Damage | Reload | Notes |
|---|---|---|---|---|---|
| `M1Carbine` | e1 | 5c0 | 1500 (LightMG) | 20 | Already used by rush-hour |
| `M60mg` | jeep, apc | 5c512 | LightMG-like | small | Vehicle MG |
| `25mm` | 1tnk | 4c768 | 2500 (Cannon vs heavy) | 21 | Tank cannon |
| `90mm` | 2tnk | 4c768 | 4000 (vs heavy 115%) | 50 | Tank cannon |
| `105mm` | 3tnk | 4c768 | 4000 burst-2 | 70 | Tank cannon |
| `RedEye` | e3 | (missile) | rocket warhead, splash | ~ | E3 anti-air rocket |
| `Dragon` | e3 | (missile) | rocket warhead, splash | ~ | E3 anti-armor rocket |
| `DogJaw` | dog | melee 1c0 | 4000 | 80 | Leap-attack |
| `Heal` | medi | 1c0 | -ve damage (heal) | small | Friendly target, allied relationship |
| `TurretGun` | gun | 6c512 | 6000 cannon | 30 | Static turret |
| `TeslaZap` | tsla | ~6c0 | huge electric, splash | 120 | Three-charge AttackTesla |

### Game mechanics not yet supported

1. **Turret + hull** — vehicles have an independent turret facing (`Turreted: TurnSpeed: ...`) plus hull facing. Required for tank visuals (pure damage parity does not strictly need it, but combat range checks pass through `LocalOffset` muzzles which need a turret position).
2. **Multi-armament** — e3 has primary (RedEye) + secondary (Dragon). Some vehicles also carry secondaries (4tnk). Requires a list-of-armaments component.
3. **Versus armor multiplier** — most weapons have `Versus: { Heavy: 115, Light: 75, ... }`. Damage = base × versus%/100. Already partially parsed in `gamerules.rs::parse_versus`, but **damage itself is NOT currently parsed correctly** (it lives inside `Warhead@1Dam: SpreadDamage` not at top level). Needs a fix.
4. **Burst** — `105mm` has `Burst: 2`. World already has a `burst` field; weapon loader doesn't fill it from warhead.
5. **Splash damage** — RedEye/Dragon have `Spread: 128+`. v3.
6. **Static buildings as combatants** — `gun`, `tsla` need Armament + AttackTurreted/AttackTesla. Currently buildings have no Armament in the world.
7. **Locomotor terrain costs** — `tracked`, `wheeled`, `foot`. Currently pathfinder uses a single cost map.
8. **APC transport** — load/unload (`Cargo: MaxWeight: 5`). Used by maginot+dilemma but agent strategy can ignore the APC entirely.
9. **Healing** — medic Armament with negative damage / target=Ally. Out of scope for combat parity.
10. **AttackTesla** — three-charge salvo, then long reload. v3.

## Phase breakdown

### Phase 6 — Vehicle + Turret support **(THIS AGENT)**

**Files to add/extend**:
- `openra-sim/src/traits/vehicle.rs` (new) — typed `Vehicle` component with locomotor variant + chassis facing.
- `openra-sim/src/traits/turret.rs` (new) — typed `Turret` component with independent facing, target-yaw helper, `tick(turn_speed, target_facing)`.
- `openra-sim/src/traits/armament.rs` — extend to a `MultiArmament` (list of Armament instances, named, with optional turret link). Keep single-weapon constructor for backward compat.
- `openra-sim/src/gamerules.rs::WeaponStats::from_weapon_info` — fix damage parsing to read from `Warhead@*: SpreadDamage → Damage`. Backfill `burst` from weapon top level.
- `openra-sim/src/world.rs` — when spawning a vehicle, attach `Mobile + Turret` traits; combat path uses `weapon_damage` already loaded from rules so the parser fix carries through.
- `openra-train/src/env.rs::build_scenario_actor` — for vehicle kinds, attach a `Turret` typed component.

**Acceptance criteria**:
- [x] `cargo test --workspace --exclude openra-data` green (156/156 passed; the
      single pre-existing `openra-data::validate_sprites` failure is unrelated
      to Phase 6, see baseline in commit 1ed0592).
- [x] New unit tests:
  - [x] `traits::turret` — 9 tests: facing turn, in-tolerance check, ticked
        rotation, wraparound, yaw_between cardinal directions.
  - [x] `traits::vehicle` — 3 tests: locomotor parse, fallback, ground check.
  - [x] `traits::armament` — multi-armament `select_for_target` (in-range,
        prefers ready, falls back to in-range when none ready, max range,
        per-tick decrement). 5 new tests.
  - [x] `gamerules::weapon_damage_from_warhead_for_real_yaml` — loads 25mm /
        90mm / 105mm / TurretGun from `vendor/OpenRA/.../ballistics.yaml`
        and asserts the documented damage values.
- [x] `tank_duel.rs` integration test: 2tnk vs 1tnk arena spawns, 2tnk fires
      `90mm`, 1tnk dies in 86 outer ticks (analytical estimate ≈84 outer ticks
      = `(1 + 5×reload) / NetFrameInterval`).
- [x] Python smoke test `scripts/smoke_strategy_phase6.py`: passes (drives
      `cargo test` for the tank duel + Phase 6 unit tests).
- [x] Determinism: existing rush-hour `env_determinism` tests still pass
      (no changes to the rush-hour code path; weapon parser fix is additive
      and the rush-hour env still uses default rules).

### Phase 7 — Static defenses + buildings as combatants

- Extend Armament-bearing actors to include buildings (`gun`, `tsla`, `pbox`).
- Implement `AttackTurreted` for static turrets (passive auto-target within range, no chasing).
- Implement `AttackTesla` (three-charge salvo, 120-tick reload).
- Add `MustBeDestroyed` trait → world end-of-tick checks the kill-all condition for `fact` + `proc` only (not `powr` / `barr`).
- Place buildings via map yaml, no production queue logic needed.
- Acceptance: scenario `scout-maginot` loads, tanks fire on the `gun` defense, `gun` fires back, both can die.

### Phase 8 — Specialized infantry weapons

- e3 → RedEye/Dragon: implement projectile travel time approximation (instant impact still ok for v1, but apply armor versus correctly), splash damage warhead (single ring for simplicity).
- dog → DogJaw: melee range `1c0`, leap = small move-then-strike.
- medi → Heal: skip for v1 (medi can sit and do nothing).
- e3 secondary armament selection: pick primary when target is ground armor heavy, otherwise primary anyway. Keep simple.

### Phase 9 — Transport (only if a scenario actually needs it)

- APC `Cargo: MaxWeight: 5` load/unload orders.
- Jeep `MaxWeight: 1`. Skipped unless the agent's policies call `enter_transport`/`unload`.
- `scout-maginot` and `scout-dilemma` reference an APC but the *winning* policy never has to use it (the human-readable description never mentions transport). So we can ship Phase 9 last and gate the two scenarios on Phase 8 completion.

## Out of scope (v3 / never)

- Aircraft (yak, mig, hind, heli) — not used by any strategy scenario.
- Naval (lst, ss, dd, …).
- AI opponent (`ai.rs` already exists for rush-hour-style passive enemies; strategy scenarios use stationary `stance: 2` enemies which is supported).
- Production queues, conyards constructing stuff, MCV deploy.
- Ore harvesting beyond what rush-hour already has.

## Mapping back to scenarios

| Scenario | Min phase to play it |
|---|---|
| `scout-twobody` | Phase 8 (needs e3 rockets to break top gap, gun/tsla to hold defenses). |
| `scout-maginot` | Phase 8 (e3 + tsla + gun in defense lines). APC optional. |
| `scout-dilemma` | Phase 8 (same set). |
| `scout-gauntlet` | Phase 8 (same set + APC optional). |

So Phases 6 → 7 → 8 in order unblocks **all four** strategy scenarios. Phase 9
is a stretch goal needed only if we want the agent to learn transports.

## Determinism rules (carry over from rush-hour)

- All RNG goes through `MersenneTwister`.
- `BTreeMap` not `HashMap` for any ordered iteration that touches outputs / hashes.
- Fixed-point math everywhere (`WPos`, `WVec`, `WAngle`, `WDist`, `CPos`).
- Sync hash must remain valid (turret facing is NOT in the C# `[VerifySync]` set
  for `Turreted`; verified by reading C# `Turreted.cs` — only `localFacing` may
  be synced and it XORs into BodyOrientation; we'll mirror that).
