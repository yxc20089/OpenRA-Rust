#!/usr/bin/env python3
"""Local 16-episode smoke test for the Rust OpenRAEnv (rush-hour).

Runs 16 episodes in parallel via ProcessPoolExecutor, each scripted
deterministically (no LLM):
  * Turn 1-5: move all own units toward the enemy spawn centroid.
  * Turn 6-30: attack the first visible enemy (Observe if none visible).

Per-episode metrics: total_kills, episode_length_ticks, win_flag,
wall_clock_ms. Aggregate stats printed at the end.

Usage:
    python scripts/local_16_episode_smoke.py
    python scripts/local_16_episode_smoke.py --episodes 16 --turns 30
"""

from __future__ import annotations

import argparse
import os
import statistics
import sys
import time
from concurrent.futures import ProcessPoolExecutor, as_completed
from dataclasses import dataclass


def default_scenario_path() -> str:
    candidates = [
        os.path.expanduser("~/Projects/OpenRA-RL-Training/scenarios/discovery/rush-hour.yaml"),
        os.path.join(os.path.dirname(__file__), "..", "..", "OpenRA-RL-Training",
                     "scenarios", "discovery", "rush-hour.yaml"),
    ]
    for c in candidates:
        if os.path.exists(c):
            return os.path.abspath(c)
    raise FileNotFoundError(
        "Could not find rush-hour.yaml. Pass --scenario explicitly."
    )


@dataclass
class EpisodeResult:
    seed: int
    total_kills: int
    episode_length_ticks: int
    win: bool
    done_reason: str
    wall_clock_ms: float
    final_explored_pct: float


def _enemy_centroid_default() -> tuple[int, int]:
    # Closest enemy squad in rush-hour spawn-point=0 is at (25, 10);
    # next cluster is at (45, 28-29). Push toward the closest first
    # so 30 turns × 30 ticks/turn is enough to make contact.
    return (25, 10)


def run_one_episode(args: tuple[str, int, int]) -> EpisodeResult:
    """Run a single 30-turn episode under a unique seed."""
    scenario_path, seed, num_turns = args

    import openra_train  # imported in worker process

    t0 = time.perf_counter()
    env = openra_train.OpenRAEnv(scenario_path, seed)
    obs = env.reset()
    unit_ids = list(obs["unit_positions"].keys())
    enemy_cx, enemy_cy = _enemy_centroid_default()

    done = False
    last_obs = obs
    final_tick = obs.get("game_tick", 0)
    win = False
    done_reason = "max_turns"

    for turn in range(1, num_turns + 1):
        if turn <= 5:
            # Move toward the enemy cluster
            cmd = openra_train.Command.move_units(unit_ids, enemy_cx, enemy_cy)
        else:
            visible_enemies = last_obs.get("enemy_positions", [])
            if visible_enemies:
                target_id = visible_enemies[0]["id"]
                cmd = openra_train.Command.attack_unit(unit_ids, target_id)
            else:
                cmd = openra_train.Command.observe()

        obs, reward, done, info = env.step([cmd])
        last_obs = obs
        final_tick = obs.get("game_tick", final_tick)
        if done:
            # Heuristic for win flag — if we still have own units and
            # no enemies remain visible at terminal, count it as a
            # win. The Rust env doesn't yet emit a typed winner.
            own_alive = len(obs.get("unit_positions", {})) > 0
            enemies_seen = len(obs.get("enemy_positions", []))
            win = own_alive and enemies_seen == 0
            done_reason = "terminal"
            break

    wall_clock_ms = (time.perf_counter() - t0) * 1000.0
    return EpisodeResult(
        seed=seed,
        total_kills=int(last_obs.get("units_killed", 0)),
        episode_length_ticks=int(final_tick),
        win=win,
        done_reason=done_reason,
        wall_clock_ms=wall_clock_ms,
        final_explored_pct=float(last_obs.get("explored_percent", 0.0)),
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--scenario", default=None,
                        help="Path to rush-hour.yaml (auto-detected if omitted)")
    parser.add_argument("--episodes", type=int, default=16)
    parser.add_argument("--turns", type=int, default=30)
    parser.add_argument("--seed-base", type=int, default=1000)
    parser.add_argument("--workers", type=int, default=None,
                        help="Number of parallel workers (default: episodes)")
    args = parser.parse_args()

    scenario = args.scenario or default_scenario_path()
    if not os.path.exists(scenario):
        print(f"ERROR: scenario not found: {scenario}", file=sys.stderr)
        return 2

    workers = args.workers or args.episodes
    seeds = [args.seed_base + i for i in range(args.episodes)]
    payload = [(scenario, s, args.turns) for s in seeds]

    print(f"Launching {args.episodes} episodes ({args.turns} turns each) "
          f"on {workers} workers...", flush=True)
    overall_t0 = time.perf_counter()

    results: list[EpisodeResult] = []
    with ProcessPoolExecutor(max_workers=workers) as pool:
        futures = [pool.submit(run_one_episode, p) for p in payload]
        for fut in as_completed(futures):
            try:
                results.append(fut.result())
            except Exception as e:
                print(f"  episode failed: {e}", file=sys.stderr)

    overall_wall_ms = (time.perf_counter() - overall_t0) * 1000.0

    if not results:
        print("ERROR: zero episodes completed", file=sys.stderr)
        return 1

    results.sort(key=lambda r: r.seed)

    # Per-episode table
    print()
    print(f"{'seed':>6} {'kills':>6} {'ticks':>6} {'explored%':>10} "
          f"{'win':>5} {'reason':>10} {'wall_ms':>8}")
    print("-" * 60)
    for r in results:
        print(f"{r.seed:>6d} {r.total_kills:>6d} {r.episode_length_ticks:>6d} "
              f"{r.final_explored_pct:>9.2f}% {('Y' if r.win else 'N'):>5} "
              f"{r.done_reason:>10} {r.wall_clock_ms:>8.1f}")

    # Aggregate stats
    kills = [r.total_kills for r in results]
    ticks = [r.episode_length_ticks for r in results]
    walls = [r.wall_clock_ms for r in results]
    wins = sum(1 for r in results if r.win)
    n = len(results)

    print("-" * 60)
    print(f"episodes:           {n}")
    print(f"win rate:           {wins/n*100:.1f}% ({wins}/{n})")
    print(f"mean kills:         {statistics.mean(kills):.2f}")
    print(f"mean ticks:         {statistics.mean(ticks):.0f}")
    print(f"mean explored%:     {statistics.mean(r.final_explored_pct for r in results):.2f}")
    print(f"per-episode wall:   {statistics.mean(walls):.0f} ms (median {statistics.median(walls):.0f} ms)")
    print(f"total wall (par):   {overall_wall_ms:.0f} ms")
    seq_estimate = sum(walls)
    print(f"sum-of-eps (seq):   {seq_estimate:.0f} ms")
    print(f"speedup (par/seq):  {seq_estimate/max(overall_wall_ms, 1.0):.2f}x")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
