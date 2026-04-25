#!/usr/bin/env python3
"""Phase 6 smoke test — vehicles + turrets damage parity.

Spawns a 2tnk vs 1tnk arena directly through the Rust integration test
`tank_duel::two_tnk_vs_one_tnk_kill_with_real_rules`. The test pulls
real weapon stats from the vendored OpenRA YAML and asserts the kill
falls within the analytical reload window.

This script also runs the full sim test suite to confirm no Phase 1-5
regressions.

We do NOT yet load the strategy scenarios end-to-end — they require
buildings as combatants (Phase 7) and specialised infantry weapons
(Phase 8). See PLAN_STRATEGY_SCENARIOS.md for the gating phase per
scenario.

Usage:
    python scripts/smoke_strategy_phase6.py
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys


REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), os.pardir))


def find_cargo() -> str:
    cargo = shutil.which("cargo")
    if cargo:
        return cargo
    candidate = os.path.expanduser("~/.cargo/bin/cargo")
    if os.path.exists(candidate):
        return candidate
    raise RuntimeError("cargo not found on PATH or ~/.cargo/bin")


def run(cmd: list[str]) -> int:
    print("$", " ".join(cmd), flush=True)
    return subprocess.call(cmd, cwd=REPO_ROOT)


def main() -> int:
    cargo = find_cargo()
    print(f"Using cargo: {cargo}\n")

    failed = False

    # 1. Tank duel integration test (the canonical Phase 6 acceptance check).
    rc = run([cargo, "test", "-p", "openra-sim", "--test", "tank_duel"])
    if rc != 0:
        print("FAIL: tank_duel integration test", file=sys.stderr)
        failed = True
    else:
        print("PASS: tank_duel\n")

    # 2. Phase 6 unit tests (turret + vehicle + multi-armament + weapon damage parser).
    rc = run([cargo, "test", "-p", "openra-sim", "--lib",
              "traits::turret::"])
    if rc != 0:
        print("FAIL: traits::turret unit tests", file=sys.stderr)
        failed = True
    else:
        print("PASS: traits::turret\n")

    rc = run([cargo, "test", "-p", "openra-sim", "--lib",
              "traits::vehicle::"])
    if rc != 0:
        print("FAIL: traits::vehicle unit tests", file=sys.stderr)
        failed = True
    else:
        print("PASS: traits::vehicle\n")

    rc = run([cargo, "test", "-p", "openra-sim", "--lib",
              "traits::armament::"])
    if rc != 0:
        print("FAIL: traits::armament unit tests", file=sys.stderr)
        failed = True
    else:
        print("PASS: traits::armament\n")

    rc = run([cargo, "test", "-p", "openra-sim", "--lib",
              "gamerules::tests::weapon_damage_from_warhead_for_real_yaml"])
    if rc != 0:
        print("FAIL: weapon damage from warheads", file=sys.stderr)
        failed = True
    else:
        print("PASS: weapon damage from warheads\n")

    # 3. Regression: sim full lib must still pass.
    rc = run([cargo, "test", "-p", "openra-sim", "--lib"])
    if rc != 0:
        print("FAIL: openra-sim full lib regression", file=sys.stderr)
        failed = True
    else:
        print("PASS: openra-sim full lib\n")

    if failed:
        print("\nPhase 6 smoke FAILED")
        return 1
    print("\nPhase 6 smoke OK — vehicles + turrets ready for env wiring")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
