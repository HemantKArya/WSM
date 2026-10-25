//! Shrink an OpenCore image (same files, smaller FAT partition): oc_repack SRC DST [MiB]
use anyhow::Result;
use std::path::PathBuf;

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(a.len() >= 2, "usage: oc_repack SRC DST [MiB]");
    let mib = a.get(2).map(|s| s.parse()).transpose()?.unwrap_or(64);
    wsm_core::opencore::repack(&PathBuf::from(&a[0]), &PathBuf::from(&a[1]), mib)?;
    println!("{} -> {} ({mib} MiB)", a[0], a[1]);
    Ok(())
}
