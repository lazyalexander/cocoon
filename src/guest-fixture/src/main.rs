//! Fixed Linux persistence acceptance fixture, never a general execution agent.
#[cfg(not(all(target_os = "linux", target_arch = "aarch64")))]
fn main() {
    eprintln!("fixture requires Linux ARM64; host execution is unavailable");
    std::process::exit(2);
}

#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
mod guest {
    use std::{
        ffi::CString,
        fs,
        io::Write,
        os::unix::{fs::PermissionsExt, process::CommandExt},
        process::Command,
    };
    unsafe extern "C" {
        fn mount(
            source: *const std::ffi::c_char,
            target: *const std::ffi::c_char,
            kind: *const std::ffi::c_char,
            flags: u64,
            data: *const u8,
        ) -> i32;
        fn syscall(number: std::ffi::c_long, ...) -> std::ffi::c_long;
        fn chroot(path: *const std::ffi::c_char) -> i32;
    }
    fn mount_fs(source: &str, target: &str, kind: &str) -> std::io::Result<()> {
        let source = CString::new(source)?;
        let target = CString::new(target)?;
        let kind = CString::new(kind)?;
        // SAFETY: all strings are NUL-terminated and alive for the synchronous call;
        // no mount-specific data is supplied. This binary only runs inside a guest.
        let result = unsafe {
            mount(
                source.as_ptr(),
                target.as_ptr(),
                kind.as_ptr(),
                0,
                std::ptr::null(),
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        if std::process::id() != 1 {
            return Err("fixture must be guest PID 1".into());
        }
        if std::env::args().nth(1).as_deref() != Some("root") {
            fs::create_dir_all("/dev")?;
            mount_fs("devtmpfs", "/dev", "devtmpfs")?;
            use std::os::fd::AsRawFd;
            for name in fs::read_to_string("/module-order")?.lines() {
                if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                {
                    return Err("invalid module name".into());
                }
                let module = fs::File::open(format!("/modules/{name}.ko"))?;
                // SAFETY: Linux ARM64 syscall 273 is finit_module(fd, params, flags).
                // The fd and NUL-terminated empty parameter string remain valid;
                // flags=0 asks the kernel to validate the uncompressed module.
                if unsafe {
                    syscall(
                        273 as std::ffi::c_long,
                        module.as_raw_fd(),
                        c"".as_ptr(),
                        0_i32,
                    )
                } != 0
                {
                    return Err(
                        format!("module {name}: {}", std::io::Error::last_os_error()).into(),
                    );
                }
            }
            fs::create_dir_all("/newroot")?;
            mount_fs("/dev/vda", "/newroot", "ext4")?;
            let path = c"/newroot";
            // SAFETY: the static NUL-terminated path lives across the call.
            if unsafe { chroot(path.as_ptr()) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            std::env::set_current_dir("/")?;
            return Err(Command::new("/guest-init").arg("root").exec().into());
        }
        fs::create_dir_all("/proc")?;
        mount_fs("proc", "/proc", "proc")?;
        let cmdline = fs::read_to_string("/proc/cmdline")?;
        let value = |key: &str| -> Result<String, Box<dyn std::error::Error>> {
            cmdline
                .split_whitespace()
                .find_map(|s| s.strip_prefix(key))
                .map(str::to_owned)
                .ok_or_else(|| "missing fixture argument".into())
        };
        let token = value("cocoon.token=")?;
        let payload = value("cocoon.payload=")?;
        for s in [&token, &payload] {
            if s.len() != 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("invalid fixture identity".into());
            }
        }
        let mode = value("cocoon.mode=")?;
        let mounts = fs::read_to_string("/proc/mounts")?;
        if !mounts.lines().any(|s| {
            let f: Vec<_> = s.split_whitespace().collect();
            f.len() >= 3 && f[0] == "/dev/vda" && f[1] == "/" && f[2] == "ext4"
        }) {
            return Err("root is not /dev/vda ext4".into());
        }
        if fs::read("/guest-init")? != fs::read("/proc/self/exe")? {
            return Err("executable is not rooted on disk".into());
        }
        println!("COCOON_READY:{token}");
        if mode == "project" {
            mount_fs("devtmpfs", "/dev", "devtmpfs")?;
            println!("COCOON_ROOT:ext4:/dev/vda:{token}");
            std::io::stdout().flush()?;
            return Err(Command::new("/guest-helper").exec().into());
        }
        if mode == "write" || mode == "snapshot" {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open("/persistent")?;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
            file.write_all(payload.as_bytes())?;
            file.sync_all()?;
            fs::File::open("/")?.sync_all()?;
        } else if mode != "read" {
            return Err("invalid fixture mode".into());
        }
        if fs::read("/persistent")? != payload.as_bytes()
            || fs::metadata("/persistent")?.permissions().mode() & 0o777 != 0o600
        {
            return Err("persistent bytes or permissions mismatch".into());
        }
        println!("COCOON_ROOT:ext4:/dev/vda:{token}");
        println!("COCOON_PERSIST:{mode}:{payload}:{token}");
        if mode == "snapshot" {
            let mut counter = u64::from_str_radix(&payload[..8], 16)?;
            let mut random = [0_u8; 16];
            // SAFETY: Linux ARM64 getrandom(278) receives a valid writable buffer;
            // flags=0 requests cryptographically secure guest randomness.
            if unsafe { syscall(278, random.as_mut_ptr(), random.len(), 0_i32) } != 16 {
                return Err("cannot initialize snapshot nonce".into());
            }
            let nonce: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
            println!("COCOON_SNAPSHOT_READY:{token}:{counter}:{nonce}");
            std::io::stdout().flush()?;
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                counter += 1;
                println!("COCOON_RESUMED:{token}:{counter}:{nonce}");
                println!("COCOON_DONE:{token}");
                std::io::stdout().flush()?;
            }
        }
        println!("COCOON_DONE:{token}");
        std::io::stdout().flush()?;
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
}
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
fn main() {
    if let Err(error) = guest::run() {
        eprintln!("COCOON_FIXTURE_ERROR:{error}");
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
}
