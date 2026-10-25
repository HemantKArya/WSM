//! Read-only installer stall capture. Run while the builder owns the VM.
use anyhow::Result;
use serde_json::json;
use std::{io::Write, path::PathBuf, time::{Duration, Instant}};
use wsm_core::qmp::Qmp;

fn main() -> Result<()> {
    let root = PathBuf::from(std::env::args().nth(1).expect("installer run directory"));
    let out = PathBuf::from(std::env::args().nth(2).expect("output log"));
    let start = Instant::now();
    while std::env::args().nth(3).as_deref() != Some("--now") && start.elapsed() < Duration::from_secs(7200) {
        let log = std::fs::read(root.join("serial.log")).unwrap_or_default();
        if String::from_utf8_lossy(&log).contains("CommandTimeout") { break; }
        std::thread::sleep(Duration::from_secs(2));
    }
    let mut f = std::fs::File::create(out)?;
    for sample in 0..3 {
        writeln!(f, "sample {sample}, elapsed {:?}", start.elapsed())?;
        let mut q = Qmp::connect(&root.join("qmp.sock"), Duration::from_secs(5))?;
        for cmd in ["query-status", "query-blockstats", "query-pci"] {
            writeln!(f, "{cmd}: {:?}", q.execute(cmd, json!({})))?;
        }
        let pci = q.execute("query-pci", json!({}))?;
        if let Some(buses) = pci.as_array() {
            for dev in buses.iter().flat_map(|b| b["devices"].as_array().into_iter().flatten()) {
                if dev["qdev_id"] != "sata" { continue; }
                for r in dev["regions"].as_array().into_iter().flatten() {
                    if r["bar"] == 5 && let Some(addr) = r["address"].as_u64() {
                        let cmd = format!("xp /128wx 0x{addr:x}");
                        writeln!(f, "AHCI MMIO: {:?}", q.execute("human-monitor-command", json!({"command-line": cmd})))?;
                    }
                }
            }
        }
        for cmd in ["info irq", "info pic", "info lapic", "info registers -a", "info block", "info qtree"] {
            writeln!(f, "{cmd}: {:?}", q.execute("human-monitor-command", json!({"command-line": cmd})))?;
        }
        f.flush()?;
        std::thread::sleep(Duration::from_secs(10));
    }
    Ok(())
}
