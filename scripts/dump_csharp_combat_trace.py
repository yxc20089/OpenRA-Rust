#!/usr/bin/env python3
"""Drive a prod OpenRA gRPC server, run a 200-tick attack from rifleman A
on rifleman B, and dump per-tick `(tick, actor_a_hp, actor_b_hp, distance)`
tuples to `openra-sim/tests/fixtures/combat_csharp.json`. The Rust
parity test (`tests/parity_combat.rs`) compares its sim trace against
this file with a ±5% terminal-HP tolerance.

Same setup as `combat_one_v_one.rs`:
    - 2× e1 rifleman 5 cells apart on TEMPERAT terrain
    - Issue Attack from A → B
    - Run 200 ticks

Prereqs (on the user's training box):
    pip install -e ~/Projects/openra-rl                 # the gRPC bridge
    ssh -i ~/.ssh/lambda_ed25519 -L 8033:localhost:8033 \\
        ubuntu@192.222.58.98                            # forward port

Usage:
    python scripts/dump_csharp_combat_trace.py \\
        --host localhost --port 8033 \\
        --output openra-sim/tests/fixtures/combat_csharp.json
"""
from __future__ import annotations

import argparse
import json
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any


@dataclass
class TickSample:
    tick: int
    a_hp: int
    b_hp: int
    distance_cells: int


def _import_client() -> Any:
    """Locate the gRPC client lib without forcing it into requirements."""
    candidates = [
        Path.home() / "Projects" / "openra-rl",
        Path("/workspace/openra-rl"),
        Path("/Users/berta/Projects/openra-rl"),
    ]
    for candidate in candidates:
        if (candidate / "openra_env").exists():
            sys.path.insert(0, str(candidate))
            break
    try:
        from openra_env import client as openra_client  # type: ignore[import]
    except ImportError as exc:
        raise SystemExit(
            "openra_env not importable. Install ~/Projects/openra-rl with "
            "`pip install -e .` or set PYTHONPATH to its parent dir."
        ) from exc
    return openra_client


def _chebyshev(a: tuple[int, int], b: tuple[int, int]) -> int:
    return max(abs(a[0] - b[0]), abs(a[1] - b[1]))


def _hp_of(client: Any, actor_id: int) -> int:
    """Try the most likely client APIs for HP. Defaults to 0 if missing."""
    try:
        info = client.get_actor_info(actor_id)
    except Exception:  # noqa: BLE001 — best-effort, treat missing as dead
        return 0
    if info is None:
        return 0
    if isinstance(info, dict):
        for key in ("hp", "health", "current_hp"):
            if key in info:
                return int(info[key])
    return 0


def _cell_of(client: Any, actor_id: int) -> tuple[int, int]:
    try:
        info = client.get_actor_info(actor_id)
    except Exception:  # noqa: BLE001
        return (0, 0)
    if info is None:
        return (0, 0)
    cell = info.get("cell") if isinstance(info, dict) else None
    if cell:
        return (int(cell[0]), int(cell[1]))
    if isinstance(info, dict):
        return (int(info.get("x", 0)), int(info.get("y", 0)))
    return (0, 0)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="localhost")
    parser.add_argument("--port", type=int, default=8033)
    parser.add_argument(
        "--output",
        default="openra-sim/tests/fixtures/combat_csharp.json",
    )
    parser.add_argument("--ticks", type=int, default=200)
    parser.add_argument("--ax", type=int, default=5)
    parser.add_argument("--ay", type=int, default=10)
    parser.add_argument("--bx", type=int, default=10)
    parser.add_argument("--by", type=int, default=10)
    args = parser.parse_args()

    openra_client = _import_client()
    print(f"Connecting to {args.host}:{args.port} ...", flush=True)
    client = openra_client.OpenRAClient(host=args.host, port=args.port)

    if hasattr(client, "reset_with_units"):
        # Two-faction setup so they're enemies by default.
        client.reset_with_units(
            faction="allies",
            units=[("e1", args.ax, args.ay)],
            enemy_units=[("e1", args.bx, args.by)],
        )
    else:
        client.reset()
        client.spawn_actor("e1", args.ax, args.ay, owner=1)
        client.spawn_actor("e1", args.bx, args.by, owner=2)

    actor_a = client.find_actor_at(args.ax, args.ay)
    actor_b = client.find_actor_at(args.bx, args.by)
    print(f"actor_a={actor_a}  actor_b={actor_b}", flush=True)

    client.attack_actor(actor_a, actor_b)

    samples: list[TickSample] = []
    for tick in range(args.ticks + 1):
        a_hp = _hp_of(client, actor_a)
        b_hp = _hp_of(client, actor_b)
        a_cell = _cell_of(client, actor_a)
        b_cell = _cell_of(client, actor_b)
        samples.append(
            TickSample(
                tick=tick,
                a_hp=a_hp,
                b_hp=b_hp,
                distance_cells=_chebyshev(a_cell, b_cell),
            )
        )
        client.advance(1)
        time.sleep(0.0)
        if b_hp <= 0:
            break

    out_path = Path(args.output)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "scenario": {
            "actor_a_type": "e1",
            "actor_b_type": "e1",
            "actor_a_cell": [args.ax, args.ay],
            "actor_b_cell": [args.bx, args.by],
            "weapon": "M1Carbine",
            "ticks_requested": args.ticks,
        },
        "samples": [s.__dict__ for s in samples],
    }
    out_path.write_text(json.dumps(payload, indent=2))
    print(f"Wrote {len(samples)} samples to {out_path}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
