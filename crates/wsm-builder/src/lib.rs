//! wsm-builder: base image management (import/export/verify) and the reproducible builder (build.rs).
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

pub mod build;

pub fn sha256_file(p: &Path) -> Result<String> {
    let mut f = std::fs::File::open(p).with_context(|| format!("opening {}", p.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 8 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// `qemu-img info --output=json`
pub fn img_info(qemu_img: &Path, p: &Path) -> Result<serde_json::Value> {
    let o = Command::new(qemu_img).args(["info", "--output=json"]).arg(p).output()?;
    anyhow::ensure!(o.status.success(), "qemu-img info failed: {}", String::from_utf8_lossy(&o.stderr));
    Ok(serde_json::from_slice(&o.stdout)?)
}

/// A distributable base must be one self-contained qcow2 (no backing file).
pub fn check_generic(qemu_img: &Path, p: &Path) -> Result<()> {
    let i = img_info(qemu_img, p)?;
    anyhow::ensure!(i["format"] == "qcow2", "{} is not qcow2", p.display());
    anyhow::ensure!(i.get("backing-filename").is_none(), "{} has a backing file; flatten it first", p.display());
    Ok(())
}

fn sidecar(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".sha256");
    PathBuf::from(s)
}

fn expected_from_sidecar(p: &Path) -> Option<String> {
    std::fs::read_to_string(sidecar(p)).ok()?.split_whitespace().next().map(|s| s.to_lowercase())
}

/// File-name prefix of a base: lite bases (BaseSystem-derived) carry a data-disk template next to them.
fn base_prefix(name: &str) -> &'static str {
    if name.starts_with("base-lite-") { "base-lite-tahoe-26.3-" } else { "base-tahoe-26.3-" }
}

/// `<dir>/<stem>-data.qcow2` for `<dir>/<stem>.qcow2`.
pub fn data_template(base: &Path) -> PathBuf {
    base.with_file_name(base.file_name().unwrap().to_string_lossy().replace(".qcow2", "-data.qcow2"))
}

fn set_readonly(p: &Path, ro: bool) -> Result<()> {
    let mut pm = std::fs::metadata(p)?.permissions();
    #[allow(clippy::permissions_set_readonly_false)] // Windows read-only attribute, not Unix mode bits.
    pm.set_readonly(ro);
    std::fs::set_permissions(p, pm)?;
    Ok(())
}

pub fn export(base: &Path, dest: &Path) -> Result<String> {
    let sum = sha256_file(base)?;
    std::fs::copy(base, dest)?;
    std::fs::write(sidecar(dest), format!("{sum}  {}\n", dest.file_name().unwrap().to_string_lossy()))?;
    let data = data_template(base);
    if base_prefix(&base.file_name().unwrap().to_string_lossy()).starts_with("base-lite-") && data.exists() {
        let ddest = data_template(dest);
        let dsum = sha256_file(&data)?;
        std::fs::copy(&data, &ddest)?;
        std::fs::write(sidecar(&ddest), format!("{dsum}  {}\n", ddest.file_name().unwrap().to_string_lossy()))?;
    }
    Ok(sum)
}

struct ImportStage(PathBuf);
impl Drop for ImportStage {
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); }
}

/// Import a base from a file path or http(s) URL; an expected SHA-256 is mandatory.
pub fn import(src: &str, bases_dir: &Path, expect: Option<&str>, qemu_img: &Path) -> Result<PathBuf> {
    let base = import_one(src, bases_dir, expect, qemu_img)?;
    // A lite base needs its data-disk template, verified against its own sidecar (local sources only).
    let remote = src.starts_with("http://") || src.starts_with("https://");
    if base.file_name().unwrap().to_string_lossy().starts_with("base-lite-") && !remote {
        let sdata = data_template(Path::new(src));
        let ddest = data_template(&base);
        if !ddest.exists() {
            let dexp = expected_from_sidecar(&sdata).context("lite base: data template or its .sha256 sidecar missing next to the source")?;
            anyhow::ensure!(sha256_file(&sdata)? == dexp, "data template SHA-256 mismatch");
            std::fs::copy(&sdata, &ddest)?;
            set_readonly(&ddest, true)?;
            std::fs::write(sidecar(&ddest), format!("{dexp}  {}\n", ddest.file_name().unwrap().to_string_lossy()))?;
        }
    }
    Ok(base)
}

fn import_one(src: &str, bases_dir: &Path, expect: Option<&str>, qemu_img: &Path) -> Result<PathBuf> {
    let prefix = base_prefix(src.rsplit(['/', '\\']).next().unwrap_or(src));
    std::fs::create_dir_all(bases_dir)?;
    let staged = bases_dir.join(format!("import-{}.part", wsm_core::provider::random_hex(8)));
    let _cleanup = ImportStage(staged.clone());
    let mut expected = expect.map(|s| s.to_lowercase());
    let remote = src.starts_with("http://") || src.starts_with("https://");
    if remote {
        if expected.is_none() {
            let o = Command::new("curl.exe").args(["-fsL", "--connect-timeout", "20", "--max-time", "60", &format!("{src}.sha256")]).output()?;
            if o.status.success() {
                expected = String::from_utf8_lossy(&o.stdout).split_whitespace().next().map(str::to_lowercase);
            }
        }
    } else {
        let p = Path::new(src);
        if expected.is_none() {
            expected = expected_from_sidecar(p);
        }
    }
    let expected = expected.context("no expected SHA-256: supply --sha256 or a .sha256 sidecar")?;
    anyhow::ensure!(expected.len() == 64 && expected.bytes().all(|b| b.is_ascii_hexdigit()), "expected SHA-256 must be 64 hexadecimal characters");
    let known = bases_dir.join(format!("{prefix}{}.qcow2", &expected[..12]));
    if !remote && known.exists() {
        anyhow::ensure!(sha256_file(Path::new(src))? == expected, "import source SHA-256 mismatch");
        anyhow::ensure!(sha256_file(&known)? == expected, "existing base SHA-256 mismatch");
        check_generic(qemu_img, &known)?;
        return Ok(known);
    }
    // A read-only local base (how bases are stored and exported) on the same volume is linked, not
    // copied: verified first, so nothing is staged and no space is used. Instances only ever open a
    // base read-only through their overlays, so sharing the file is safe.
    if !remote && !known.exists() && std::fs::metadata(src).is_ok_and(|m| m.permissions().readonly()) {
        let src = Path::new(src);
        anyhow::ensure!(sha256_file(src)? == expected, "import source SHA-256 mismatch");
        check_generic(qemu_img, src)?;
        if std::fs::hard_link(src, &known).is_ok() {
            std::fs::write(sidecar(&known), format!("{expected}  {}\n", known.file_name().unwrap().to_string_lossy()))?;
            return Ok(known);
        }
    }
    if remote {
        let st = Command::new("curl.exe").args(["-fL", "--retry", "3", "--connect-timeout", "20", "-o"]).arg(&staged).arg(src).status()?;
        anyhow::ensure!(st.success(), "download failed");
    } else {
        std::fs::copy(src, &staged).with_context(|| format!("copying {src}"))?;
        // Windows CopyFile also copies the source's read-only attribute. The staging file must be
        // removable on validation failure; the final base is marked read-only only after verification.
        #[cfg(windows)]
        #[allow(clippy::permissions_set_readonly_false)] // Windows read-only attribute, not Unix mode bits.
        {
            let mut permissions = std::fs::metadata(&staged)?.permissions();
            permissions.set_readonly(false);
            std::fs::set_permissions(&staged, permissions)?;
        }
    }
    let sum = sha256_file(&staged)?;
    if expected != sum {
            let _ = std::fs::remove_file(&staged);
            bail!("sha256 mismatch: expected {expected}, got {sum}");
    }
    if let Err(e) = check_generic(qemu_img, &staged) {
        let _ = std::fs::remove_file(&staged);
        return Err(e);
    }
    let dest = bases_dir.join(format!("{prefix}{}.qcow2", &sum[..12]));
    if dest.exists() {
        anyhow::ensure!(sha256_file(&dest)? == sum, "existing base has a different checksum");
        std::fs::remove_file(&staged)?;
    } else {
        std::fs::rename(&staged, &dest)?;
    }
    let mut permissions = std::fs::metadata(&dest)?.permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&dest, permissions)?;
    std::fs::write(sidecar(&dest), format!("{sum}  {}\n", dest.file_name().unwrap().to_string_lossy()))?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vector() {
        let d = std::env::temp_dir().join("wsm-sha-test.bin");
        std::fs::write(&d, b"abc").unwrap();
        assert_eq!(sha256_file(&d).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let _ = std::fs::remove_file(d);
    }

    #[test]
    fn rejects_missing_and_invalid_hash_before_copying() {
        let dir = std::env::temp_dir().join(format!("wsm-import-test-{}", wsm_core::provider::random_hex(4)));
        for hash in [None, Some("invalid")] {
            let error = import("does-not-exist.qcow2", &dir, hash, Path::new("no-qemu-needed")).unwrap_err();
            assert!(error.to_string().contains("SHA-256"), "{error}");
            assert!(std::fs::read_dir(&dir).unwrap().next().is_none());
        }
        std::fs::remove_dir(&dir).unwrap();
    }
}
