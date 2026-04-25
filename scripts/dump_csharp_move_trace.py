#!/usr/bin/env python3
"""Drive a prod OpenRA gRPC server, run a 50-tick move from (10,10) to
(15,15), and dump per-tick `(tick, cell_x, cell_y, sub_x, sub_y)` tuples
to a JSON file consumed by `openra-sim/tests/parity_move_vs_csharp.rs`.

Prereqs (on the user's training box):
    pip install -e ~/Projects/openra-rl                 # the gRPC bridge
    ssh -i ~/.ssh/lambda_ed25519 -L 8033:localhost:8033 \
        ubuntu@192.222.58.98                            # forward port

Usage:
    python scripts/dump_csharp_move_trace.py \
        --host localhost --port 8033 \
        --output openra-sim/tests/fixtures/move_trace.json
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any


@dataclass
class TickSample:
    tick: int
    cell_x: int
    cell_y: int
    sub_x: int  # world units (1024 = 1 cell)
    sub_y: int


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


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="localhost")
    parser.add_argument("--port", type=int, default=8033)
    parser.add_argument(
        "--output",
        default="openra-sim/tests/fixtures/move_trace.json",
        help="Output JSON file (relative to repo root).",
    )
    parser.add_argument(
        "--ticks",
        type=int,
        default=50,
        help="Number of ticks to advance after issuing the move.",
    )
    parser.add_argument(
        "--from-x", type=int, default=10, help="Spawn cell X.",
    )
    parser.add_argument(
        "--from-y", type=int, default=10, help="Spawn cell Y.",
    )
    parser.add_argument(
        "--to-x", type=int, default=15, help="Move target cell X.",
    )
    parser.add_argument(
        "--to-y", type=int, default=15, help="Move target cell Y.",
    )
    args = parser.parse_args()

    openra_client = _import_client()
    print(f"Connecting to {args.host}:{args.port} ...", flush=True)
    client = openra_client.OpenRAClient(host=args.host, port=args.port)

    # Spawn an empty 32×32 single-actor scenario via the test harness.
    # The exact API depends on the gRPC bridge version; this script
    # reaches for `reset_with_units` if available, else falls back to
    # `reset` + single move command.
    if hasattr(client, "reset_with_units"):
        client.reset_with_units(
            faction="allies",
            units=[("e1", args.from_x, args.from_y)],
        )
    else:
        client.reset()
        # Bridge-specific spawn order; user can adapt as needed.
        client.spawn_actor("e1", args.from_x, args.from_y)

    actor_id = client.find_actor_at(args.from_x, args.from_y)
    print(f"actor_id={actor_id}", flush=True)

    client.move_actor(actor_id, args.to_x, args.to_y)

    samples: list[TickSample] = []
    for tick in range(args.ticks + 1):
        info = client.get_actor_info(actor_id)
        cx = int(info.get("center_x", info.get("cx", 0)))
        cy = int(info.get("center_y", info.get("cy", 0)))
        cell = info.get("cell", (info.get("x", 0), info.get("y", 0)))
        samples.append(
            TickSample(
                tick=tick,
                cell_x=int(cell[0]),
                cell_y=int(cell[1]),
                sub_x=cx,
                sub_y=cy,
            )
        )
        # Advance one game-tick.
        client.advance(1)
        time.sleep(0.0)

    out_path = Path(args.output)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "scenario": {
            "from": [args.from_x, args.from_y],
            "to": [args.to_x, args.to_y],
            "actor_type": "e1",
        },
        "samples": [s.__dict__ for s in samples],
    }
    out_path.write_text(json.dumps(payload, indent=2))
    print(f"Wrote {len(samples)} samples to {out_path}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
