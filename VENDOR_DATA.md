# Vendor RA Data

The engine's unit / weapon / building / armor-class statistics live as
YAML embedded directly in the source tree at
`openra-data/src/embedded/{rules,weapons}/*.yaml`. The embedded YAML is
parsed once per process by `openra_data::embedded::load_ruleset_embedded`
(via `include_str!`), and the resulting `Ruleset` is fed through the
existing `GameRules::from_ruleset` translator. Result: the engine no
longer needs the upstream OpenRA repo at runtime — the wheel is fully
self-contained.

## Provenance

The embedded files were copied byte-for-byte from upstream OpenRA at
SHA `0938a27` (bleed branch, release-20231010 line). All OpenRA-Bench
scenario packs were tuned against this snapshot, so changing any stat
risks invalidating dozens of bench packs (the "no defect, no cheat" bar
in `OpenRA-Bench/CLAUDE.md`).

## How the loader resolves vendor data

`GameRules::from_vendor()` (and the train-side `load_rules_strict`) walk
this short chain:

1. `OPENRA_VENDOR_DIR` env var — if set, the path is parsed with the
   filesystem loader (`openra_data::rules::load_ruleset`). A broken
   path or unparseable ruleset PANICS — explicit overrides are
   expected to work, never silently swallowed.
2. Legacy in-tree `vendor/OpenRA/mods/ra/` — if present, used
   transparently (back-compat for older clones; tests in
   `openra-sim/tests/*.rs` that build their own ruleset directly via
   `load_ruleset(mod_dir)` still work).
3. **Embedded YAML** — the default. Cannot fail.

## Bumping the snapshot

1. Copy the new rules into `openra-data/src/embedded/{rules,weapons}/`
   (overwriting, NOT editing — keep the file content byte-identical to
   upstream so the provenance stays obvious).
2. Run `cargo test --release` and fix any stat-pinned tests that
   regress (rare — most pinned values come from the YAML).
3. Rebuild the wheel and re-run the OpenRA-Bench test suite. Any
   movement in pack pass/fail counts likely means the new snapshot
   shifted a HP/damage/cost that a pack was tuned against.
4. Bump the SHA reference at the top of this doc.

## Set of files embedded

Rule files (load order — must match `openra_data::rules::load_ruleset`):
`defaults`, `player`, `world`, `infantry`, `vehicles`, `aircraft`,
`ships`, `structures`, `decoration`, `misc`, `civilian`, `fakes`,
`husks`.

Weapon files (sorted-by-name, matching the filesystem loader's
sort order): `ballistics`, `explosions`, `missiles`, `other`,
`smallcaliber`, `superweapons`.
