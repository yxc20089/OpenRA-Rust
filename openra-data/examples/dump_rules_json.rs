//! Dump the typed `Rules` view to JSON, for cross-impl comparison against
//! `scripts/dump_csharp_rules.py`. Output schema matches the Python script
//! exactly (same field names, same types) so a tiny diff harness can
//! compare them line-for-line.
//!
//! Usage:
//!     cargo run -p openra-data --example dump_rules_json -- \
//!         --mod-dir vendor/OpenRA/mods/ra \
//!         --units E1 E3 E4 E6 JEEP \
//!         --output openra-data/tests/fixtures/rust_rules_dump.json
//!
//! No external JSON crate — we stick to `std` to keep the data crate's
//! dependency footprint small (per the Phase 4 brief).

use openra_data::rules;
use std::collections::BTreeMap;
use std::path::PathBuf;

const DEFAULT_UNITS: &[&str] = &["E1", "E3", "E4", "E6", "JEEP"];

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_or_null<T: std::fmt::Display>(v: Option<T>) -> String {
    match v {
        Some(x) => format!("{x}"),
        None => "null".to_string(),
    }
}

fn json_str_or_null(v: Option<&str>) -> String {
    match v {
        Some(s) => json_escape(s),
        None => "null".to_string(),
    }
}

fn main() -> std::io::Result<()> {
    // Lightweight argv parsing — keep dependencies to zero.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut mod_dir = PathBuf::from("vendor/OpenRA/mods/ra");
    let mut output = PathBuf::from("openra-data/tests/fixtures/rust_rules_dump.json");
    let mut units: Vec<String> = DEFAULT_UNITS.iter().map(|s| s.to_string()).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--mod-dir" => {
                mod_dir = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "--output" => {
                output = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "--units" => {
                // Consume all following non-flag args.
                units.clear();
                i += 1;
                while i < args.len() && !args[i].starts_with("--") {
                    units.push(args[i].clone());
                    i += 1;
                }
            }
            other => {
                eprintln!("unknown arg: {other}");
                i += 1;
            }
        }
    }

    if !mod_dir.exists() {
        eprintln!("mod dir not found: {}", mod_dir.display());
        std::process::exit(2);
    }

    let r = rules::Rules::load(&mod_dir)?;

    // Build deterministic output. Python script uses OrderedDict in spec
    // order for units, but for JSON-equality robustness we sort keys here
    // (the Python script also writes with sort_keys=True).
    let mut units_out: BTreeMap<String, String> = BTreeMap::new();
    let mut weapon_keys: std::collections::BTreeSet<String> = Default::default();
    for name in &units {
        let Some(u) = r.unit(name) else {
            eprintln!("warn: unit {name} not in ruleset");
            continue;
        };
        let mut obj = String::new();
        obj.push_str("    {\n");
        obj.push_str(&format!("      \"hp\": {},\n", u.hp));
        obj.push_str(&format!(
            "      \"must_be_destroyed\": {},\n",
            u.must_be_destroyed
        ));
        obj.push_str(&format!("      \"name\": {},\n", json_escape(&u.name)));
        obj.push_str(&format!(
            "      \"primary_weapon\": {},\n",
            json_str_or_null(u.primary_weapon.as_deref())
        ));
        obj.push_str(&format!(
            "      \"reveal_range\": {},\n",
            json_or_null(u.reveal_range.map(|w| w.length))
        ));
        obj.push_str(&format!("      \"speed\": {}\n", json_or_null(u.speed)));
        obj.push_str("    }");
        units_out.insert(name.clone(), obj);
        if let Some(w) = u.primary_weapon.as_deref() {
            weapon_keys.insert(w.to_string());
        }
    }

    let mut weapons_out: BTreeMap<String, String> = BTreeMap::new();
    for w_name in weapon_keys {
        let Some(w) = r.weapon(&w_name) else {
            continue;
        };
        let mut obj = String::new();
        obj.push_str("    {\n");
        obj.push_str(&format!("      \"damage\": {},\n", w.damage));
        obj.push_str(&format!("      \"name\": {},\n", json_escape(&w.name)));
        obj.push_str(&format!("      \"range\": {},\n", w.range.length));
        obj.push_str(&format!("      \"reload_delay\": {}\n", w.reload_delay));
        obj.push_str("    }");
        weapons_out.insert(w_name, obj);
    }

    let mut buf = String::new();
    buf.push_str("{\n");
    buf.push_str(&format!(
        "  \"source\": \"rust:{}\",\n",
        mod_dir.display()
    ));
    buf.push_str("  \"units\": {\n");
    let mut first = true;
    for (name, body) in &units_out {
        if !first {
            buf.push_str(",\n");
        }
        first = false;
        buf.push_str(&format!("    {}: {}", json_escape(name), &body[4..]));
        // The body string already has the indent from `    {`; we want to
        // strip the leading 4 spaces to get our own indent — done above.
    }
    buf.push_str("\n  },\n");
    buf.push_str("  \"weapons\": {\n");
    let mut first = true;
    for (name, body) in &weapons_out {
        if !first {
            buf.push_str(",\n");
        }
        first = false;
        buf.push_str(&format!("    {}: {}", json_escape(name), &body[4..]));
    }
    buf.push_str("\n  }\n");
    buf.push_str("}\n");

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output, &buf)?;
    eprintln!("wrote {}", output.display());
    Ok(())
}
