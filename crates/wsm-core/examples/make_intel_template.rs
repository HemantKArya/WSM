//! Produce the Intel firmware variant from the pinned AMD template, keeping its boot/GPU setup.
use anyhow::{Context, Result};
use std::path::PathBuf;
use wsm_core::opencore::{self, Identity};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).map(PathBuf::from).collect();
    anyhow::ensure!(args.len() == 2, "usage: make_intel_template AMD.img INTEL.img");
    let id = Identity { serial: "C02WSM000000", mlb: "C02WSM00000000000", uuid: "00000000-0000-4000-8000-000000000000", rom: [0x52, 0x54, 0, 0, 0, 0] };
    opencore::make_image(&args[0], &args[1], 4, &id, |cfg| {
        let kernel = cfg.as_dictionary_mut().context("root")?.get_mut("Kernel").context("Kernel")?.as_dictionary_mut().context("Kernel dict")?;
        let patches = kernel.get_mut("Patch").context("Patch")?.as_array_mut().context("patch array")?;
        patches.retain(|p| {
            p.as_dictionary().and_then(|p| p.get("Comment")).and_then(|c| c.as_string()) == Some("Enable early serial output on RELEASE kernel")
        });
        kernel.get_mut("Quirks").context("Quirks")?.as_dictionary_mut().context("Quirks dict")?.insert("ProvideCurrentCpuInfo".into(), true.into());
        opencore::no_picker(cfg)
    })
}
