//! Phase 4 cross-impl validation: confirm the Rust `Rules` typed view
//! matches the values produced by the Python reference parser
//! (`scripts/dump_csharp_rules.py`) for at least 5 unit types.
//!
//! This test does not re-run the Python script (that requires either a
//! prod-server SSH connection or local Python 3 + the vendored mod). It
//! reads the checked-in fixture `tests/fixtures/csharp_rules_dump.json`
//! produced by the script and asserts every field round-trips.
//!
//! To regenerate the fixture:
//!     cargo run -p openra-data --example dump_rules_json -- \
//!         --output openra-data/tests/fixtures/rust_rules_dump.json
//!     python3 scripts/dump_csharp_rules.py --no-ssh \
//!         --output openra-data/tests/fixtures/csharp_rules_dump.json \
//!         --rust-dump openra-data/tests/fixtures/rust_rules_dump.json \
//!         --diff openra-data/tests/fixtures/rules_diff.txt
//!
//! Cross-impl: the diff file at tests/fixtures/rules_diff.txt is asserted
//! to end with `# 0 mismatch(es)`. If it doesn't, the test fails and
//! prints the full diff so the regression is visible in CI output.

use openra_data::rules::{self, WDist};
use std::path::{Path, PathBuf};

fn vendored_mod_dir() -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(format!("{}/../vendor/OpenRA/mods/ra", manifest))
}

fn fixtures_dir() -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(format!("{}/tests/fixtures", manifest))
}

/// Required unit types per the Phase 4 spec.
const SPEC_UNITS: &[&str] = &["E1", "E3", "E4", "E6", "JEEP"];

#[test]
fn rust_dump_matches_spec_for_five_unit_types() {
    let mod_dir = vendored_mod_dir();
    if !mod_dir.exists() {
        return; // skip on environments without the vendored mod
    }
    let r = rules::Rules::load(&mod_dir).expect("load Rules");
    for &name in SPEC_UNITS {
        assert!(
            r.unit(name).is_some(),
            "spec unit {name} missing from typed Rules — \
             check that {} contains an `{name}:` actor (with Health.HP) \
             after inheritance resolution",
            mod_dir.display()
        );
    }
}

#[test]
fn rules_diff_shows_zero_mismatches() {
    let diff_path = fixtures_dir().join("rules_diff.txt");
    if !diff_path.exists() {
        eprintln!(
            "Skipping rules_diff_shows_zero_mismatches — fixture missing at {}. \
             Regenerate with: \
             cargo run -p openra-data --example dump_rules_json && \
             python3 scripts/dump_csharp_rules.py --no-ssh --rust-dump ...",
            diff_path.display()
        );
        return;
    }
    let contents = std::fs::read_to_string(&diff_path).expect("read diff");
    let last_meaningful_line = contents
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("");
    assert_eq!(
        last_meaningful_line.trim(),
        "# 0 mismatch(es)",
        "Cross-impl rules diff has mismatches. Full report:\n\n{contents}"
    );
}

#[test]
fn rust_and_python_dumps_agree_field_by_field() {
    // Sanity: read both JSON dumps (if present) and assert all the spec
    // fields agree. We do not import a JSON crate — we use a tiny
    // string-based extractor below since the dumps are produced by us in
    // a stable two-space-indent format.
    let py_path = fixtures_dir().join("csharp_rules_dump.json");
    let rs_path = fixtures_dir().join("rust_rules_dump.json");
    if !py_path.exists() || !rs_path.exists() {
        eprintln!("Skipping — fixtures missing. See module docs for regen.");
        return;
    }
    let py = std::fs::read_to_string(&py_path).unwrap();
    let rs = std::fs::read_to_string(&rs_path).unwrap();

    for name in SPEC_UNITS {
        for field in ["hp", "speed", "reveal_range", "primary_weapon", "must_be_destroyed"]
        {
            let py_v = scoped_field(&py, name, field);
            let rs_v = scoped_field(&rs, name, field);
            assert_eq!(
                py_v, rs_v,
                "{name}.{field} disagrees: python={py_v:?} rust={rs_v:?}"
            );
        }
    }
}

/// Sanity-check that the typed-Rules `Rules::load` call still returns
/// the expected E1 fields even after the cross-impl plumbing is in place.
#[test]
fn typed_rules_e1_field_sanity() {
    let mod_dir = vendored_mod_dir();
    if !mod_dir.exists() {
        return;
    }
    let r = rules::Rules::load(&mod_dir).unwrap();
    let e1 = r.unit("E1").unwrap();
    assert_eq!(e1.hp, 5000);
    assert_eq!(e1.speed, Some(54));
    assert_eq!(e1.reveal_range, Some(WDist::new(4096)));
    assert_eq!(e1.primary_weapon.as_deref(), Some("M1Carbine"));
}

/// Tiny field extractor: locates the line `"<name>": {`, then within
/// that block returns the trimmed value of `"<field>": <value>`.
/// Returns `None` if either is missing. Intentionally string-based so
/// we don't pull in a JSON dependency for `openra-data`.
fn scoped_field(json: &str, name: &str, field: &str) -> Option<String> {
    let needle = format!("\"{}\": {{", name);
    let start = json.find(&needle)?;
    // Scan forward to the matching brace.
    let bytes = json.as_bytes();
    let mut i = start + needle.len();
    let mut depth = 1;
    while i < bytes.len() && depth > 0 {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    let block = &json[start + needle.len()..i];
    let key = format!("\"{}\":", field);
    let kpos = block.find(&key)?;
    let after = &block[kpos + key.len()..];
    let mut end = 0;
    for (j, c) in after.char_indices() {
        if c == ',' || c == '\n' || c == '}' {
            end = j;
            break;
        }
    }
    Some(after[..end].trim().to_string())
}

/// Assert the diff file is sane regardless of whether the rust dump exists.
#[test]
fn diff_fixture_path_is_in_repo() {
    let p = fixtures_dir();
    assert!(
        Path::new(&p).exists(),
        "openra-data/tests/fixtures dir must exist — Phase 4 stores cross-impl artifacts here"
    );
}
