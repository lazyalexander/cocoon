use crate::{DiskCopy, DiskCopyMethod, Result};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
};
pub(crate) fn copy_new(source: &Path, target: &Path) -> Result<DiskCopy> {
    let method = match cocoon_platform::copy_disk(source, target, true)? {
        cocoon_platform::CopyMethod::Clone => DiskCopyMethod::Clone,
        cocoon_platform::CopyMethod::Sparse => DiskCopyMethod::Sparse,
    };
    fs::set_permissions(target, fs::Permissions::from_mode(0o600))?;
    fs::File::open(target)?.sync_all()?;
    let metadata = fs::metadata(target)?;
    Ok(DiskCopy {
        method,
        logical_bytes: metadata.len(),
        allocated_bytes: metadata.blocks() * 512,
    })
}

pub(crate) fn hash(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub(crate) fn sync_directory(path: &Path) -> Result<()> {
    fs::File::open(path)?.sync_all()?;
    Ok(())
}
