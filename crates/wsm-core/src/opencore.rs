//! Per-instance OpenCore image: the shipped template (one per host CPU vendor) with this instance's
//! SMBIOS identity and the AMD_Vanilla `cpuid_cores_per_package` patch set to its vCPU count
//! (a mismatch hangs XNU while starting the application processors).
use anyhow::{Context, Result, bail};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// A window onto one partition of the image file, so fatfs sees a bare filesystem.
struct Part {
    f: File,
    start: u64,
    len: u64,
    pos: u64,
}

impl Read for Part {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = buf.len().min(self.len.saturating_sub(self.pos) as usize);
        self.f.seek(SeekFrom::Start(self.start + self.pos))?;
        let k = self.f.read(&mut buf[..n])?;
        self.pos += k as u64;
        Ok(k)
    }
}

impl Write for Part {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = buf.len().min(self.len.saturating_sub(self.pos) as usize);
        self.f.seek(SeekFrom::Start(self.start + self.pos))?;
        let k = self.f.write(&buf[..n])?;
        self.pos += k as u64;
        Ok(k)
    }
    fn flush(&mut self) -> std::io::Result<()> { self.f.flush() }
}

impl Seek for Part {
    fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
        let np = match p {
            SeekFrom::Start(x) => x as i64,
            SeekFrom::End(x) => self.len as i64 + x,
            SeekFrom::Current(x) => self.pos as i64 + x,
        };
        if np < 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "seek before start"));
        }
        self.pos = np as u64;
        Ok(self.pos)
    }
}

/// First FAT partition from the MBR, or the whole image if it is a bare FAT volume.
fn fat_partition(f: &mut File) -> Result<(u64, u64)> {
    let mut mbr = [0u8; 512];
    f.seek(SeekFrom::Start(0))?;
    f.read_exact(&mut mbr)?;
    let total = f.metadata()?.len();
    if &mbr[54..59] == b"FAT16" || &mbr[54..59] == b"FAT12" || &mbr[82..87] == b"FAT32" {
        return Ok((0, total));
    }
    anyhow::ensure!(mbr[510] == 0x55 && mbr[511] == 0xAA, "no MBR signature");
    for i in 0..4 {
        let e = &mbr[446 + 16 * i..462 + 16 * i];
        if matches!(e[4], 0x01 | 0x04 | 0x06 | 0x0B | 0x0C | 0x0E | 0xEF) {
            let lba = u32::from_le_bytes(e[8..12].try_into()?) as u64;
            let n = u32::from_le_bytes(e[12..16].try_into()?) as u64;
            return Ok((lba * 512, n * 512));
        }
    }
    bail!("no FAT partition in the OpenCore image")
}

pub struct Identity<'a> {
    pub serial: &'a str,
    pub mlb: &'a str,
    pub uuid: &'a str,
    pub rom: [u8; 6],
}

/// Apply this instance's settings to a parsed config.plist.
pub fn patch_config(cfg: &mut plist::Value, vcpus: u32, id: &Identity) -> Result<()> {
    let root = cfg.as_dictionary_mut().context("config.plist root")?;
    let mut cores_patched = false;
    if let Some(patches) = root
        .get_mut("Kernel")
        .and_then(|k| k.as_dictionary_mut())
        .and_then(|k| k.get_mut("Patch"))
        .and_then(|p| p.as_array_mut())
    {
        for p in patches {
            let Some(d) = p.as_dictionary_mut() else { continue };
            let is_cores = d.get("Comment").and_then(|c| c.as_string()).is_some_and(|c| c.contains("cores_per_package"));
            if is_cores {
                // mov edx, imm32
                d.insert("Replace".into(), plist::Value::Data(vec![0xBA, vcpus as u8, 0, 0, 0]));
                cores_patched = true;
            }
        }
    }
    let g = root
        .get_mut("PlatformInfo")
        .and_then(|p| p.as_dictionary_mut())
        .and_then(|p| p.get_mut("Generic"))
        .and_then(|g| g.as_dictionary_mut())
        .context("PlatformInfo/Generic missing")?;
    g.insert("SystemSerialNumber".into(), id.serial.into());
    g.insert("MLB".into(), id.mlb.into());
    g.insert("SystemUUID".into(), id.uuid.to_uppercase().into());
    g.insert("ROM".into(), plist::Value::Data(id.rom.to_vec()));
    let _ = cores_patched; // Intel templates carry no topology patch
    Ok(())
}

/// The Apple boot-args NVRAM variable OpenCore writes at every boot.
const APPLE_BOOT_VAR_GUID: &str = "7C436110-AB2A-4BBB-A880-FE41995C9F82";

/// Boot the default entry without showing the picker (unattended builds).
pub fn no_picker(cfg: &mut plist::Value) -> Result<()> {
    let b = cfg
        .as_dictionary_mut()
        .and_then(|r| r.get_mut("Misc"))
        .and_then(|m| m.as_dictionary_mut())
        .and_then(|m| m.get_mut("Boot"))
        .and_then(|b| b.as_dictionary_mut())
        .context("Misc/Boot missing in config.plist")?;
    b.insert("ShowPicker".into(), false.into());
    b.insert("Timeout".into(), plist::Value::Integer(0.into()));
    Ok(())
}

/// SIP setting OpenCore writes when NVRAM has none (fresh OVMF vars). Lite instances boot BaseSystem,
/// whose opendirectoryd only serves user lookups (libinfo) with SIP relaxed (0x67, round 1 X3).
pub fn set_csr(cfg: &mut plist::Value, v: u32) -> Result<()> {
    let g = cfg
        .as_dictionary_mut()
        .and_then(|r| r.get_mut("NVRAM"))
        .and_then(|n| n.as_dictionary_mut())
        .and_then(|n| n.get_mut("Add"))
        .and_then(|a| a.as_dictionary_mut())
        .and_then(|a| a.get_mut(APPLE_BOOT_VAR_GUID))
        .and_then(|g| g.as_dictionary_mut())
        .context("NVRAM/Add/<apple boot GUID> missing in config.plist")?;
    g.insert("csr-active-config".into(), plist::Value::Data(v.to_le_bytes().to_vec()));
    Ok(())
}

/// Append `extra` to the template's boot-args (e.g. `-s` for the builder's single-user stage).
pub fn add_boot_args(cfg: &mut plist::Value, extra: &str) -> Result<()> {
    let v = cfg
        .as_dictionary_mut()
        .and_then(|r| r.get_mut("NVRAM"))
        .and_then(|n| n.as_dictionary_mut())
        .and_then(|n| n.get_mut("Add"))
        .and_then(|a| a.as_dictionary_mut())
        .and_then(|a| a.get_mut(APPLE_BOOT_VAR_GUID))
        .and_then(|g| g.as_dictionary_mut())
        .context("NVRAM/Add/<apple boot GUID> missing in config.plist")?;
    let cur = v.get("boot-args").and_then(|b| b.as_string()).unwrap_or("").to_string();
    v.insert("boot-args".into(), format!("{cur} {extra}").trim().to_string().into());
    Ok(())
}

pub fn make_instance_image(template: &Path, out: &Path, vcpus: u32, id: &Identity) -> Result<()> {
    // This VM belongs to one WSM instance. The template's interactive picker adds a measured
    // 10-second timeout before macOS starts and has no useful choice for an instance overlay.
    // The template logs to screen, serial and a file on the ESP (Target 0x4B); measured: the kernel
    // starts 0.5 s earlier with logging off. Kernel output still reaches the serial log.
    make_image(template, out, vcpus, id, |cfg| {
        no_picker(cfg)?;
        set_log_target(cfg, 0)
    })
}

/// Like `make_instance_image`, then `edit` adjusts the config further (the base builder's
/// single-user boot-args and no picker).
pub fn make_image(
    template: &Path,
    out: &Path,
    vcpus: u32,
    id: &Identity,
    edit: impl FnOnce(&mut plist::Value) -> Result<()>,
) -> Result<()> {
    std::fs::copy(template, out).with_context(|| format!("copying {}", template.display()))?;
    edit_config(out, |cfg| {
        patch_config(cfg, vcpus, id)?;
        edit(cfg)
    })
}

/// Rewrite `EFI/OC/config.plist` inside an existing OpenCore image.
pub fn edit_config(image: &Path, edit: impl FnOnce(&mut plist::Value) -> Result<()>) -> Result<()> {
    let mut f = OpenOptions::new().read(true).write(true).open(image)?;
    let (start, len) = fat_partition(&mut f)?;
    let fs = fatfs::FileSystem::new(Part { f, start, len, pos: 0 }, fatfs::FsOptions::new())
        .context("opening the OpenCore FAT volume")?;
    let dir = fs.root_dir();
    let mut raw = Vec::new();
    dir.open_file("EFI/OC/config.plist")?.read_to_end(&mut raw)?;
    let mut cfg = plist::Value::from_reader(std::io::Cursor::new(&raw)).context("parsing config.plist")?;
    edit(&mut cfg)?;
    let mut xml = Vec::new();
    cfg.to_writer_xml(&mut xml)?;
    let mut w = dir.create_file("EFI/OC/config.plist")?;
    w.truncate()?;
    w.write_all(&xml)?;
    w.flush()?;
    drop(w);
    drop(dir);
    fs.unmount()?;
    Ok(())
}

/// Copy an OpenCore image's files into a fresh `mib`-MiB image with the same MBR layout. The
/// round-2 template is a 503 MiB FAT16 partition holding <1 MB of files, copied for every instance
/// and builder run; the files, not the free space, are what OVMF and OpenCore need.
pub fn repack(src: &Path, dst: &Path, mib: u64) -> Result<()> {
    let mut f = File::open(src)?;
    let mut mbr = [0u8; 512];
    f.read_exact(&mut mbr)?;
    let (start, len) = fat_partition(&mut f)?;
    anyhow::ensure!(start == 63 * 512, "unexpected partition start {start}");
    let from = fatfs::FileSystem::new(Part { f, start, len, pos: 0 }, fatfs::FsOptions::new())?;
    let mut label = [b' '; 11];
    for (d, s) in label.iter_mut().zip(from.volume_label_as_bytes()) { *d = *s; }
    let total = mib << 20;
    let plen = total - start;
    for e in 0..4 {
        let o = 446 + 16 * e;
        if matches!(mbr[o + 4], 0x01 | 0x04 | 0x06 | 0x0B | 0x0C | 0x0E | 0xEF) {
            mbr[o + 12..o + 16].copy_from_slice(&((plen / 512) as u32).to_le_bytes());
            break;
        }
    }
    let mut out = OpenOptions::new().create(true).read(true).write(true).truncate(true).open(dst)?;
    out.set_len(total)?;
    out.write_all(&mbr)?;
    fatfs::format_volume(Part { f: out.try_clone()?, start, len: plen, pos: 0 },
        fatfs::FormatVolumeOptions::new().volume_label(label))?;
    let to = fatfs::FileSystem::new(Part { f: out, start, len: plen, pos: 0 }, fatfs::FsOptions::new())?;
    fn copy<A: fatfs::ReadWriteSeek, B: fatfs::ReadWriteSeek>(a: &fatfs::Dir<A>, b: &fatfs::Dir<B>) -> Result<()> {
        for e in a.iter() {
            let e = e?;
            let name = e.file_name();
            if name == "." || name == ".." { continue; }
            if e.is_dir() {
                copy(&e.to_dir(), &b.create_dir(&name)?)?;
            } else {
                let mut data = Vec::new();
                e.to_file().read_to_end(&mut data)?;
                let mut w = b.create_file(&name)?;
                w.truncate()?;
                w.write_all(&data)?;
            }
        }
        Ok(())
    }
    copy(&from.root_dir(), &to.root_dir())?;
    to.unmount()?;
    Ok(())
}

/// `Misc/Debug/Target`: where OpenCore writes its log (0 = nowhere).
pub fn set_log_target(cfg: &mut plist::Value, target: i64) -> Result<()> {
    let d = cfg
        .as_dictionary_mut()
        .and_then(|r| r.get_mut("Misc"))
        .and_then(|m| m.as_dictionary_mut())
        .and_then(|m| m.get_mut("Debug"))
        .and_then(|b| b.as_dictionary_mut())
        .context("Misc/Debug missing in config.plist")?;
    d.insert("Target".into(), plist::Value::Integer(target.into()));
    Ok(())
}

/// Remove one word from the boot-args (e.g. `-v`).
pub fn remove_boot_arg(cfg: &mut plist::Value, word: &str) -> Result<()> {
    let v = cfg
        .as_dictionary_mut()
        .and_then(|r| r.get_mut("NVRAM"))
        .and_then(|n| n.as_dictionary_mut())
        .and_then(|n| n.get_mut("Add"))
        .and_then(|a| a.as_dictionary_mut())
        .and_then(|a| a.get_mut(APPLE_BOOT_VAR_GUID))
        .and_then(|g| g.as_dictionary_mut())
        .context("NVRAM/Add/<apple boot GUID> missing in config.plist")?;
    let cur = v.get("boot-args").and_then(|b| b.as_string()).unwrap_or("").to_string();
    let kept: Vec<&str> = cur.split_whitespace().filter(|w| *w != word).collect();
    v.insert("boot-args".into(), kept.join(" ").into());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patches_cores_and_identity() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>Kernel</key><dict><key>Patch</key><array>
<dict><key>Comment</key><string>algrey | Force cpuid_cores_per_package to constant (user-specified) | 13.3+</string>
<key>Replace</key><data>uggAAAA=</data></dict></array></dict>
<key>PlatformInfo</key><dict><key>Generic</key><dict><key>MLB</key><string>x</string></dict></dict>
</dict></plist>"#;
        let mut v = plist::Value::from_reader(std::io::Cursor::new(&xml[..])).unwrap();
        let id = Identity { serial: "C02ABCDEFGHJ", mlb: "C02123456789ABCDE", uuid: "aa-bb", rom: [1, 2, 3, 4, 5, 6] };
        patch_config(&mut v, 4, &id).unwrap();
        let d = v.as_dictionary().unwrap();
        let p = &d["Kernel"].as_dictionary().unwrap()["Patch"].as_array().unwrap()[0];
        assert_eq!(p.as_dictionary().unwrap()["Replace"].as_data().unwrap(), &[0xBA, 4, 0, 0, 0]);
        let g = d["PlatformInfo"].as_dictionary().unwrap()["Generic"].as_dictionary().unwrap();
        assert_eq!(g["MLB"].as_string().unwrap(), "C02123456789ABCDE");
        assert_eq!(g["SystemUUID"].as_string().unwrap(), "AA-BB");
    }

    #[test]
    fn appends_boot_args() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>NVRAM</key><dict><key>Add</key><dict>
<key>7C436110-AB2A-4BBB-A880-FE41995C9F82</key><dict><key>boot-args</key><string>-v keepsyms=1</string></dict>
</dict></dict></dict></plist>"#;
        let mut v = plist::Value::from_reader(std::io::Cursor::new(&xml[..])).unwrap();
        add_boot_args(&mut v, "-s").unwrap();
        let ba = v.as_dictionary().unwrap()["NVRAM"].as_dictionary().unwrap()["Add"].as_dictionary().unwrap()
            [APPLE_BOOT_VAR_GUID].as_dictionary().unwrap()["boot-args"].as_string().unwrap().to_string();
        assert_eq!(ba, "-v keepsyms=1 -s");
    }
}
