# Strategy Scenarios Integration — Status

Branch: `feat/strategy-scenarios-integration` (off `feat/rust-sim-integration`).
Merges Phase 6 (vehicles + turrets), Phase 7 (static defenses + buildings as
combatants), Phase 8 (specialist infantry + projectiles + melee + Versus
multipliers + env Vehicle/Turret components) onto the rush-hour sprint.

PR #1 (`feat/rust-sim-integration` → `main`) is still **OPEN** at the time of
this report; this branch is therefore stacked on top of #1.

## Merge log

```
git checkout feat/rust-sim-integration
git checkout -b feat/strategy-scenarios-integration
git merge --no-ff agent/phase-8-specialist-infantry
```

Single fast-forward-style merge; no conflicts (Phase 8 was already cut off
Phase 7 which was already cut off `feat/rust-sim-integration`, so the agent
branches form a linear stack).  35 files changed, 5 036 insertions.

## Test results

`cargo build --workspace` — clean (`Finished dev profile`, 4 dead-code
warnings in `world.rs` / `ai.rs`, no errors).

`cargo test --workspace` — green except the documented pre-existing
`openra-data::validate_all_shp_sprites` baseline failure (6 sprites with
0 frames; unrelated to Phases 6-8).  Excluding `openra-data`, the entire
workspace passes:

* `openra-sim` lib: 133 unit tests
* `openra-train` lib: 5 unit tests
* Integration tests (per crate, all green): `tank_duel`,
  `building_takes_damage`, `building_fires_back`, `building_blocks_path`,
  `rocket_projectile_flies`, `rocket_splash`, `dog_melee`, `versus_damage`,
  `env_attaches_turret`, `scout_maginot_smoke`, `rush_hour_map`,
  `rules_e1`, `mix_extract`, `replay_parse`, `terrain_parse`,
  `parity_combat`, `parity_smoke`, `scan_infantry`.

No new test failures introduced by the integration merge.

## Wheel + import

`maturin develop --release` (anaconda Python 3.12 venv at
`/opt/anaconda3`) builds successfully; `import openra_train` works and
exposes `OpenRAEnv` + `Command`.  The wheel is `abi3` so the same .so
serves Python ≥3.9.

## Smoke results — `scripts/smoke_strategy_all_scenarios.py` (new)

60 turns per env (30 Observe warm-up + 30 scripted Move/Attack), seed 42:

| scenario        | own_s | own_e | enU seen | enB seen | kills | tick | expl% | wall_ms |
|-----------------|------:|------:|---------:|---------:|------:|-----:|------:|--------:|
| scout-twobody   |    16 |     4 |        4 |        1 |    14 | 5403 | 33.3% |    52   |
| scout-gauntlet  |    16 |     0 |        0 |        1 |     0 | 3513 | 30.3% |    45   |
| scout-dilemma   |    12 |     0 |        0 |        0 |     0 | 3333 | 14.1% |    36   |
| scout-maginot   |    12 |     0 |        0 |        0 |     0 | 2973 |  8.4% |    34   |

All four scenarios load and tick to natural termination (player units
killed) without panic — meeting the fallback acceptance criterion.
`scout-twobody` even drives 14 kills with the dumb scripted policy.
`scout-dilemma` / `scout-maginot` close out before the units find any
enemy to attack (centre-rush vs spread-out spawns); fixing that is a
policy/positioning concern, not an engine one.

## Performance benchmark

64 envs (4 scenarios × 16 seeds) via `ProcessPoolExecutor`, 64 workers:

* **Total wall: 636 ms**, per-env mean 245 ms (median 236 ms).
* Strategy scenarios run ~3-5× more ticks than rush-hour (3 000-5 400 vs
  ~900) so per-env wall is higher; throughput is still **>100 envs/s**
  on a single Mac.  Determinism holds: every seed inside a scenario
  produces bit-identical kill counts and tick totals.

vs the rush-hour baseline (137 ms / 16 eps): expected slowdown given
the bigger maps + projectiles + auto-targeting buildings.

## Deferred (TODO P9 / out of scope)

* **Medic Heal** — `medi` actor sits idle, no heal armament.
* **APC + Jeep transport** (`Cargo: MaxWeight`) — strategy scenarios
  spawn an APC on dilemma/maginot but the winning policy never has to
  use it.
* **AA-only weapons** (`sam`, `agun`) — never fire (no aircraft).
* **MiniYAML concrete-parent inheritance** — `RedEye: Inherits: Nike`
  isn't followed; e3 rocket range is 5c0 (from `^AntiGroundMissile`)
  instead of Nike's 7c512.  Cosmetic / tactical depth, not blocking.
* **Cross-impl rocket parity** — `parity_rocket.rs` not landed; existing
  `parity_combat.rs` only covers instant-hit weapons.

## Wired into training repo

* `openra_rl_training/training/rust_env_pool.py` — no change needed;
  the obs forwarder is dict-shaped and already passes
  `enemy_buildings_summary` (added in Phase 7) through transparently.
* `openra-rl-training/examples/config-local-rust-strategy.yaml` (new) —
  sample local config exposing `use_rust_env: true` + `rust_env.scenarios`
  list.  `agent_rollout.py` is **not** edited; the swap remains a manual
  code change in the training repo, as requested.

## Recommended next steps

1. Land PR #1 first (rush-hour minimum) so this stack collapses.
2. Drive a real LLM through `scout-twobody` locally to confirm the
   richer obs (`enemy_buildings_summary`) parses cleanly into the
   prompt schema.
3. Ship Phase 9 (APC) + concrete-parent inheritance to unlock the full
   "scout" tactical layer (Nike-range rockets + transport).
4. Add a `parity_rocket.rs` cross-impl test once the C# game-server can
   be driven by gRPC at port 8033 from CI.
