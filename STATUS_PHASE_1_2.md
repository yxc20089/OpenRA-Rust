# Phase 1 & 2 Status

Branch: `agent/phase-1-2-foundation`
Commits: `e211411` (phase 1), `d42f0d9` (phase 2)

## What was built

### Phase 1 — Actor + traits + typed Order
- `openra-sim/src/traits/` (folder module, ex-`traits.rs`)
  - `mod.rs`: existing `TraitState` enum (unchanged sync hashes) + new
    `pub mod health; pub mod mobile;` re-exports.
  - `health.rs`: `Health { hp, max_hp }` typed component with
    `take_damage`, `heal`, `kill`, `is_dead`, round-trip to
    `TraitState::Health`. 5 unit tests.
  - `mobile.rs`: `Mobile { facing, from_cell, to_cell, center_position,
    speed }` with stationary helper and round-trip to
    `TraitState::Mobile`. 3 unit tests.
- `openra-sim/src/order.rs`: typed `Order::{Move, Stop, Attack}` enum
  with `to_game_order(subject_id)` adapter onto the existing
  string-based `world::GameOrder`. 4 unit tests.

Actor identity hashing was already correct — `sync::hash_actor` uses
the C# formula `(actor_id << 16) as i32` and the existing
`sync_hash_tick0_matches_replay` integration test passes (computed
SyncHash matches the recorded `605399687`). C#'s `ActorInfo.GetHashCode`
itself is the default `Object.GetHashCode` (identity-based, not
content-based) — i.e. it does not contribute directly to the world
sync hash; the `Sync.HashActor(a)` formula is what we already match.

### Phase 2 — Activity stack + Move/Wait + parity scaffolding
- `openra-sim/src/activity.rs`: `Activity` trait
  (`fn tick -> ActivityState::{Continue,Done,Cancel}`), `take_child`
  hook, and `ActivityStack` LIFO that pushes children and pops
  finished/cancelled activities. 4 unit tests.
- `openra-sim/src/activities/move_.rs`: `MoveActivity { target_cell }`
  uses the existing `pathfinder::find_path` to install
  `actor::Activity::Move` (the data-driven enum the existing
  `world.tick_actors` already advances), then observes until arrival.
  4 unit tests.
- `openra-sim/src/activities/wait.rs`: `WaitActivity { ticks_remaining
  }` — deterministic countdown. 3 unit tests.
- `openra-sim/src/world.rs`: two `#[doc(hidden)] pub fn`
  test-only helpers — `insert_test_actor`, `set_test_unpaused` — for
  hand-built scenarios.
- `openra-sim/tests/move_activity_replay.rs`: 5-tick east move
  trajectory test (hand-computed reference: dx ≈ 516 world units
  after 12 ticks at speed 43). 2 integration tests.
- `openra-sim/tests/parity_move_vs_csharp.rs`: spawns one e1 at
  (10,10), orders Move to (15,15), asserts monotone Chebyshev
  convergence and arrival within 200 process_frame calls. The
  fixture-driven parity assertion (`tests/fixtures/move_trace.json`)
  is skipped when the fixture is absent. 5 integration tests.
- `scripts/dump_csharp_move_trace.py`: helper that drives the prod
  OpenRA gRPC server (`ubuntu@192.222.58.98:8033`) via
  `~/Projects/openra-rl/openra_env`, runs the same scenario, and
  dumps `(tick, cell_x, cell_y, sub_x, sub_y)` tuples as JSON. The
  Rust test consumes whatever fields it can find via a
  zero-dependency parser.

## Test results

```
$ cargo build --workspace
   Finished `dev` profile [unoptimized + debuginfo] target(s)

$ cargo test -p openra-sim
running 86 tests   ... 86 passed  (lib)
running 1 test     ...  1 passed  (debug_sync)
running 4 tests    ...  4 passed  (gamerules_integration)
running 2 tests    ...  2 passed  (move_activity_replay)        ← new
running 5 tests    ...  5 passed  (parity_move_vs_csharp)       ← new
running 2 tests    ...  2 passed  (sync_hash_verify)
total: 100/100 passed
```

`cargo clippy -p openra-sim --lib --tests` reports zero new warnings
in the files added by this work. Pre-existing warnings in `world.rs`
and `ai.rs` were left untouched.

## Deviations from the plan

1. **Existing scaffolding kept**: the project already had a
   data-driven `actor::Activity` enum (Move/Turn/Attack/Harvest)
   wired into a fully working `world::tick_actors` loop. Rather than
   re-write that, the new `Activity` trait sits as a parallel
   architecture for high-level callers; `MoveActivity` installs the
   data-driven `Activity::Move` and then observes until arrival.
   Both representations co-exist deterministically.
2. **Mobile.facing turn-rate not interpolated by the trait**: the
   existing `world` engine already handles facing interpolation via
   `Activity::Turn` and `pathfinder::facing_between`. The Phase-1
   `Mobile` component records the current facing only.
3. **C# `ActorInfo.GetHashCode`**: investigated — `ActorInfo` does
   not override `GetHashCode`, so it falls back to
   `Object.GetHashCode` (identity hash). The world-level identity
   contribution comes from `Sync.HashActor(a) = (int)(a.ActorID <<
   16)` which the existing `sync::hash_actor` already matches.
4. **`sync_hash_tick1`**: the test mentioned in the plan is actually
   named `sync_hash_tick0_matches_replay` and it already passes on
   `main`. No re-enablement was needed.
5. **`Actor` struct**: kept the existing `Actor` definition (no
   SlotMap migration, since the world stores actors in a
   `BTreeMap<u32, Actor>` keyed by stable u32 ids — same external
   behavior as a SlotMap, deterministic iteration order).

## Known issues / follow-ups

- `cargo clippy --workspace -- -D warnings` would currently fail on
  ~70 pre-existing warnings in `world.rs`/`ai.rs`/`oramap.rs` — none
  introduced by this work. Recommend a separate clippy-cleanup
  commit before enforcing `-D warnings` in CI.
- `openra-data` tests `parse_*_yaml` and `validate_all_shp_sprites`
  fail on machines without a checked-out `~/Projects/OpenRA/mods/ra/`
  asset tree. Pre-existing, unrelated to Phase 1/2.
- The parity fixture is intentionally not committed; running
  `scripts/dump_csharp_move_trace.py` on a host with prod-server
  access (or an SSH tunnel to `ubuntu@192.222.58.98:8033`) creates
  it under `openra-sim/tests/fixtures/move_trace.json` and the
  parity test will then assert ≤ 2-cell drift across the 50-tick
  trajectory.
- Phase 4 work (YAML rules + map loader) is in flight on
  `agent/phase-4-yaml-rules`; the two branches share the trait
  module rename (`traits.rs → traits/mod.rs`) and the
  `OraMap`/`PlayerDef` field shape used by Phase-2 tests, so
  merging should be straightforward.
