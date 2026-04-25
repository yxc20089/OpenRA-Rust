#!/usr/bin/env python3
"""Dump per-unit + per-weapon stats from the C# OpenRA YAML rules, for
cross-impl validation against the Rust `openra-data` parser.

The reference data is fetched from one of:
  1. A remote prod server via SSH (preferred for "this is what the engine
     actually runs"). Default: `ubuntu@192.222.58.98` with key
     `~/.ssh/lambda_ed25519`. Override with `--ssh-host` / `--ssh-key`.
     The remote OpenRA install is expected at `/home/ubuntu/openra-rl/OpenRA`
     (override with `--remote-mod-dir`).
  2. The local vendored copy at `vendor/OpenRA/mods/ra` (fallback when
     `--no-ssh` is passed or SSH fails).

Output is a JSON dump consumed by the Rust test suite via the
`openra-data/tests/cross_impl_rules.rs` integration test, plus a
human-readable diff report at `openra-data/tests/fixtures/rules_diff.txt`.

Units dumped (matches the spec — at least 5 unit types):
    e1, e3, e4, e6, jeep
Weapons dumped (referenced by the units above):
    M1Carbine, RedEye (e3), M60mg (e4), Maverick (e6 grenade?), 25mm (jeep)

Usage:
    python scripts/dump_csharp_rules.py                       # SSH default
    python scripts/dump_csharp_rules.py --no-ssh              # local YAML
    python scripts/dump_csharp_rules.py --output FILE.json
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from collections import OrderedDict
from pathlib import Path
from typing import Any

# ----------------------------------------------------------------------------
# YAML loader with OpenRA's MiniYaml inheritance semantics.
# ----------------------------------------------------------------------------

# Files we need (mirrors openra-data/src/rules.rs::load_ruleset).
RULES_FILES = [
    "defaults.yaml",
    "player.yaml",
    "world.yaml",
    "infantry.yaml",
    "vehicles.yaml",
    "aircraft.yaml",
    "ships.yaml",
    "structures.yaml",
    "decoration.yaml",
    "misc.yaml",
    "civilian.yaml",
    "fakes.yaml",
    "husks.yaml",
]


class Node:
    __slots__ = ("key", "value", "children")

    def __init__(self, key: str, value: str = "") -> None:
        self.key = key
        self.value = value
        self.children: list[Node] = []

    def child(self, key: str) -> "Node | None":
        for c in self.children:
            if c.key == key:
                return c
        return None

    def child_value(self, key: str) -> str | None:
        c = self.child(key)
        return c.value if c is not None else None


def _strip_comment(s: str) -> str:
    out = []
    for i, ch in enumerate(s):
        if ch == "#" and (i == 0 or s[i - 1] != "\\"):
            break
        out.append(ch)
    return "".join(out)


def _level(line: str) -> tuple[int, str]:
    level = 0
    spaces = 0
    text_start = 0
    for i, ch in enumerate(line):
        if ch == "\t":
            level += 1
            spaces = 0
            text_start = i + 1
        elif ch == " ":
            spaces += 1
            if spaces >= 4:
                spaces = 0
                level += 1
            text_start = i + 1
        else:
            text_start = i
            break
    return level, line[text_start:]


def parse_miniyaml(text: str) -> list[Node]:
    flat: list[tuple[int, str, str]] = []
    for raw in text.splitlines():
        level, rest = _level(raw)
        rest = _strip_comment(rest).strip()
        if not rest:
            continue
        if ":" in rest:
            k, _, v = rest.partition(":")
            flat.append((level, k.strip(), v.strip().replace("\\#", "#")))
        else:
            flat.append((level, rest.strip(), ""))

    # Stack-based tree build.
    roots: list[Node] = []
    stack: list[tuple[int, Node]] = []
    for level, key, value in flat:
        node = Node(key, value)
        # Pop until parent at level-1 is at top.
        while stack and stack[-1][0] >= level:
            stack.pop()
        if stack:
            stack[-1][1].children.append(node)
        else:
            roots.append(node)
        stack.append((level, node))
    return roots


def merge_nodes(base: list[Node], over: list[Node]) -> list[Node]:
    out = list(base)
    for ov in over:
        existing = next((n for n in out if n.key == ov.key), None)
        if existing is not None:
            if ov.value:
                existing.value = ov.value
            existing.children = merge_nodes(existing.children, ov.children)
        else:
            out.append(ov)
    return out


def parse_and_merge(sources: list[str]) -> list[Node]:
    merged: list[Node] = []
    for s in sources:
        merged = merge_nodes(merged, parse_miniyaml(s))
    return merged


def resolve_inherits(nodes: list[Node]) -> list[Node]:
    lookup = {n.key: n for n in nodes if n.key.startswith("^")}

    def resolve(n: Node) -> Node:
        out: list[Node] = []
        for c in n.children:
            if c.key == "Inherits" or c.key.startswith("Inherits@"):
                parent = lookup.get(c.value)
                if parent is None:
                    continue
                resolved_parent = resolve(parent)
                for pc in resolved_parent.children:
                    merge_into(out, pc)
            elif c.key.startswith("-"):
                target = c.key[1:]
                out = [x for x in out if x.key != target]
            else:
                merge_into(out, c)
        new = Node(n.key, n.value)
        new.children = [resolve(x) for x in out]
        return new

    def merge_into(dst: list[Node], n: Node) -> None:
        existing = next((x for x in dst if x.key == n.key), None)
        if existing is None:
            dst.append(n)
        else:
            if n.value:
                existing.value = n.value
            existing.children = merge_nodes(existing.children, n.children)

    return [resolve(n) for n in nodes if not n.key.startswith("^")]


# ----------------------------------------------------------------------------
# Field extraction (mirrors rules.rs::Rules / UnitInfo / WeaponStats).
# ----------------------------------------------------------------------------


def parse_wdist(s: str) -> int | None:
    """Return the fixed-point WDist length (1024/cell) or None."""
    s = s.strip()
    if "c" in s:
        cells_s, _, sub_s = s.partition("c")
        try:
            cells = int(cells_s)
            sub = int(sub_s)
        except ValueError:
            return None
        sign = -1 if cells < 0 else 1
        return cells * 1024 + sign * sub
    try:
        return int(s)
    except ValueError:
        return None


def find_trait(actor: Node, name: str, instance: str | None = None) -> Node | None:
    for t in actor.children:
        # Trait keys may be "Foo" or "Foo@INST".
        bare, _, inst = t.key.partition("@")
        if bare != name:
            continue
        if instance is None or inst == instance:
            return t
    return None


def extract_unit(actor: Node) -> dict[str, Any] | None:
    health = find_trait(actor, "Health")
    if health is None:
        return None
    hp_raw = health.child_value("HP")
    if hp_raw is None:
        return None
    try:
        hp = int(hp_raw)
    except ValueError:
        return None

    mobile = find_trait(actor, "Mobile")
    speed = None
    if mobile and mobile.child_value("Speed"):
        try:
            speed = int(mobile.child_value("Speed"))
        except ValueError:
            speed = None

    rs = find_trait(actor, "RevealsShroud")
    reveal_range = None
    if rs and rs.child_value("Range"):
        reveal_range = parse_wdist(rs.child_value("Range") or "")

    arm = find_trait(actor, "Armament", "PRIMARY") or find_trait(actor, "Armament")
    primary_weapon = arm.child_value("Weapon") if arm else None

    must_be_destroyed = find_trait(actor, "MustBeDestroyed") is not None

    return {
        "name": actor.key,
        "hp": hp,
        "speed": speed,
        "reveal_range": reveal_range,
        "primary_weapon": primary_weapon,
        "must_be_destroyed": must_be_destroyed,
    }


def extract_weapon(weapon: Node) -> dict[str, Any] | None:
    range_raw = weapon.child_value("Range")
    if range_raw is None:
        return None
    rng = parse_wdist(range_raw)
    if rng is None:
        return None

    rd_raw = weapon.child_value("ReloadDelay")
    if rd_raw is None:
        return None
    try:
        rd = int(rd_raw)
    except ValueError:
        return None

    damage = None
    for c in weapon.children:
        if not c.key.startswith("Warhead"):
            continue
        if c.value != "SpreadDamage":
            continue
        d = c.child_value("Damage")
        if d is not None:
            try:
                damage = int(d)
                break
            except ValueError:
                pass
    if damage is None:
        return None

    return {
        "name": weapon.key,
        "range": rng,
        "reload_delay": rd,
        "damage": damage,
    }


# ----------------------------------------------------------------------------
# Source acquisition (SSH or local).
# ----------------------------------------------------------------------------


def fetch_remote(
    host: str, key: str, remote_dir: str, files: list[str], subdir: str
) -> list[str]:
    """SCP a list of files from `remote_dir/subdir` and return their bytes."""
    out: list[str] = []
    for fname in files:
        remote_path = f"{remote_dir}/{subdir}/{fname}"
        with tempfile.NamedTemporaryFile("w+", delete=False) as tmp:
            local = tmp.name
        try:
            cmd = [
                "scp",
                "-q",
                "-i",
                key,
                "-o",
                "StrictHostKeyChecking=no",
                f"{host}:{remote_path}",
                local,
            ]
            r = subprocess.run(cmd, capture_output=True)
            if r.returncode != 0:
                # Skip missing optional files but warn on weapons (single dir).
                continue
            with open(local) as f:
                out.append(f.read())
        finally:
            try:
                os.unlink(local)
            except FileNotFoundError:
                pass
    return out


def fetch_remote_weapons(host: str, key: str, remote_dir: str) -> list[str]:
    # List all yaml files under {remote_dir}/weapons/ via ssh ls.
    cmd = ["ssh", "-i", key, "-o", "StrictHostKeyChecking=no", host, f"ls {remote_dir}/weapons/*.yaml 2>/dev/null"]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        return []
    paths = sorted(p.strip() for p in r.stdout.splitlines() if p.strip())
    out = []
    for p in paths:
        with tempfile.NamedTemporaryFile("w+", delete=False) as tmp:
            local = tmp.name
        try:
            cp = subprocess.run(
                ["scp", "-q", "-i", key, "-o", "StrictHostKeyChecking=no", f"{host}:{p}", local],
                capture_output=True,
            )
            if cp.returncode == 0:
                with open(local) as f:
                    out.append(f.read())
        finally:
            try:
                os.unlink(local)
            except FileNotFoundError:
                pass
    return out


def fetch_local(local_dir: Path) -> tuple[list[str], list[str]]:
    rules_dir = local_dir / "rules"
    weapons_dir = local_dir / "weapons"
    rule_sources = []
    for fname in RULES_FILES:
        p = rules_dir / fname
        if p.exists():
            rule_sources.append(p.read_text())
    weapons_sources = []
    if weapons_dir.exists():
        for p in sorted(weapons_dir.glob("*.yaml")):
            weapons_sources.append(p.read_text())
    return rule_sources, weapons_sources


# ----------------------------------------------------------------------------
# Top-level diff harness.
# ----------------------------------------------------------------------------

# Spec asks for at least 5 unit types: e1, e3, e4, e6, jeep.
DEFAULT_UNITS = ["E1", "E3", "E4", "E6", "JEEP"]


def main() -> int:
    here = Path(__file__).resolve().parent.parent
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--ssh-host", default="ubuntu@192.222.58.98")
    p.add_argument("--ssh-key", default=str(Path.home() / ".ssh" / "lambda_ed25519"))
    p.add_argument(
        "--remote-mod-dir",
        default="/home/ubuntu/openra-rl/OpenRA/mods/ra",
        help="Path on the remote server to the OpenRA RA mod directory.",
    )
    p.add_argument(
        "--no-ssh",
        action="store_true",
        help="Skip SSH and parse the local vendored YAML at vendor/OpenRA/mods/ra.",
    )
    p.add_argument("--units", nargs="+", default=DEFAULT_UNITS)
    p.add_argument(
        "--output",
        default=str(here / "openra-data/tests/fixtures/csharp_rules_dump.json"),
    )
    p.add_argument(
        "--diff",
        default=str(here / "openra-data/tests/fixtures/rules_diff.txt"),
        help="Path to write the human-readable Rust-vs-Python diff report.",
    )
    p.add_argument(
        "--rust-dump",
        default=None,
        help="Optional path to a Rust-side dump JSON for in-script diffing.",
    )
    args = p.parse_args()

    rule_srcs: list[str] = []
    weapon_srcs: list[str] = []
    source_label = ""
    if not args.no_ssh:
        try:
            rule_srcs = fetch_remote(args.ssh_host, args.ssh_key, args.remote_mod_dir, RULES_FILES, "rules")
            weapon_srcs = fetch_remote_weapons(args.ssh_host, args.ssh_key, args.remote_mod_dir)
            if rule_srcs:
                source_label = f"ssh:{args.ssh_host}:{args.remote_mod_dir}"
        except Exception as exc:  # pragma: no cover
            print(f"SSH fetch failed: {exc}", file=sys.stderr)

    if not rule_srcs:
        local = here / "vendor/OpenRA/mods/ra"
        if not local.exists():
            print(f"Local vendored mod dir not found at {local}", file=sys.stderr)
            return 2
        print(f"Using local vendored YAML at {local}", file=sys.stderr)
        rule_srcs, weapon_srcs = fetch_local(local)
        source_label = f"local:{local}"

    # Parse + resolve.
    rule_tree = resolve_inherits(parse_and_merge(rule_srcs))
    weapon_tree = resolve_inherits(parse_and_merge(weapon_srcs)) if weapon_srcs else []

    actors_by_name = {n.key: n for n in rule_tree}
    weapons_by_name = {n.key: n for n in weapon_tree}

    units_out: dict[str, dict[str, Any]] = OrderedDict()
    weapons_seen: set[str] = set()
    for name in args.units:
        node = actors_by_name.get(name)
        if node is None:
            print(f"warn: unit {name} not found in source", file=sys.stderr)
            continue
        info = extract_unit(node)
        if info is None:
            print(f"warn: unit {name} missing required fields", file=sys.stderr)
            continue
        units_out[name] = info
        if info.get("primary_weapon"):
            weapons_seen.add(info["primary_weapon"])

    weapons_out: dict[str, dict[str, Any]] = OrderedDict()
    for name in sorted(weapons_seen):
        node = weapons_by_name.get(name)
        if node is None:
            continue
        winfo = extract_weapon(node)
        if winfo is None:
            continue
        weapons_out[name] = winfo

    payload = {
        "source": source_label,
        "units": units_out,
        "weapons": weapons_out,
    }

    out_path = Path(args.output)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(payload, indent=2, sort_keys=True))
    print(f"wrote {out_path}", file=sys.stderr)

    # Diff against Rust-side dump if provided.
    if args.rust_dump:
        rust_path = Path(args.rust_dump)
        if not rust_path.exists():
            print(f"rust dump {rust_path} missing — skipping diff", file=sys.stderr)
        else:
            rust = json.loads(rust_path.read_text())
            lines = [
                f"# Cross-impl rules diff",
                f"# Python source: {source_label}",
                f"# Rust source:   {rust.get('source', 'unknown')}",
                "",
            ]
            mismatches = 0
            for name, py in units_out.items():
                ru = rust.get("units", {}).get(name)
                if ru is None:
                    lines.append(f"unit {name}: MISSING in Rust dump")
                    mismatches += 1
                    continue
                for field in ("hp", "speed", "reveal_range", "primary_weapon", "must_be_destroyed"):
                    if py.get(field) != ru.get(field):
                        lines.append(
                            f"unit {name}.{field}: py={py.get(field)!r} rust={ru.get(field)!r}"
                        )
                        mismatches += 1
            for name, py in weapons_out.items():
                ru = rust.get("weapons", {}).get(name)
                if ru is None:
                    lines.append(f"weapon {name}: MISSING in Rust dump")
                    mismatches += 1
                    continue
                for field in ("range", "reload_delay", "damage"):
                    if py.get(field) != ru.get(field):
                        lines.append(
                            f"weapon {name}.{field}: py={py.get(field)!r} rust={ru.get(field)!r}"
                        )
                        mismatches += 1
            lines.append("")
            lines.append(f"# {mismatches} mismatch(es)")
            Path(args.diff).parent.mkdir(parents=True, exist_ok=True)
            Path(args.diff).write_text("\n".join(lines))
            print(f"wrote {args.diff} ({mismatches} mismatches)", file=sys.stderr)
            return 1 if mismatches else 0
    return 0


if __name__ == "__main__":
    sys.exit(main())
