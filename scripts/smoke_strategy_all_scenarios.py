#!/usr/bin/env python3
"""Local smoke test for the four strategy scout scenarios on the Rust env.

Loops over scout-twobody, scout-maginot, scout-dilemma, scout-gauntlet:
  * Builds OpenRAEnv(scenario, seed=42)
  * Resets and runs 30 turns of `Observe` (warm-up / fog reveal)
  * Then 30 turns of scripted commands (Move all units toward map
    centre, Attack any visible enemy, else Observe)

Per-scenario metrics: starting unit count, units_killed across the
full 60-turn run, episode_completed, wall_clock_ms, plus a coarse note
on TODO(P9) features (medic Heal, APC transport).

Two modes:
  * Default: sequential (deterministic, easy to read).
  * --parallel N: runs N seeds × 4 scenarios via ProcessPoolExecutor
    (mirrors `local_16_episode_smoke.py`).

Exit 0 if every scenario ticked through without panic.
"""

from __future__ import annotations

import argparse
import os
import statistics
import sys
import time
import traceback
from concurrent.futures import ProcessPoolExecutor, as_completed
from dataclasses import dataclass

SCENARIO_DIR_DEFAULT = os.path.expanduser(
    "~/Projects/OpenRA-RL-Training/scenarios/strategy/"
)
SCENARIOS = [
    "scout-twobody",
    "scout-maginot",
    "scout-dilemma",
    "scout-gauntlet",
]


@dataclass
class ScenarioResult:
    name: str
    seed: int
    ok: bool
    own_units_start: int
    own_units_end: int
    enemy_units_seen: int
    enemy_buildings_seen: int
    units_killed: int
    final_tick: int
    explored_pct: float
    wall_clock_ms: float
    error: str = ""


# Map centres. Singletons maps are 128 wide / 64 tall (singles-twobody/maginot)
# and 96/72 for dilemma/gauntlet — close enough; we use a single neutral
# centre for the scripted "drive to centre" phase and rely on Observe for
# warm-up.
CENTRE = (64, 36)

WARMUP_TURNS = 30
ACTION_TURNS = 30


def _scenario_path(name: str) -> str:
    return os.path.join(SCENARIO_DIR_DEFAULT, f"{name}.yaml")


def run_one(name: str, seed: int) -> ScenarioResult:
    """Run one full smoke episode (60 turns) and return the result."""
    import openra_train  # imported in worker

    path = _scenario_path(name)
    if not os.path.exists(path):
        return ScenarioResult(
            name=name, seed=seed, ok=False,
            own_units_start=0, own_units_end=0,
            enemy_units_seen=0, enemy_buildings_seen=0,
            units_killed=0, final_tick=0, explored_pct=0.0,
            wall_clock_ms=0.0,
            error=f"scenario yaml missing: {path}",
        )

    t0 = time.perf_counter()
    try:
        env = openra_train.OpenRAEnv(path, seed)
        obs = env.reset()
        own_start = len(obs.get("unit_positions", {}))
        unit_ids = list(obs.get("unit_positions", {}).keys())
        enemy_units_seen = 0
        enemy_buildings_seen = 0
        last_obs = obs

        # Phase A: 30 turns of Observe — let fog reveal & enemies tick.
        for _ in range(WARMUP_TURNS):
            cmd = openra_train.Command.observe()
            obs, _r, done, _info = env.step([cmd])
            last_obs = obs
            enemy_units_seen = max(
                enemy_units_seen, len(obs.get("enemy_positions", []))
            )
            enemy_buildings_seen = max(
                enemy_buildings_seen,
                len(obs.get("enemy_buildings_summary", [])),
            )
            if done:
                break

        # Phase B: 30 turns of scripted action.
        for _ in range(ACTION_TURNS):
            visible_enemies = last_obs.get("enemy_positions", [])
            visible_buildings = last_obs.get("enemy_buildings_summary", [])
            if visible_enemies and unit_ids:
                target_id = visible_enemies[0]["id"]
                cmd = openra_train.Command.attack_unit(unit_ids, target_id)
            elif visible_buildings and unit_ids:
                # Buildings are valid attack targets via attack_unit (id-based).
                target_id = visible_buildings[0].get("id")
                if target_id is not None:
                    cmd = openra_train.Command.attack_unit(unit_ids, target_id)
                else:
                    cmd = openra_train.Command.move_units(
                        unit_ids, CENTRE[0], CENTRE[1]
                    )
            elif unit_ids:
                cmd = openra_train.Command.move_units(
                    unit_ids, CENTRE[0], CENTRE[1]
                )
            else:
                cmd = openra_train.Command.observe()

            obs, _r, done, _info = env.step([cmd])
            last_obs = obs
            enemy_units_seen = max(
                enemy_units_seen, len(obs.get("enemy_positions", []))
            )
            enemy_buildings_seen = max(
                enemy_buildings_seen,
                len(obs.get("enemy_buildings_summary", [])),
            )
            if done:
                break

        wall_ms = (time.perf_counter() - t0) * 1000.0
        return ScenarioResult(
            name=name, seed=seed, ok=True,
            own_units_start=own_start,
            own_units_end=len(last_obs.get("unit_positions", {})),
            enemy_units_seen=enemy_units_seen,
            enemy_buildings_seen=enemy_buildings_seen,
            units_killed=int(last_obs.get("units_killed", 0)),
            final_tick=int(last_obs.get("game_tick", 0)),
            explored_pct=float(last_obs.get("explored_percent", 0.0)),
            wall_clock_ms=wall_ms,
        )
    except Exception as e:
        wall_ms = (time.perf_counter() - t0) * 1000.0
        return ScenarioResult(
            name=name, seed=seed, ok=False,
            own_units_start=0, own_units_end=0,
            enemy_units_seen=0, enemy_buildings_seen=0,
            units_killed=0, final_tick=0, explored_pct=0.0,
            wall_clock_ms=wall_ms,
            error=f"{type(e).__name__}: {e}\n{traceback.format_exc()}",
        )


def _worker(payload):
    name, seed = payload
    return run_one(name, seed)


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--seed", type=int, default=42)
    p.add_argument("--parallel-seeds", type=int, default=0,
                   help="If >0, run N seeds per scenario in parallel "
                        "(perf benchmark mode). Default 0 = sequential.")
    p.add_argument("--workers", type=int, default=None)
    p.add_argument("--seed-base", type=int, default=1000)
    args = p.parse_args()

    overall_t0 = time.perf_counter()

    if args.parallel_seeds > 0:
        # 4 scenarios × N seeds, parallel.
        payload = []
        for name in SCENARIOS:
            for i in range(args.parallel_seeds):
                payload.append((name, args.seed_base + i))
        workers = args.workers or len(payload)
        print(f"Parallel mode: {len(payload)} envs ({len(SCENARIOS)} scenarios "
              f"× {args.parallel_seeds} seeds) on {workers} workers...",
              flush=True)
        results = []
        with ProcessPoolExecutor(max_workers=workers) as pool:
            futs = [pool.submit(_worker, x) for x in payload]
            for fut in as_completed(futs):
                results.append(fut.result())
    else:
        print(f"Sequential mode: 4 scenarios × seed={args.seed}", flush=True)
        results = [run_one(name, args.seed) for name in SCENARIOS]

    overall_wall_ms = (time.perf_counter() - overall_t0) * 1000.0

    # Per-scenario summary (for sequential mode this is one row each;
    # for parallel mode we group + aggregate).
    print()
    print(f"{'scenario':<18} {'seed':>5} {'ok':>3} "
          f"{'own_s':>5} {'own_e':>5} {'enU':>4} {'enB':>4} "
          f"{'kills':>5} {'tick':>5} {'expl%':>6} {'wall_ms':>8}")
    print("-" * 88)
    results_sorted = sorted(results, key=lambda r: (r.name, r.seed))
    for r in results_sorted:
        ok_s = "Y" if r.ok else "N"
        print(f"{r.name:<18} {r.seed:>5} {ok_s:>3} "
              f"{r.own_units_start:>5} {r.own_units_end:>5} "
              f"{r.enemy_units_seen:>4} {r.enemy_buildings_seen:>4} "
              f"{r.units_killed:>5} {r.final_tick:>5} "
              f"{r.explored_pct:>5.1f}% {r.wall_clock_ms:>8.1f}")
        if not r.ok:
            print(f"  ERROR: {r.error.splitlines()[0] if r.error else 'unknown'}")

    print("-" * 88)
    walls = [r.wall_clock_ms for r in results]
    if walls:
        print(f"per-env mean: {statistics.mean(walls):.0f} ms "
              f"(median {statistics.median(walls):.0f} ms)")
    print(f"total wall (par={args.parallel_seeds>0}): {overall_wall_ms:.0f} ms "
          f"over {len(results)} envs")

    # TODO(P9) note — features the strategy scenarios reference but
    # don't yet implement.
    print()
    print("Deferred (TODO P9): medic Heal, APC transport (Cargo load/unload),")
    print("AA-only weapons (sam, agun), MiniYAML concrete-parent inheritance")
    print("(RedEye → Nike range 7c512 → currently 5c0).")

    failures = [r for r in results if not r.ok]
    if failures:
        print()
        print(f"FAIL: {len(failures)}/{len(results)} envs panicked or errored")
        for r in failures:
            print(f"  {r.name} seed={r.seed}: {r.error.splitlines()[0]}")
        return 1
    print()
    print("OK: all envs ticked through without panic")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
