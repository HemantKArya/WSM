//! Config disk: a raw ustar archive padded to 1 MiB, read in the guest with `tar -xf /dev/diskN`
//! (FAT is unusable at that stage of boot; see RESULTS.md X3).
use anyhow::Result;
use std::path::Path;

fn octal(buf: &mut [u8], v: u64) {
    let s = format!("{:0width$o}\0", v, width = buf.len() - 1);
    buf.copy_from_slice(s.as_bytes());
}

fn header(name: &str, size: u64, mode: u32) -> [u8; 512] {
    let mut h = [0u8; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    octal(&mut h[100..108], mode as u64);
    octal(&mut h[108..116], 0);
    octal(&mut h[116..124], 0);
    octal(&mut h[124..136], size);
    octal(&mut h[136..148], 0);
    h[148..156].fill(b' ');
    h[156] = b'0';
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[265..269].copy_from_slice(b"root");
    h[297..301].copy_from_slice(b"root");
    let sum: u32 = h.iter().map(|b| *b as u32).sum();
    let s = format!("{sum:06o}\0 ");
    h[148..156].copy_from_slice(s.as_bytes());
    h
}

pub fn make_tar(files: &[(&str, &[u8], u32)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, data, mode) in files {
        out.extend_from_slice(&header(name, data.len() as u64, *mode));
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.extend_from_slice(&[0u8; 1024]);
    out.resize(out.len().div_ceil(1 << 20) * (1 << 20), 0);
    out
}

/// Write the per-instance config disk: user, hostname and the public key to authorise.
pub fn write_config_disk(path: &Path, user: &str, hostname: &str, pubkey: &str, host_key: Option<&str>) -> Result<()> {
    let cfg = serde_json::json!({"user": user, "hostname": hostname});
    let cfg_text = cfg.to_string();
    let mut files: Vec<(&str, &[u8], u32)> = vec![
        ("wsm-config.json", cfg_text.as_bytes(), 0o644),
        ("authorized_keys", pubkey.trim().as_bytes(), 0o600),
    ];
    if let Some(k) = host_key { files.push(("ssh_host_ed25519_key", k.as_bytes(), 0o600)); }
    let tar = make_tar(&files);
    std::fs::write(path, tar)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tar_is_block_aligned_and_checksummed() {
        let t = make_tar(&[("a", b"hello", 0o644)]);
        assert_eq!(t.len() % (1 << 20), 0);
        assert_eq!(&t[257..262], b"ustar");
        let mut h = t[..512].to_vec();
        let stored = u32::from_str_radix(std::str::from_utf8(&h[148..154]).unwrap(), 8).unwrap();
        h[148..156].fill(b' ');
        assert_eq!(stored, h.iter().map(|b| *b as u32).sum::<u32>());
        assert_eq!(&t[512..517], b"hello");
    }
}
