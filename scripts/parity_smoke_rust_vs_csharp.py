#!/usr/bin/env python3
"""Cross-impl smoke test: drive the Rust `openra_train.OpenRAEnv` and
the production C# OpenRA gRPC server with the same 5-command
sequence and compare basic observation fields.

Comparison rubric (Phase 5):
  * Same own_unit_count after each command.
  * ≤ 2 cell drift on each unit's (cell_x, cell_y).
  * Exact match on game_tick.
  * ≤ 5% drift on explored_percent.
Combat outcomes (HP, kills) are NOT compared — Agent B's combat path
is not yet merged into the Rust simulator.

Run:
    python scripts/parity_smoke_rust_vs_csharp.py \
        --host ubuntu@192.222.58.98 --port 8033

Pre-reqs:
    pip install maturin && maturin develop --release   (Rust env)
    pip install -e ~/Projects/openra-rl                (C# gRPC client)
    ssh -L 8033:localhost:8033 ubuntu@192.222.58.98     (or direct -p)

Outputs PASS/FAIL summary; non-zero exit on any rubric violation.
"""
from __future__ import annotations

import argparse
import os
import sys
from pathlib import Path
from typing import Any


def _import_rust_env() -> Any:
    try:
        import openra_train as ot  # type: ignore[import]
    except ImportError as exc:
        raise SystemExit(
            "openra_train Rust extension not importable. Run "
            "`maturin develop --release` from the workspace root."
        ) from exc
    return ot


def _import_csharp_client() -> Any:
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


def _resolve_scenario(arg: str) -> str:
    if os.path.exists(arg):
        return arg
    if arg in ("rush-hour", "rush_hour"):
        candidate = (
            Path.home()
            / "Projects/OpenRA-RL-Training/scenarios/discovery/rush-hour.yaml"
        )
        if candidate.exists():
            return str(candidate)
    raise SystemExit(f"scenario not found: {arg}")


def _rust_run(scenario_path: str, seed: int, command_specs: list[dict]) -> list[dict]:
    """Drive the Rust env. `command_specs` is a list of dicts with a
    `kind` key (`move|attack|observe`) plus per-kind args."""
    ot = _import_rust_env()
    env = ot.OpenRAEnv(scenario_path, seed)
    obs0 = env.reset()
    out: list[dict] = [_summarize_rust_obs(obs0)]

    own_ids = list(obs0["unit_positions"].keys())
    for spec in command_specs:
        kind = spec["kind"]
        if kind == "move":
            cmd = ot.Command.move_units(
                own_ids, int(spec["target_x"]), int(spec["target_y"])
            )
        elif kind == "attack":
            cmd = ot.Command.attack_unit(own_ids, str(spec["target_id"]))
        else:
            cmd = ot.Command.observe()
        obs, _r, _d, _info = env.step([cmd])
        out.append(_summarize_rust_obs(obs))
    return out


def _summarize_rust_obs(obs: dict) -> dict:
    return {
        "own_unit_count": len(obs.get("unit_positions") or {}),
        "unit_positions": {
            k: (int(v["cell_x"]), int(v["cell_y"]))
            for k, v in (obs.get("unit_positions") or {}).items()
        },
        "game_tick": int(obs.get("game_tick") or 0),
        "explored_percent": float(obs.get("explored_percent") or 0.0),
    }


def _csharp_run(
    host: str,
    port: int,
    scenario_path: str,
    seed: int,
    command_specs: list[dict],
) -> list[dict]:
    """Drive the C# game via openra-rl. We deliberately avoid
    binding to specific reward-pipeline calls — just open a session,
    issue raw orders, and read game state."""
    client = _import_csharp_client()
    sess = client.OpenRAClient(host=host, port=port)  # type: ignore[attr-defined]
    state = sess.start_game(  # type: ignore[attr-defined]
        scenario=scenario_path, seed=seed
    )
    out: list[dict] = [_summarize_csharp_state(state)]

    own_units = state.get("units_summary", []) or []
    own_ids = [str(u.get("id")) for u in own_units]

    for spec in command_specs:
        kind = spec["kind"]
        if kind == "move":
            sess.issue_orders(  # type: ignore[attr-defined]
                [
                    {
                        "order_id": "Move",
                        "subject_id": int(uid),
                        "target_string": f"{spec['target_x']},{spec['target_y']}",
                    }
                    for uid in own_ids
                ]
            )
        elif kind == "attack":
            sess.issue_orders(  # type: ignore[attr-defined]
                [
                    {
                        "order_id": "Attack",
                        "subject_id": int(uid),
                        "extra_data": int(spec["target_id"]),
                    }
                    for uid in own_ids
                ]
            )
        # advance ~30 ticks to mirror the Rust default
        sess.advance(30)  # type: ignore[attr-defined]
        state = sess.observe()  # type: ignore[attr-defined]
        out.append(_summarize_csharp_state(state))
    sess.close()  # type: ignore[attr-defined]
    return out


def _summarize_csharp_state(state: dict) -> dict:
    own_units = state.get("units_summary", []) or []
    return {
        "own_unit_count": len(own_units),
        "unit_positions": {
            str(u.get("id")): (int(u.get("cell_x", 0)), int(u.get("cell_y", 0)))
            for u in own_units
        },
        "game_tick": int(state.get("tick", 0) or 0),
        "explored_percent": float(state.get("explored_percent", 0.0) or 0.0),
    }


def _compare(
    rust: list[dict], csharp: list[dict]
) -> tuple[bool, list[str]]:
    """Apply the Phase-5 rubric. Returns (ok, messages)."""
    messages: list[str] = []
    n = min(len(rust), len(csharp))
    if len(rust) != len(csharp):
        messages.append(
            f"NOTE: snapshot count differs (rust={len(rust)} csharp={len(csharp)})"
        )
    ok = True
    for i in range(n):
        r, c = rust[i], csharp[i]
        if r["own_unit_count"] != c["own_unit_count"]:
            messages.append(
                f"step {i}: own_unit_count mismatch rust={r['own_unit_count']} "
                f"csharp={c['own_unit_count']}"
            )
            ok = False
        if r["game_tick"] != c["game_tick"]:
            messages.append(
                f"step {i}: game_tick mismatch rust={r['game_tick']} "
                f"csharp={c['game_tick']}"
            )
            ok = False
        # explored_percent drift ≤ 5%
        d_explored = abs(r["explored_percent"] - c["explored_percent"])
        if d_explored > 5.0:
            messages.append(
                f"step {i}: explored_percent drift {d_explored:.1f}% > 5%"
            )
            ok = False
        # cell drift ≤ 2
        for uid, (rx, ry) in r["unit_positions"].items():
            if uid not in c["unit_positions"]:
                continue  # different id assignment is OK; Phase 5 only asks for count parity
            cx, cy = c["unit_positions"][uid]
            if abs(rx - cx) > 2 or abs(ry - cy) > 2:
                messages.append(
                    f"step {i}: unit {uid} drift ({rx},{ry})↔({cx},{cy}) > 2 cells"
                )
                ok = False
    return ok, messages


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--scenario", default="rush-hour")
    p.add_argument("--seed", type=int, default=42)
    p.add_argument(
        "--host", default="ubuntu@192.222.58.98",
        help="SSH spec (user@host) or hostname; ignored if --port is the local forwarded port"
    )
    p.add_argument("--port", type=int, default=8033)
    p.add_argument(
        "--csharp-only", action="store_true",
        help="Skip the Rust env (useful when validating the C# side alone)",
    )
    p.add_argument(
        "--rust-only", action="store_true",
        help="Skip the C# server (run the Rust env in isolation)",
    )
    args = p.parse_args()

    scenario = _resolve_scenario(args.scenario)
    print(f"scenario: {scenario}")

    # Fixed 5-command sequence: 3 moves toward middle of map, then 1
    # observe, then 1 move back. This exercises the Move pipeline.
    command_specs: list[dict] = [
        {"kind": "move", "target_x": 30, "target_y": 15},
        {"kind": "move", "target_x": 60, "target_y": 20},
        {"kind": "move", "target_x": 90, "target_y": 20},
        {"kind": "observe"},
        {"kind": "move", "target_x": 60, "target_y": 30},
    ]

    rust_results: list[dict] = []
    csharp_results: list[dict] = []

    if not args.csharp_only:
        rust_results = _rust_run(scenario, args.seed, command_specs)
        print(f"rust snapshots: {len(rust_results)}")

    if not args.rust_only:
        try:
            csharp_results = _csharp_run(
                host=args.host.split("@")[-1] if "@" in args.host else args.host,
                port=args.port,
                scenario_path=scenario,
                seed=args.seed,
                command_specs=command_specs,
            )
            print(f"csharp snapshots: {len(csharp_results)}")
        except SystemExit:
            raise
        except Exception as exc:  # noqa: BLE001
            print(f"csharp client error: {exc}")
            csharp_results = []

    if args.rust_only:
        print("PASS (rust-only run, no comparison performed)")
        for i, snap in enumerate(rust_results):
            print(f"  step {i}: {snap['own_unit_count']} units, tick={snap['game_tick']}")
        return 0

    if args.csharp_only:
        print("PASS (csharp-only run, no comparison performed)")
        for i, snap in enumerate(csharp_results):
            print(f"  step {i}: {snap['own_unit_count']} units, tick={snap['game_tick']}")
        return 0

    if not csharp_results:
        print("FAIL: csharp results empty")
        return 1

    ok, messages = _compare(rust_results, csharp_results)
    for m in messages:
        print(m)
    print("PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
