#!/usr/bin/env python3
"""Phase 8 smoke test — drive `cargo test` for the new specialist-infantry
suite (rocket projectile, splash, dog melee, versus damage, env turret
attachment).

Skip semantics
--------------
If `cargo` is not on PATH the script prints a skip notice and exits 0.
The full test suite runs through `cargo test --workspace` which already
covers Phase 8; this entry-point exists for parity with
`smoke_strategy_phase{6,7}.py` so a single Python invocation gates the
phase.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent

# Each test corresponds to one Phase 8 acceptance criterion.
PHASE_8_TESTS = [
    ("rocket_projectile_flies",       "openra-sim"),
    ("rocket_splash",                  "openra-sim"),
    ("dog_melee",                      "openra-sim"),
    ("versus_damage",                  "openra-sim"),
    ("env_attaches_turret",            "openra-train"),
    # Regression: Phase 6/7 still green.
    ("tank_duel",                      "openra-sim"),
    ("building_takes_damage",          "openra-sim"),
    ("building_fires_back",            "openra-sim"),
    ("building_blocks_path",           "openra-sim"),
    ("scout_maginot_smoke",            "openra-train"),
]

# Library tests for new modules.
PHASE_8_LIB_FILTERS = [
    ("openra-sim", "projectile::tests"),
    ("openra-sim", "traits::melee::tests"),
]


def run_test(crate: str, target: str) -> bool:
    print(f"--- cargo test -p {crate} --test {target}")
    res = subprocess.run(
        ["cargo", "test", "-p", crate, "--test", target],
        cwd=REPO_ROOT,
        env={**os.environ, "CARGO_TERM_COLOR": "never"},
    )
    return res.returncode == 0


def run_lib_filter(crate: str, filter_str: str) -> bool:
    print(f"--- cargo test -p {crate} --lib -- {filter_str}")
    res = subprocess.run(
        ["cargo", "test", "-p", crate, "--lib", "--", filter_str],
        cwd=REPO_ROOT,
        env={**os.environ, "CARGO_TERM_COLOR": "never"},
    )
    return res.returncode == 0


def main() -> int:
    if shutil.which("cargo") is None:
        print("skip: cargo not on PATH; Phase 8 smoke test needs Rust toolchain")
        return 0

    failures: list[str] = []
    for target, crate in PHASE_8_TESTS:
        ok = run_test(crate, target)
        status = "PASS" if ok else "FAIL"
        print(f"{status}: {crate}::{target}")
        if not ok:
            failures.append(f"{crate}::{target}")

    for crate, fil in PHASE_8_LIB_FILTERS:
        ok = run_lib_filter(crate, fil)
        status = "PASS" if ok else "FAIL"
        print(f"{status}: {crate} lib `{fil}`")
        if not ok:
            failures.append(f"{crate}::{fil}")

    if failures:
        print()
        print("Phase 8 smoke FAILED:")
        for f in failures:
            print(f"  - {f}")
        return 1
    print()
    print("Phase 8 smoke OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
