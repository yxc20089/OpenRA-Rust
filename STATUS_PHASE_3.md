# Phase 3 status — Combat + Shroud

Branch: `agent/phase-3-combat-shroud` (off `main`, with
`agent/phase-1-2-foundation` and `agent/phase-4-yaml-rules` merged in
clean — no conflicts).

Commits: `85cc789` (core), `6491dd1` (parity harness), plus inherited
phase 1/2/4 commits.

## What shipped

- **`openra-sim/src/traits/armament.rs`** — typed `Armament` carrying
  a cloned `openra_data::rules::WeaponStats` plus a per-actor
  `current_cooldown_ticks`. `tick()` decrements (saturating);
  `is_ready()` / `mark_fired()` gate firing.
- **`openra-sim/src/traits/shroud.rs`** — per-player `Shroud` with
  separate `visible[]` (this-tick) and `explored[]` (sticky) bool
  grids sized `map.width × map.height`. `update_from_actors()`
  recomputes from each own actor's `RevealsShroud.Range`.
  `wdist_to_cell_radius()` does inclusive rounding (`(d + 1023) / 1024`).
- **`openra-sim/src/activities/attack.rs`** — `AttackActivity{target,
  armament}`: dead/missing → Done; out of range → push `MoveActivity`
  child; in-range + cooldown 0 → mutate `TraitState::Health` and
  `mark_fired`; on cooldown → wait. `fired_this_tick` exposed for
  tests.
- **`openra-sim/src/world.rs`** — added `actor()/actor_mut()/
  actor_summary()/map_width()/map_height()/typed_shroud()/
  update_typed_shroud_all_players()/winners()/tick(orders)`. New
  `typed_shroud: BTreeMap<u32, Shroud>` field on `World`, populated
  by `tick()` (which wraps `process_frame` then refreshes typed
  shroud). `BTreeMap` keying preserves deterministic iteration; the
  per-tick visibility recompute is order-stable.

## Test results

```
cargo test --workspace                      # all green except 1 pre-existing
                                            # validate_all_shp_sprites
                                            # (documented in STATUS_PHASE_4)
cargo test -p openra-sim                    # 101 lib + 22 integration ✅
  - combat_one_v_one             1/1
  - combat_out_of_range          1/1
  - shroud_basic                 3/3
  - shroud_persistence           3/3
  - parity_combat                3/3 (live test skips when fixture absent)
cargo clippy -p openra-sim --tests --lib    # 0 warnings on files I touched
```

`combat_one_v_one`: 5000 hp / 1000 dmg / 20-tick reload → kill at
tick 81, well within the ±5% window of the analytical 81 estimate.

## Simplifications vs C# `AttackBase`

- **No projectile flight** — instant-hit. `Projectile` traits and the
  `WeaponInfo.Projectile` field aren't read. Defer to v2.
- **No splash damage** — single-target. `Warhead.Spread` ignored.
- **No `Versus` armor multipliers** — flat `weapon.damage` applied.
  This is the main source of the ±5% parity tolerance.
- **Single-weapon armaments only** — no turret + hull combos. Only
  the `Armament@PRIMARY` weapon is honoured.
- **No burst** — every shot resets cooldown to `reload_delay`.
  `Burst` / `BurstDelay` ignored.
- **Chebyshev range check** — matches the existing
  `world::tick_actors` Attack loop and the OpenRA grid melee
  semantics; the C# `WDist`-vs-`WPos` Euclidean check would round
  differently for diagonal shots at the cell boundary.

## Shroud model

OpenRA splits visibility into two flags. Verified against
`vendor/OpenRA/OpenRA.Mods.Common/Traits/World/Shroud.cs`:

- `IsVisible(cell)` — true iff a friendly source is revealing the
  cell *this tick*. Used by the actor visibility filter.
- `IsExplored(cell)` — sticky. Once set, never cleared. Used for
  terrain-tile rendering ("gray" fogged areas).

Our typed `Shroud` mirrors both. `is_visible_at` for actor
visibility, `is_explored` for terrain. Tests
`shroud_persistence::enemy_actor_only_visible_when_in_active_sight`
and `explored_remains_after_actor_leaves` lock that semantic in.

## Known parity gaps vs C#

1. **Versus class multipliers** — Rust applies flat damage; C# scales
   by `Warhead.Versus.<TargetClass>`. Up to ~50% drift on
   armored-vs-light pairings; e1-vs-e1 (none vs none) is the
   exception and matches.
2. **Range = Chebyshev cells** — OpenRA actually uses a Euclidean
   `WDist` check; our grid-melee approximation rounds differently at
   the cell edge for diagonal shots.
3. **Cooldown ticks every Activity::tick** — the data-driven
   `tick_actors` path decrements once per game tick; the trait-based
   path decrements once per `Activity::tick` call. In `World::tick`
   these are 1:1, but in unit tests that single-step the trait stack
   the count can diverge from the data-driven path's count — the
   integration tests use the trait stack exclusively to keep this
   well-defined.

## Files touched / added

```
openra-sim/src/traits/armament.rs        (new, 117 lines)
openra-sim/src/traits/shroud.rs          (new, 232 lines)
openra-sim/src/traits/mod.rs             (re-export + ActorSummary)
openra-sim/src/activities/attack.rs      (new, 233 lines)
openra-sim/src/activities/mod.rs         (wire AttackActivity)
openra-sim/src/world.rs                  (+typed_shroud, accessors,
                                          winners(), tick(orders))
openra-sim/tests/combat_one_v_one.rs     (new)
openra-sim/tests/combat_out_of_range.rs  (new)
openra-sim/tests/shroud_basic.rs         (new)
openra-sim/tests/shroud_persistence.rs   (new)
openra-sim/tests/parity_combat.rs        (new, fixture-driven)
scripts/dump_csharp_combat_trace.py      (new, parity dumper)
```

## Follow-ups for v2

- Projectile flight + travel-time damage (`AttackFrontal.Tick`).
- `Versus` warhead multipliers.
- Multi-armament actors (turret + hull).
- Burst patterns + `BurstDelay`.
- Per-tick cooldown unification: drive cooldowns from `World::tick`
  rather than per-`Activity::tick`, so the trait and data-driven
  paths agree to the cycle.
