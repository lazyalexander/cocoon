use cocoon_runtime::manager::{FinishPolicy, Manager};
use std::{fs, path::PathBuf};
use uuid::Uuid;
struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("cocoon-test-{}", Uuid::new_v4())))
    }
    fn fixture(&self, manager: &Manager) -> Uuid {
        let input = self.0.join("fixture");
        fs::write(&input, b"test bytes; not a bootable image").unwrap();
        manager
            .import_image(&input, &input, &input, "Original test fixture; MIT")
            .unwrap()
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn persistent_registration_is_idempotent_but_never_execution() {
    let sandbox = Sandbox::new();
    let manager = Manager::open(&sandbox.0).unwrap();
    let image = sandbox.fixture(&manager);
    let id = Uuid::new_v4();
    manager
        .create(id, image, "owner", FinishPolicy::Retain)
        .unwrap();
    drop(manager);
    let manager = Manager::open(&sandbox.0).unwrap();
    let vm = manager
        .create(id, image, "owner", FinishPolicy::Retain)
        .unwrap();
    assert_eq!(vm.state, "prepared");
    assert!(!vm.network);
    assert_eq!(manager.list().unwrap().len(), 1);
    assert!(
        manager
            .create(id, image, "different-owner", FinishPolicy::Retain)
            .is_err()
    );
    assert!(manager.start(id).is_err());
    assert_eq!(manager.get(id).unwrap().state, "prepared");
}
#[test]
fn image_tampering_blocks_creation() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new();
    let manager = Manager::open(&sandbox.0).unwrap();
    let image = sandbox.fixture(&manager);
    let disk = sandbox
        .0
        .join("images")
        .join(image.to_string())
        .join("root.raw");
    fs::set_permissions(&disk, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(disk, b"modified").unwrap();
    assert!(
        manager
            .create(Uuid::new_v4(), image, "owner", FinishPolicy::Retain)
            .is_err()
    );
}
#[test]
fn failed_copy_leaves_a_pending_operation() {
    let sandbox = Sandbox::new();
    let manager = Manager::open(&sandbox.0).unwrap();
    let missing = sandbox.0.join("missing");
    assert!(
        manager
            .import_image(&missing, &missing, &missing, "test")
            .is_err()
    );
    let operations = manager.operations().unwrap();
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0]["status"], "pending");
}
#[test]
fn work_disk_does_not_modify_baseline() {
    let sandbox = Sandbox::new();
    let manager = Manager::open(&sandbox.0).unwrap();
    let image = sandbox.fixture(&manager);
    let id = Uuid::new_v4();
    manager
        .create(id, image, "owner", FinishPolicy::Retain)
        .unwrap();
    fs::write(
        sandbox.0.join("vms").join(id.to_string()).join("root.raw"),
        b"changed",
    )
    .unwrap();
    manager
        .create(Uuid::new_v4(), image, "owner", FinishPolicy::Retain)
        .unwrap();
}

#[test]
fn uncertain_creation_is_not_automatically_replayed() {
    let sandbox = Sandbox::new();
    let manager = Manager::open(&sandbox.0).unwrap();
    let image = sandbox.fixture(&manager);
    let id = Uuid::new_v4();
    // Simulate a filesystem side effect without a committed VM registration.
    let dir = sandbox.0.join("vms").join(id.to_string());
    fs::create_dir(&dir).unwrap();
    assert!(
        manager
            .create(id, image, "owner", FinishPolicy::Retain)
            .is_err()
    );
    fs::remove_dir(&dir).unwrap();
    drop(manager);
    let manager = Manager::open(&sandbox.0).unwrap();
    assert!(
        manager
            .create(id, image, "owner", FinishPolicy::Retain)
            .is_err()
    );
    assert!(!dir.exists());
    assert!(manager.get(id).is_err());
}
