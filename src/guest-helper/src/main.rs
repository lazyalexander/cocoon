//! Experimental Linux-only helper; deliberately has no host lifecycle verbs.
#[cfg(target_os = "linux")]
mod execution;
#[cfg(target_os = "linux")]
mod vsock;

#[cfg(target_os = "linux")]
fn main() {
    if let Err(error) = vsock::serve() {
        eprintln!("COCOON_HELPER_ERROR:{error}");
        std::process::exit(2);
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("Linux guest helper only; native execution unavailable");
    std::process::exit(2);
}
