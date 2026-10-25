use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Runtime root: `WSM_HOME` (dev workspace and tests); else the directory of an installed package
/// (`package-manifest.json` next to wsm.exe/wsmd.exe), so a copy extracted anywhere is self-contained;
/// else `%LOCALAPPDATA%\wsm`.
pub fn home() -> Result<PathBuf> {
    if let Some(h) = std::env::var_os("WSM_HOME") {
        return Ok(PathBuf::from(h));
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf))
        && dir.join("package-manifest.json").is_file() {
        return Ok(dir);
    }
    let base = std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is not set")?;
    Ok(PathBuf::from(base).join("wsm"))
}

pub struct InstanceDirs {
    pub root: PathBuf,
}

impl InstanceDirs {
    pub fn new(home: &Path, name: &str) -> Self {
        Self { root: home.join("instances").join(name) }
    }
    pub fn manifest(&self) -> PathBuf { self.root.join("manifest.toml") }
    pub fn disk(&self) -> PathBuf { self.root.join("disk.qcow2") }
    pub fn vars(&self) -> PathBuf { self.root.join("OVMF_VARS.fd") }
    pub fn state(&self) -> PathBuf { self.root.join("state.bin") }
    pub fn state_meta(&self) -> PathBuf { self.root.join("state.json") }
    pub fn keys(&self) -> PathBuf { self.root.join("keys") }
    pub fn logs(&self) -> PathBuf { self.root.join("logs") }
    pub fn pid(&self) -> PathBuf { self.root.join("qemu.pid") }
    pub fn data_disk(&self) -> PathBuf { self.root.join("data.qcow2") }
    pub fn config_disk(&self) -> PathBuf { self.root.join("config.img") }
    /// Owner-only socket files for QEMU's control channels (see `afunix`).
    pub fn qmp_sock(&self) -> PathBuf { self.root.join("qmp.sock") }
    pub fn serial_sock(&self) -> PathBuf { self.root.join("serial.sock") }
    pub fn migrate_sock(&self) -> PathBuf { self.root.join("migrate.sock") }
}

pub fn runtime_dir(home: &Path) -> PathBuf { home.join("runtime") }
pub fn bases_dir(home: &Path) -> PathBuf { home.join("bases") }
