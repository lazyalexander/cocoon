use cocoon_platform::{CopyMethod, copy_disk};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::{MetadataExt, symlink},
};
fn sparse_roundtrip(logical_bytes: u64) {
    let dir = std::env::temp_dir().join(format!("cocoon-sparse-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    let source = dir.join("source");
    let destination = dir.join("copy");
    let mut file = fs::File::create(&source).unwrap();
    file.set_len(logical_bytes).unwrap();
    file.seek(SeekFrom::Start(32 * 1024 * 1024)).unwrap();
    file.write_all(b"retained").unwrap();
    file.sync_all().unwrap();
    assert_eq!(
        copy_disk(&source, &destination, false).unwrap(),
        CopyMethod::Sparse
    );
    let meta = fs::metadata(&destination).unwrap();
    assert_eq!(meta.len(), logical_bytes);
    assert!(meta.blocks() * 512 < 4 * 1024 * 1024);
    let mut copied = fs::File::open(&destination).unwrap();
    copied.seek(SeekFrom::Start(32 * 1024 * 1024)).unwrap();
    let mut bytes = [0; 8];
    copied.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"retained");
    assert!(copy_disk(&source, &destination, false).is_err());
    let link = dir.join("link");
    symlink(&source, &link).unwrap();
    assert!(copy_disk(&link, &dir.join("unsafe"), false).is_err());
    fs::remove_dir_all(dir).unwrap();
}
#[cfg(target_os = "macos")]
#[test]
fn native_clone_is_independent() {
    let dir = std::env::temp_dir().join(format!("cocoon-clone-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    let source = dir.join("source");
    let destination = dir.join("copy");
    fs::write(&source, b"baseline").unwrap();
    let method = copy_disk(&source, &destination, true).unwrap();
    assert_eq!(method, CopyMethod::Clone);
    fs::write(&destination, b"modified").unwrap();
    assert_eq!(fs::read(&source).unwrap(), b"baseline");
    assert!(copy_disk(&source, &destination, true).is_err());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn sparse_copy_preserves_holes_bytes_and_refuses_overwrite() {
    sparse_roundtrip(64 * 1024 * 1024);
}
#[test]
#[ignore = "explicit release-mode 8 GiB sparse-copy acceptance"]
fn eight_gib_sparse_fallback() {
    sparse_roundtrip(8 * 1024 * 1024 * 1024);
}
