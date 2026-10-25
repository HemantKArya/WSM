use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub name: String,
    /// Read-only base image the instance disk overlays.
    pub base: String,
    pub ram_mib: u32,
    pub vcpus: u32,
    pub gpu: bool,
    /// "lite" for a BaseSystem-derived base with a separate data disk; empty or "full" otherwise.
    #[serde(default)]
    pub kind: String,
    /// Loopback port forwarded to the guest's sshd (key-only). QMP, serial and migration use
    /// owner-only socket files in the instance directory instead of ports.
    pub ssh_port: u16,
    pub mac: String,
    pub smbios_serial: String,
    pub smbios_uuid: String,
    #[serde(default)]
    pub smbios_mlb: String,
    pub user: String,
    pub idle_minutes: u32,
    /// OpenCore ESP image generated for this host's CPU vendor.
    pub oc_image: String,
    /// vCPU count the OpenCore image was generated for (its topology patch must match).
    #[serde(default)]
    pub oc_vcpus: u32,
    /// `-accel` value. In-kernel irqchip (the default) is the only mode this QEMU boots macOS with;
    /// restore works with it since the round-3 WHPX patches.
    pub accel: String,
    pub display: String,
}

impl Manifest {
    pub fn new(name: &str, base: &str, gpu: bool, ssh_port: u16) -> Self {
        let id = crate::provider::random_hex(16);
        Self {
            version: MANIFEST_VERSION,
            name: name.into(),
            base: base.into(),
            ram_mib: if gpu { 4096 } else { 2048 },
            // Cold boot is CPU-bound (measured, 2 GiB guest: 4 vCPU 23.6 s -> 8 vCPU 20.8 s to SSH).
            // Half the host's threads, 2..=8.
            vcpus: std::thread::available_parallelism().map_or(4, |n| (n.get() as u32 / 2).clamp(2, 8)),
            gpu,
            kind: String::new(),
            ssh_port,
            mac: format!("52:54:00:{}:{}:{}", &id[0..2], &id[2..4], &id[4..6]),
            smbios_serial: format!("C02{}", id[6..15].to_uppercase()),
            smbios_uuid: format!(
                "{}-{}-{}-{}-{}",
                &id[0..8], &id[8..12], &id[12..16], &id[16..20], &id[20..32]
            ),
            smbios_mlb: format!("C02{}", crate::provider::random_hex(8)[..14].to_uppercase()),
            user: "dev".into(),
            idle_minutes: 15,
            oc_image: String::new(),
            oc_vcpus: 0,
            accel: "whpx,hyperv=off".into(),
            display: "1920x1080".into(),
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let s = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let m: Manifest = toml::from_str(&s).context("parsing manifest")?;
        anyhow::ensure!(m.version == MANIFEST_VERSION, "unsupported manifest version {}", m.version);
        Ok(m)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        std::fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn rom(&self) -> [u8; 6] {
        let mut r = [0u8; 6];
        for (i, b) in self.mac.split(':').take(6).enumerate() {
            r[i] = u8::from_str_radix(b, 16).unwrap_or(0);
        }
        r
    }

    /// The GPU device loses its Vulkan device above this much guest RAM (round 2).
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(!(self.gpu && self.ram_mib > 4096), "GPU instances are capped at 4096 MiB RAM");
        anyhow::ensure!(self.ram_mib >= 1024, "RAM below 1024 MiB cannot boot");
        anyhow::ensure!((1..=16).contains(&self.vcpus), "vcpus must be 1..16");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_gpu_cap() {
        let mut m = Manifest::new("default", "base.qcow2", true, 2222);
        m.validate().unwrap();
        let t = toml::to_string_pretty(&m).unwrap();
        let back: Manifest = toml::from_str(&t).unwrap();
        assert_eq!(back.name, "default");
        m.ram_mib = 8192;
        assert!(m.validate().is_err());
    }

    #[test]
    fn identities_differ() {
        let a = Manifest::new("a", "b", false, 1);
        let b = Manifest::new("b", "b", false, 3);
        assert_ne!(a.mac, b.mac);
        assert_ne!(a.smbios_serial, b.smbios_serial);
        assert_ne!(a.smbios_uuid, b.smbios_uuid);
    }
}
