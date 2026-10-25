//! Boot-time experiments on one instance's OpenCore image (stopped instance only).
//! usage: oc_tweak <opencore.img> [target=N] [rmarg=WORD]... ; prints the resulting boot-args/Target
use anyhow::Result;
use std::path::PathBuf;
use wsm_core::opencore;

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let img = PathBuf::from(a.first().expect("opencore.img"));
    opencore::edit_config(&img, |cfg| {
        for op in &a[1..] {
            match op.split_once('=') {
                Some(("target", n)) => opencore::set_log_target(cfg, n.parse()?)?,
                Some(("rmarg", w)) => opencore::remove_boot_arg(cfg, w)?,
                _ => anyhow::bail!("unknown op {op}"),
            }
        }
        let d = cfg.as_dictionary().unwrap();
        let target = d["Misc"].as_dictionary().unwrap()["Debug"].as_dictionary().unwrap()["Target"].clone();
        let args = d["NVRAM"].as_dictionary().unwrap()["Add"].as_dictionary().unwrap()["7C436110-AB2A-4BBB-A880-FE41995C9F82"]
            .as_dictionary().unwrap()["boot-args"].clone();
        println!("Target={target:?} boot-args={args:?}");
        Ok(())
    })
}
