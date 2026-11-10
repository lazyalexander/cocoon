//! Narrow native filesystem boundary. No authorization or lifecycle policy.
use std::{
    fs,
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyMethod {
    Clone,
    Sparse,
}

pub fn copy_disk(source: &Path, destination: &Path, allow_clone: bool) -> io::Result<CopyMethod> {
    if !fs::symlink_metadata(source)?.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "source must be a regular non-symlink file",
        ));
    }
    #[cfg(target_os = "macos")]
    if allow_clone {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        unsafe extern "C" {
            fn clonefile(source: *const i8, destination: *const i8, flags: u32) -> i32;
        }
        let src = CString::new(source.as_os_str().as_bytes())?;
        let dst = CString::new(destination.as_os_str().as_bytes())?;
        // SAFETY: owned C strings are alive and NUL-terminated. clonefile does not
        // retain these pointers; destination must not exist. Flag 1 forbids symlink following.
        if unsafe { clonefile(src.as_ptr(), dst.as_ptr(), 1) } == 0 {
            return Ok(CopyMethod::Clone);
        }
        let error = io::Error::last_os_error();
        // EXDEV / ENOTSUP only. Permission, space, existing destination and I/O
        // failures must not silently select a different path.
        if !matches!(error.raw_os_error(), Some(18 | 45)) {
            return Err(error);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = allow_clone;
    let mut input = fs::File::open(source)?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let mut bytes = vec![0; 1024 * 1024];
    let mut size = 0;
    loop {
        let count = input.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        if bytes[..count].iter().all(|b| *b == 0) {
            output.seek(SeekFrom::Current(count as i64))?;
        } else {
            output.write_all(&bytes[..count])?;
        }
        size += count as u64;
    }
    output.set_len(size)?;
    output.sync_all()?;
    Ok(CopyMethod::Sparse)
}
