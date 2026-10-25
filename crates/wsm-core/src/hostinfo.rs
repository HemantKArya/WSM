//! Host probing for `wsm doctor` and OpenCore variant selection. Nothing here needs admin.
use std::process::Command;

/// A detached daemon must not inherit the caller's capture pipes through its launcher.
/// Commands with Stdio::inherit still receive their own explicit duplicated standard handles.
pub fn private_standard_handles() {
    #[cfg(windows)]
    unsafe {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetStdHandle(which: u32) -> isize;
            fn SetHandleInformation(handle: isize, mask: u32, flags: u32) -> i32;
        }
        for which in [-10i32, -11, -12] {
            let handle = GetStdHandle(which as u32);
            if handle != 0 && handle != -1 { SetHandleInformation(handle, 1, 0); }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuVendor {
    Intel,
    Amd,
    Other,
}

impl CpuVendor {
    /// Name used for the OpenCore template file (`opencore-<name>.img`).
    pub fn oc_variant(self) -> &'static str {
        match self {
            CpuVendor::Intel => "intel",
            _ => "amd",
        }
    }
}

pub fn cpu_vendor() -> CpuVendor {
    let id = std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_default();
    if id.contains("GenuineIntel") {
        CpuVendor::Intel
    } else if id.contains("AuthenticAMD") {
        CpuVendor::Amd
    } else {
        CpuVendor::Other
    }
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let mut c = Command::new(cmd);
    c.args(args);
    let o = crate::ssh::hidden(&mut c).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

pub fn gpus() -> Vec<String> {
    run("powershell.exe", &["-NoProfile", "-Command", "(Get-CimInstance Win32_VideoController).Name"])
        .map(|s| s.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default()
}

pub fn free_disk_gb(path: &std::path::Path) -> Option<u64> {
    let drive = path.display().to_string().chars().next()?;
    let out = run(
        "powershell.exe",
        &["-NoProfile", "-Command", &format!("[math]::Floor((Get-PSDrive {drive}).Free/1GB)")],
    )?;
    out.parse().ok()
}

/// WHPX usable? Asks the shipped QEMU, which is the only answer that matters.
pub fn whpx_available(qemu: &std::path::Path) -> bool {
    run(&qemu.display().to_string(), &["-accel", "help"]).map(|s| s.contains("whpx")).unwrap_or(false)
}

const GPU_PREF_KEY: &str = r"HKCU\Software\Microsoft\DirectX\UserGpuPreferences";

pub fn gpu_preference_set(exe: &std::path::Path) -> bool {
    run("reg", &["query", GPU_PREF_KEY, "/v", &exe.display().to_string()])
        .map(|s| s.contains("GpuPreference=2"))
        .unwrap_or(false)
}

/// Per-user "High performance" graphics preference (the Settings > Display > Graphics entry).
pub fn set_gpu_preference(exe: &std::path::Path) -> bool {
    let mut c = Command::new("reg");
    c.args(["add", GPU_PREF_KEY, "/v", &exe.display().to_string(), "/t", "REG_SZ", "/d", "GpuPreference=2;", "/f"]);
    crate::ssh::hidden(&mut c).output().map(|o| o.status.success()).unwrap_or(false)
}

pub fn remove_gpu_preference(exe: &std::path::Path) -> bool {
    let mut c = Command::new("reg");
    c.args(["delete", GPU_PREF_KEY, "/v", &exe.display().to_string(), "/f"]);
    crate::ssh::hidden(&mut c).output().map(|o| o.status.success()).unwrap_or(false)
}

pub fn winfsp_installed() -> bool {
    std::path::Path::new(r"C:\Program Files (x86)\WinFsp\bin").exists()
        || std::path::Path::new(r"C:\Program Files\WinFsp\bin").exists()
}

/// Hide the native display while the guest pre-warms; expose it only for `wsm gui start`.
#[cfg(windows)]
pub fn set_vm_window_visible(pid: u32, visible: bool) -> bool {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn EnumWindows(callback: unsafe extern "system" fn(isize, isize) -> i32, data: isize) -> i32;
        fn GetWindowThreadProcessId(window: isize, pid: *mut u32) -> u32;
        fn ShowWindow(window: isize, command: i32) -> i32;
        fn GetWindowTextLengthW(window: isize) -> i32;
    }
    struct Args { pid: u32, visible: bool, found: bool }
    unsafe extern "system" fn visit(window: isize, data: isize) -> i32 {
        unsafe {
            let a = &mut *(data as *mut Args);
            let mut owner = 0;
            GetWindowThreadProcessId(window, &mut owner);
            // Ignore the process's untitled helper/message windows.
            if owner == a.pid && GetWindowTextLengthW(window) > 0 {
                ShowWindow(window, if a.visible { 9 } else { 0 });
                a.found = true;
            }
        }
        1
    }
    let mut args = Args { pid, visible, found: false };
    unsafe { EnumWindows(visit, &mut args as *mut Args as isize); }
    args.found
}

#[cfg(not(windows))]
pub fn set_vm_window_visible(_pid: u32, _visible: bool) -> bool { false }
