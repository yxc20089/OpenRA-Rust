# Phase 4 status — YAML rules + rush-hour map loader

Branch: `agent/phase-4-yaml-rules` (4 commits ahead of `main`).

## What landed

- **Typed Rules layer** (`openra-data/src/rules.rs`):
  `Rules`, `UnitInfo`, `WeaponStats`, `WDist` (mirrors `openra-sim`'s
  fixed-point type), `parse_wdist("5c512")`. `Rules::load(mod_dir)` reads
  every rules/*.yaml + weapons/*.yaml under a mod, runs the existing
  MiniYaml parser + inheritance resolver, and exposes deterministic
  `BTreeMap`-backed lookups by uppercase name. `UnitInfo` extracts
  `Health.HP`, `Mobile.Speed`, `RevealsShroud.Range`,
  `Armament[@PRIMARY].Weapon`, `MustBeDestroyed`. `WeaponStats` extracts
  `Range`, `ReloadDelay`, and the first `Warhead@*: SpreadDamage.Damage`.
- **Rush-hour map loader** (`openra-data/src/oramap.rs`):
  `MapDef` + `ScenarioActor` + `load_rush_hour_map(path)` /
  `load_rush_hour_map_with_spawn(path, n)`. Combines the base
  `rush-hour-arena.oramap` (terrain, bounds) with the discovery scenario
  YAML (PyYAML compact form) — handles `count: N` expansion,
  `spawn_point: N` agent filter, skips `randomize:` blocks for
  determinism. Falls back through scenario-relative ref → sibling →
  `~/Projects/openra-rl/maps/` → `~/Projects/OpenRA-RL-Training/scenarios/maps/`.
- **MiniYaml parser** was already in place — left as-is, only added
  fixture-style tests.
- **Cross-impl validation**: Python `scripts/dump_csharp_rules.py`
  (SSHs to `ubuntu@192.222.58.98` for live YAML, falls back to vendored
  copy with `--no-ssh`), Rust `openra-data/examples/dump_rules_json.rs`
  (stdlib-only JSON), checked-in fixtures
  (`csharp_rules_dump.json` + `rust_rules_dump.json` + `rules_diff.txt`),
  and `cross_impl_rules.rs` test that fails on any field divergence.

## Test results

```
cargo build --workspace                                # clean
cargo test -p openra-data --lib                        # 44/44 passed
cargo test -p openra-data --test miniyaml_basic        # 2/2
cargo test -p openra-data --test miniyaml_inheritance  # 2/2
cargo test -p openra-data --test rules_e1              # 4/4
cargo test -p openra-data --test rush_hour_map         # 2/2
cargo test -p openra-data --test cross_impl_rules      # 5/5
cargo test -p openra-data --test miniyaml_real         # 5/5 (was failing on
                                                       #  hardcoded path; fixed)
```

The pre-existing `validate_all_shp_sprites` failure is unrelated to
Phase 4 (six SHP files in the vendored .mix archive decode to zero
frames; that's a graphics-pipeline issue not gated by this sprint).

## Verified field values (E1 + M1Carbine + 4 more)

```
E1.hp                = 5000
E1.speed             = 54        (inherited from ^Infantry)
E1.reveal_range      = WDist(4096)  (= 4c0)
E1.primary_weapon    = M1Carbine
E1.must_be_destroyed = true      (inherited from ^Soldier)
M1Carbine.range        = WDist(5120)  (= 5c0)
M1Carbine.reload_delay = 20
M1Carbine.damage       = 1000   (inherited from ^LightMG → ^HeavyMG)
```

5 units (E1, E3, E4, E6, JEEP) round-trip identically between the Rust
typed `Rules` view and the Python reference parser — diff report shows
`# 0 mismatch(es)`.

## Rush-hour map assertion

`load_rush_hour_map(scenarios/discovery/rush-hour.yaml)` at
`spawn_point=0` yields exactly **13 enemy infantry + 5 own infantry**
(3× e1 + 2× dog), matching the spec. Map metadata round-trips:
128×40 cells, TEMPERAT tileset, bounds (2, 2, 124, 36). `spawn_point=1`
returns the same enemy set with different agent positions.

## Parser quirks worth flagging

- The discovery scenario YAML uses **PyYAML compact form**: list items
  under `actors:` sit at indent 0, *same* column as the `actors:` key.
  Detection had to switch from "indent > parent" to "starts with `- `".
- OpenRA's MiniYaml parser already handled `Inherits@TAG: ^Parent`
  (multiple Inherits with disambiguating tags); the test fixture was
  built around the simpler unprefixed form.
- `Armament` actually appears unprefixed on E1 in the resolved tree
  *and* as `Armament@PRIMARY` — we prefer `@PRIMARY` first to match the
  C# behaviour where it is the dominant trait used by the AttackBase
  controller.
- `Damage` lives inside `Warhead@1Dam: SpreadDamage`, which is a
  sub-block (not a leaf in `params`). The typed extractor walks
  `weapon.children` to find it.

## C# fields not yet replicated

- `Mobile.Locomotor`, `TerrainSpeeds`, and per-terrain speed multipliers
  — captured in `ActorInfo.params` but not surfaced on `UnitInfo`.
- `Burst`, `Spread`, `Versus:` armor multipliers, multi-warhead damage
  combinations — left for the combat phase (Agent B).
- Turreted vs. always-front armaments — `WeaponInfo` keeps the raw tree
  in `children` so consumers can extract turret data later.
- Validation is currently against the vendored YAML (same source the
  Rust parser reads). The SSH path in `dump_csharp_rules.py` is
  implemented but untested in this session — it will activate
  automatically when `ssh ubuntu@192.222.58.98 ls /home/ubuntu/openra-rl/OpenRA/mods/ra/rules`
  succeeds.

## Files of interest

- `openra-data/src/rules.rs` — typed Rules layer
- `openra-data/src/oramap.rs` — `load_rush_hour_map` + scenario parser
- `openra-data/tests/{miniyaml_basic,miniyaml_inheritance,rules_e1,rush_hour_map,cross_impl_rules}.rs`
- `openra-data/tests/fixtures/{csharp,rust}_rules_dump.json`, `rules_diff.txt`
- `openra-data/examples/dump_rules_json.rs` (run via `cargo run --example`)
- `scripts/dump_csharp_rules.py`

Phase 5 (PyO3) and `World::from_rules_and_map(rules, map)` can consume
`Rules` + `MapDef` directly — no further plumbing in `openra-data`
expected unless map terrain semantics need richer typing.
