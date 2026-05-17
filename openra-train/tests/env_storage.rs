//! S1 silo storage: storage capacity comes from refineries/silos
//! (proc≈2000, silo≈3000), harvested resources are capped by it
//! (overflow lost), and a per-tick drain converts stored resources to
//! spendable cash. Building a silo raises the cap — the user's
//! "build more storage so the harvest limit is higher" mechanic.

use openra_train::{Command, Env};
use std::io::Write;
use std::path::PathBuf;

fn write(name: &str, body: &str) -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let dir = PathBuf::from(&home).join("Projects/OpenRA-RL-Training/scenarios/discovery");
    if !dir.join("rush-hour.yaml").exists() {
        return None;
    }
    let p = dir.join(name);
    std::fs::File::create(&p).ok()?.write_all(body.as_bytes()).ok()?;
    Some(p)
}

const HEAD: &str = "name: Storage\nbase_map: ../maps/rush-hour-arena.oramap\n\
spawn_mcvs: false\nstarting_cash: 200\nagent:\n  faction: allies\n\
enemy:\n  faction: soviet\ntermination:\n  max_ticks: 40000\nactors:\n";

fn proc_harv_mine() -> String {
    format!(
        "{HEAD}- type: proc\n  owner: agent\n  position:\n  - 12\n  - 18\n\
- type: harv\n  owner: agent\n  position:\n  - 14\n  - 18\n\
- type: mine\n  owner: neutral\n  position:\n  - 22\n  - 18\n"
    )
}

#[test]
fn capacity_from_buildings_and_cap_enforced_and_drains_to_cash() {
    let Some(p1) = write("_stor_proc.yaml", &proc_harv_mine()) else {
        eprintln!("skip: RL-Training scenarios not present");
        return;
    };
    let mut e = Env::new(p1.to_str().unwrap(), 7).unwrap();
    let o = e.reset();
    let h: Vec<String> = o
        .unit_positions
        .iter()
        .filter(|(_, p)| (p.cell_x, p.cell_y) == (14, 18))
        .map(|(id, _)| id.clone())
        .collect();
    // proc only → capacity 2000.
    assert_eq!(o.economy.resource_capacity, 2000, "proc capacity");

    let mut peak_cash = 200;
    for _ in 0..200 {
        let r = e.step(&[Command::Harvest {
            unit_ids: h.clone(),
            target_x: 22,
            target_y: 18,
        }]);
        // Stored resources can never exceed the storage cap.
        assert!(
            r.obs.economy.resources <= r.obs.economy.resource_capacity,
            "resources {} exceeded cap {}",
            r.obs.economy.resources,
            r.obs.economy.resource_capacity,
        );
        peak_cash = peak_cash.max(r.obs.economy.cash);
    }
    assert!(peak_cash > 200, "drain must convert stored ore to cash");
    let _ = std::fs::remove_file(&p1);

    // proc + silo → capacity 2000 + 3000.
    let with_silo = proc_harv_mine()
        + "- type: silo\n  owner: agent\n  position:\n  - 10\n  - 21\n";
    let Some(p2) = write("_stor_silo.yaml", &with_silo) else { return };
    let o2 = Env::new(p2.to_str().unwrap(), 7).unwrap().reset();
    assert_eq!(
        o2.economy.resource_capacity, 5000,
        "silo must raise the storage cap"
    );
    let _ = std::fs::remove_file(&p2);
}
