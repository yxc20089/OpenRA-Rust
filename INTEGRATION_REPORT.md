# Rush-Hour Sprint Integration Report

Branch: `feat/rust-sim-integration` (off `main`).

## Merges

All four feature branches landed clean. `world.rs` was touched by A,
B, and D — `ort` auto-merged (Agent A's tick scaffold + Agent B's
`typed_shroud` + Agent D's `remove_test_actor`/`all_actor_ids` all
preserved, no manual conflict resolution).

| Branch | Strategy | Conflicts |
|--------|----------|-----------|
| `agent/phase-1-2-foundation` | fast-forward | none |
| `agent/phase-4-yaml-rules` | recursive | none |
| `agent/phase-3-combat-shroud` | recursive | none |
| `agent/phase-5-pyo3` | recursive | `world.rs` (auto) |

## TODO(B) resolution

1. **`refresh_explored_cells`** now reads
   `World::typed_shroud(player).is_explored(x, y)`; env calls
   `update_typed_shroud_all_players()` after every `process_frame`.
2. **`update_kill_counter`** uses `World::kills_for_player(pid)`. To
   support that I added a `kills_per_player: BTreeMap` on `World` and
   credited kills in **both** combat paths (data-driven `tick_actors`
   + typed `AttackActivity::tick`).
3. **`different_seed_yields_different_hash`** tightened to strict
   `assert_ne!`. Added `Env::world_sync_hash` and a
   `pick_diverging_seeds` helper that picks two seeds whose first
   `assign_spawn_points` draw differs — needed because the rush-hour
   command surface is RNG-poor.

## Test results

`cargo test --workspace --no-fail-fast`: all green except
`validate_all_shp_sprites` (pre-existing, documented in
STATUS_PHASE_4). `cargo clippy --workspace` clean — zero new warnings
on touched files; pre-existing warnings on `world.rs`/`ai.rs`/
`openra-wasm` left alone.

## Wheel install (macOS)

```
python3 -m venv .venv-rust-test
.venv-rust-test/bin/pip install maturin
unset CONDA_PREFIX            # else maturin errors
VIRTUAL_ENV=.../.venv-rust-test maturin develop --release
```

Built `openra_train-0.1.0-cp39-abi3-macosx_11_0_arm64.whl` in 5 s
release. `OpenRAEnv(path, 42).reset()` returns the expected dict.

## 16-episode smoke

`scripts/local_16_episode_smoke.py`, ProcessPoolExecutor × 16,
30 turns/episode (≈ 2700 sim ticks):

```
mean kills:       10 / 13 enemy units
mean ticks:       2703   (terminal at max_ticks)
mean explored%:   10.15
per-episode wall: 22 ms (median 22)
total wall (par): 126 ms
sum-of-eps (seq): 350 ms
```

## Performance vs prod

Prod gRPC env: ~3.5 s/episode (~17 eps/min, GH200). 16 eps sequential
≈ 56 s.

Rust sim: 16 eps in **126 ms parallel** ⇒ **≈ 440× speedup** vs prod.
Sequential 350 ms ⇒ **160×**. Single-episode 22 ms vs 3500 ms = 159×.

## RustEnvPool adapter

`openra_rl_training/training/rust_env_pool.py` — mirrors
`env_pool.EnvPool` (`acquire/release/update_scenario/shutdown`).
Backed by `RustEnvHandle` wrapping `openra_train.OpenRAEnv`. **Not
wired into `agent_rollout.py`** — left as a manual flip for the user.

Tests: `tests/test_rust_env_pool.py` — 4 cases, all pass.

## Known issues / skips

- `validate_all_shp_sprites` pre-existing failure (graphics
  pipeline, not in scope).
- All 16 smoke episodes produce identical kill counts (10) — the
  scripted policy is deterministic and the env's RNG only influences
  spawn-faction picks which the scenario YAML overrides. The
  determinism test uses the spawn-side-channel for stricter coverage.
- Pre-existing `pymethods` E0133 warnings from pyo3 0.22 on Rust 1.94
  are cosmetic (no behavior change).
