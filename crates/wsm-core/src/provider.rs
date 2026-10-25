//! QemuProvider: instance manifest -> QEMU argv + environment, and process lifecycle.
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::manifest::Manifest;
use crate::paths::InstanceDirs;

pub fn random_hex(bytes: usize) -> String {
    // 128-bit seed from the OS-randomised HashMap hasher; enough for MAC/serial identity, not a secret.
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    while out.len() < bytes * 2 {
        let mut h = RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out.truncate(bytes * 2);
    out
}

/// Files the installed runtime provides (see `wsm-setup.zip`).
#[derive(Debug, Clone)]
pub struct Runtime {
    pub dir: PathBuf,
}

impl Runtime {
    pub fn new(dir: PathBuf) -> Self { Self { dir } }
    pub fn qemu(&self) -> PathBuf { self.dir.join("qemu").join("qemu-system-x86_64.exe") }
    pub fn qemu_img(&self) -> PathBuf { self.dir.join("qemu").join("qemu-img.exe") }
    pub fn bios_dir(&self) -> PathBuf { self.dir.join("qemu").join("pc-bios") }
    pub fn ovmf_code(&self) -> PathBuf { self.dir.join("firmware").join("OVMF_CODE.fd") }
    pub fn ovmf_vars(&self) -> PathBuf { self.dir.join("firmware").join("OVMF_VARS.fd") }
    pub fn rom(&self) -> PathBuf { self.dir.join("qemu").join("ReimsVgpu.rom") }
    /// 7-Zip (LGPL, shipped in the runtime): reads the installer ISO's HFS+ volume for the base builder.
    pub fn seven_zip(&self) -> PathBuf { self.dir.join("tools").join("7z.exe") }
    pub fn oc_template(&self, vendor: &str) -> PathBuf {
        self.dir.join("firmware").join(format!("opencore-{vendor}.img"))
    }
    pub fn missing(&self) -> Vec<PathBuf> {
        [self.qemu(), self.qemu_img(), self.ovmf_code(), self.ovmf_vars()]
            .into_iter()
            .filter(|p| !p.exists())
            .collect()
    }
}

pub fn fwd(p: &Path) -> String { p.display().to_string().replace('\\', "/") }

/// One CPU model for every profile and the base builder: the aljus7 OpenCore + our AMD patches are
/// validated against it, and a base installed under one model must boot under the same one.
pub const CPU_MODEL: &str = "Haswell-noTSX,stepping=3,vendor=GenuineIntel,+ssse3,+sse4.2,+popcnt,+avx,+avx2,+aes,+xsave,+xsaveopt,-arat,-pcid,-tsc-deadline,enforce";

/// The AppleSMC device macOS requires (QEMU's documented OSK string).
pub const APPLESMC: &str = "isa-applesmc,osk=ourhardworkbythesewordsguardedpleasedontsteal(c)AppleComputerInc";

/// QEMU command line. `incoming` adds `-incoming unix:<instance>/migrate.sock` for a resume.
pub fn build_argv(m: &Manifest, d: &InstanceDirs, rt: &Runtime, incoming: bool) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    let mut p = |s: &str| a.push(s.to_string());
    p("-L"); p(&fwd(&rt.bios_dir()));
    p("-name"); p(&format!("wsm-{}", m.name));
    p("-machine"); p("q35");
    p("-accel"); p(&m.accel);
    p("-cpu"); p(CPU_MODEL);
    p("-smp"); p(&format!("{n},sockets=1,cores={n},threads=1", n = m.vcpus));
    p("-m"); p(&format!("{}M", m.ram_mib));
    // No S3/S4 for any guest: macOS idle-sleeps a VM nobody is "using", which takes the NIC (and every
    // SSH session) down until something wakes it (round 3: `ACPI SLEEP … acpi_sleep_kernel` on serial).
    for g in ["ICH9-LPC.disable_s3=1", "ICH9-LPC.disable_s4=1"] {
        p("-global"); p(g);
    }
    if m.gpu {
        p("-global"); p("ICH9-LPC.acpi-pci-hotplug-with-bridge-support=off");
        p("-device"); p("pcie-root-port,bus=pcie.0,addr=0x1,slot=1,x-speed=16,x-width=32");
    }
    p("-device"); p(APPLESMC);
    p("-drive"); p(&format!("if=pflash,format=raw,readonly=on,file={}", fwd(&rt.ovmf_code())));
    p("-drive"); p(&format!("if=pflash,format=raw,file={}", fwd(&d.vars())));
    p("-smbios"); p(&format!("type=1,serial={},uuid={}", m.smbios_serial, m.smbios_uuid));
    p("-smbios"); p("type=2");
    p("-device"); p("ich9-ahci,id=sata,addr=0x2");
    p("-drive"); p(&format!("id=opencore,if=none,format=raw,snapshot=on,file={}", m.oc_image.replace('\\', "/")));
    p("-device"); p("ide-hd,bus=sata.2,drive=opencore,bootindex=1");
    p("-drive"); p(&format!("id=disk,if=none,format=qcow2,discard=unmap,detect-zeroes=unmap,file={}", fwd(&d.disk())));
    p("-device"); p("ide-hd,bus=sata.3,drive=disk,rotation_rate=1");
    if d.config_disk().exists() {
        p("-drive"); p(&format!("id=cfg,if=none,format=raw,snapshot=on,file={}", fwd(&d.config_disk())));
        p("-device"); p("ide-hd,bus=sata.4,drive=cfg");
    }
    if d.data_disk().exists() {
        p("-drive"); p(&format!("id=data,if=none,format=qcow2,discard=unmap,detect-zeroes=unmap,file={}", fwd(&d.data_disk())));
        p("-device"); p("ide-hd,bus=sata.5,drive=data,rotation_rate=1");
    }
    p("-netdev"); p(&format!("user,id=net0,ipv6=off,hostfwd=tcp:127.0.0.1:{}-:22", m.ssh_port));
    p("-device"); p(&format!("virtio-net-pci,netdev=net0,id=net0,mac={},addr=0x3", m.mac));
    if m.gpu {
        p("-vga"); p("none");
        let dev = serde_json::json!({
            "driver": "apple-gfx-pci", "id": "gfx", "bus": "pcie.0", "addr": "0x4", "displays": 1,
            "resolution": m.display, "romfile": fwd(&rt.rom()),
            "display-modes": [format!("{}@60", m.display), "1280x720@60"]
        });
        p("-device"); p(&dev.to_string());
    } else {
        p("-device"); p("VGA");
    }
    p("-device"); p("qemu-xhci,id=xhci,addr=0x5,p2=8,p3=6,conditional-intr-mapping=on");
    p("-device"); p("usb-kbd,id=kbd,bus=xhci.0");
    p("-device"); p("usb-tablet,id=tablet,bus=xhci.0");
    p("-display"); p("none");
    p("-monitor"); p("none");
    p("-chardev"); p(&format!("socket,id=ser0,path={},server=on,wait=off", fwd(&d.serial_sock())));
    p("-serial"); p("chardev:ser0");
    p("-msg"); p("timestamp=on");
    p("-qmp"); p(&format!("unix:{},server=on,wait=off", fwd(&d.qmp_sock())));
    p("-no-shutdown");
    if incoming {
        p("-incoming"); p(&format!("unix:{}", fwd(&d.migrate_sock())));
    }
    a
}

/// Environment for the QEMU process (never leaks into the user's session).
pub fn build_env(m: &Manifest, rt: &Runtime, logs: &Path) -> Vec<(String, String)> {
    let mut e = vec![
        ("WSM_WHPX_ALLOW_MIGRATION".to_string(), "1".to_string()),
        ("PATH".to_string(), format!("{};{}", rt.dir.join("qemu").display(), std::env::var("PATH").unwrap_or_default())),
        ("TMP".to_string(), logs.display().to_string()),
        ("TEMP".to_string(), logs.display().to_string()),
    ];
    if m.gpu {
        e.push(("REIMS_VGPU_WINDOW".into(), "1".into()));
        e.push(("REIMS_VGPU_LOG".into(), "info".into()));
        e.push(("REIMS_VGPU_STATS".into(), "1".into()));
        e.push(("REIMS_VGPU_LAB_LOG_DIR".into(), fwd(logs)));
    }
    e
}

/// Rotate `name.log` -> `name.log.1` .. keeping `keep` generations.
pub fn rotate_log(dir: &Path, name: &str, keep: usize) {
    for i in (1..keep).rev() {
        let _ = std::fs::rename(dir.join(format!("{name}.{i}")), dir.join(format!("{name}.{}", i + 1)));
    }
    let _ = std::fs::rename(dir.join(name), dir.join(format!("{name}.1")));
}

pub fn spawn(m: &Manifest, d: &InstanceDirs, rt: &Runtime, incoming: bool) -> Result<u32> {
    std::fs::create_dir_all(d.logs())?;
    rotate_log(&d.logs(), "qemu.err.log", 5);
    rotate_log(&d.logs(), "serial.log", 5);
    rotate_log(&d.logs(), "serial-timed.log", 5);
    let err = std::fs::File::create(d.logs().join("qemu.err.log"))?;
    let out = std::fs::File::create(d.logs().join("qemu.out.log"))?;
    let mut cmd = Command::new(rt.qemu());
    cmd.args(build_argv(m, d, rt, incoming))
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err);
    for (k, v) in build_env(m, rt, &d.logs()) {
        cmd.env(k, v);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP: QEMU must outlive the CLI/service console.
        cmd.creation_flags(0x0800_0000 | 0x0000_0200);
    }
    let child = cmd.spawn().with_context(|| format!("starting {}", rt.qemu().display()))?;
    let pid = child.id();
    std::fs::write(d.pid(), pid.to_string())?;
    #[cfg(windows)]
    if let Some(created) = process_identity(pid) {
        std::fs::write(d.pid().with_extension("identity"), created.to_string())?;
    }
    if m.gpu {
        std::thread::spawn(move || {
            for _ in 0..300 {
                if !pid_alive(pid) || crate::hostinfo::set_vm_window_visible(pid, false) { break; }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        });
    }
    Ok(pid)
}

pub fn pid_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        process_identity(pid).is_some()
    }
    #[cfg(not(windows))]
    {
    let mut c = Command::new("tasklist");
    c.args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"]);
    crate::ssh::hidden(&mut c)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(&format!("\"{pid}\"")))
        .unwrap_or(false)
    }
}

/// Query the process directly instead of launching tasklist for every `exec`.
/// Creation time also distinguishes a stale PID from a newly reused one.
#[cfg(windows)]
fn process_identity(pid: u32) -> Option<u64> {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn GetExitCodeProcess(h: *mut c_void, code: *mut u32) -> i32;
        fn GetProcessTimes(h: *mut c_void, created: *mut u64, exited: *mut u64, kernel: *mut u64, user: *mut u64) -> i32;
        fn CloseHandle(h: *mut c_void) -> i32;
    }
    unsafe {
        let h = OpenProcess(0x1000, 0, pid);
        if h.is_null() { return None; }
        let (mut code, mut created, mut exited, mut kernel, mut user) = (0, 0, 0, 0, 0);
        let live = GetExitCodeProcess(h, &mut code) != 0 && code == 259;
        let times = GetProcessTimes(h, &mut created, &mut exited, &mut kernel, &mut user) != 0;
        CloseHandle(h);
        (live && times).then_some(created)
    }
}

/// PID recorded for this instance if (and only if) that process still exists.
pub fn running_pid(d: &InstanceDirs) -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(d.pid()).ok()?.trim().parse().ok()?;
    #[cfg(windows)]
    if let Ok(recorded) = std::fs::read_to_string(d.pid().with_extension("identity"))
        && process_identity(pid) != recorded.trim().parse::<u64>().ok() {
            let _ = std::fs::remove_file(d.pid());
            return None;
    }
    if pid_alive(pid) { Some(pid) } else { let _ = std::fs::remove_file(d.pid()); None }
}

pub fn kill(pid: u32) {
    let mut c = Command::new("taskkill");
    c.args(["/F", "/PID", &pid.to_string()]);
    let _ = crate::ssh::hidden(&mut c).output();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(gpu: bool) -> (Manifest, InstanceDirs, Runtime) {
        let mut m = Manifest::new("default", "b.qcow2", gpu, 2222);
        m.oc_image = "C:/oc.img".into();
        (m, InstanceDirs::new(Path::new("C:/w"), "default"), Runtime::new("C:/rt".into()))
    }

    #[test]
    fn headless_argv_has_serial_socket_and_no_gfx() {
        let (m, d, rt) = fixture(false);
        let a = build_argv(&m, &d, &rt, false);
        assert!(a.iter().any(|x| x.starts_with("socket,id=ser0,path=C:/w/instances/default/serial.sock,server=on")));
        assert!(a.iter().any(|x| x.starts_with("unix:C:/w/instances/default/qmp.sock,server=on")));
        assert!(!a.iter().any(|x| x.contains("127.0.0.1") && !x.contains("hostfwd")), "only SSH may use loopback TCP");
        assert!(!a.iter().any(|x| x.contains("apple-gfx-pci")));
        assert!(!a.contains(&"-incoming".to_string()));
        assert!(a.iter().any(|x| x == "ICH9-LPC.disable_s3=1"), "headless guests must not be able to sleep");
    }

    #[test]
    fn gpu_argv_has_device_and_incoming() {
        let (m, d, rt) = fixture(true);
        let a = build_argv(&m, &d, &rt, true);
        assert!(a.iter().any(|x| x.contains("apple-gfx-pci")));
        assert!(a.windows(2).any(|w| w[0] == "-incoming" && w[1] == "unix:C:/w/instances/default/migrate.sock"));
        assert!(a.windows(2).any(|w| w[0] == "-m" && w[1] == "4096M"));
    }
}
