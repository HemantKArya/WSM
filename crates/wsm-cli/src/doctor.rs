use anyhow::Result;
use wsm_core::hostinfo;
use wsm_core::paths;
use wsm_core::provider::Runtime;

fn line(ok: bool, what: &str, detail: &str) {
    println!("[{}] {what}: {detail}", if ok { " ok " } else { "FAIL" });
}

pub fn run(fix: bool) -> Result<i32> {
    let home = paths::home()?;
    let rt = Runtime::new(paths::runtime_dir(&home));
    let mut bad = 0;
    let mut check = |ok: bool, what: &str, detail: &str| {
        if !ok { bad += 1; }
        line(ok, what, detail);
    };

    let missing = rt.missing();
    check(missing.is_empty(), "runtime", &if missing.is_empty() { rt.dir.display().to_string() } else { format!("missing {missing:?} (extract wsm-setup.zip)") });
    let whpx = hostinfo::whpx_available(&rt.qemu());
    check(whpx, "WHPX", if whpx { "available to QEMU" } else { "not available: enable 'Windows Hypervisor Platform' in Windows Features (needs admin + reboot)" });
    let v = hostinfo::cpu_vendor();
    check(v != hostinfo::CpuVendor::Other, "CPU", &format!("{v:?} -> OpenCore variant '{}'", v.oc_variant()));
    let gpus = hostinfo::gpus();
    check(!gpus.is_empty(), "GPUs", &gpus.join("; "));
    let pref = hostinfo::gpu_preference_set(&rt.qemu());
    if !pref && fix {
        let done = hostinfo::set_gpu_preference(&rt.qemu());
        check(done, "GPU preference", "set High performance for QEMU (per-user registry, no admin)");
    } else {
        // not fatal: only GPU instances need it
        line(pref, "GPU preference", if pref { "High performance set for QEMU" } else { "not set; run `wsm doctor --fix` to set it for QEMU" });
    }
    let free = hostinfo::free_disk_gb(&home);
    check(free.is_none_or(|g| g >= 30), "disk", &format!("{} GB free on the runtime drive (base 15-20 GB + instances)", free.map(|g| g.to_string()).unwrap_or("?".into())));
    // `wsm mount` uses Windows' WebDAV redirector; the WebClient service starts on demand for a normal user.
    let mut sc = std::process::Command::new("sc.exe");
    sc.args(["query", "WebClient"]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    let webclient = wsm_core::ssh::hidden(&mut sc).status().is_ok_and(|s| s.success());
    line(webclient, "WebDAV client", if webclient { "WebClient service present (wsm mount available)" } else {
        "WebClient service missing: `wsm mount` unavailable (Windows optional feature 'WebDAV Redirector'); `wsm cp` still works" });
    if hostinfo::winfsp_installed() { line(true, "WinFsp/SSHFS-Win", "found (wsm mount --sshfs available)"); }
    let ssh = wsm_core::ssh::ssh_exe("ssh");
    check(ssh.exists(), "OpenSSH client", &ssh.display().to_string());
    println!("wsm {}", env!("CARGO_PKG_VERSION"));
    Ok(if bad == 0 { 0 } else { 1 })
}
