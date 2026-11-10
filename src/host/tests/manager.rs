use cocoon_host::{FinishPolicy, ImageId, Manager, VmId, VmState};
use std::{fs, path::PathBuf};
use uuid::Uuid;
struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("cocoon-test-{}", Uuid::new_v4())))
    }
    fn fixture(&self, manager: &Manager) -> ImageId {
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
    let id = VmId::from(Uuid::new_v4());
    manager
        .create(id, image, "owner", FinishPolicy::Retain)
        .unwrap();
    drop(manager);
    let manager = Manager::open(&sandbox.0).unwrap();
    let vm = manager
        .create(id, image, "owner", FinishPolicy::Retain)
        .unwrap();
    assert_eq!(vm.state, VmState::Prepared);
    assert!(!vm.network);
    assert_eq!(manager.list().unwrap().len(), 1);
    assert!(
        manager
            .create(id, image, "different-owner", FinishPolicy::Retain)
            .is_err()
    );
    assert!(manager.start(id).is_err());
    assert_eq!(manager.get(id).unwrap().state, VmState::Prepared);
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
            .create(Uuid::new_v4().into(), image, "owner", FinishPolicy::Retain)
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
    let id = VmId::from(Uuid::new_v4());
    manager
        .create(id, image, "owner", FinishPolicy::Retain)
        .unwrap();
    fs::write(
        sandbox.0.join("vms").join(id.to_string()).join("root.raw"),
        b"changed",
    )
    .unwrap();
    manager
        .create(Uuid::new_v4().into(), image, "owner", FinishPolicy::Retain)
        .unwrap();
}

#[test]
fn uncertain_creation_is_not_automatically_replayed() {
    let sandbox = Sandbox::new();
    let manager = Manager::open(&sandbox.0).unwrap();
    let image = sandbox.fixture(&manager);
    let id = VmId::from(Uuid::new_v4());
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

#[test]
fn unsupported_database_version_is_preserved() {
    let sandbox = Sandbox::new();
    drop(Manager::open(&sandbox.0).unwrap());
    let db = rusqlite::Connection::open(sandbox.0.join("state.sqlite3")).unwrap();
    db.execute_batch("PRAGMA user_version=99").unwrap();
    assert!(Manager::open(&sandbox.0).is_err());
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        99
    );
}

#[test]
fn experimental_boot_refuses_invalid_parameters_without_starting() {
    let sandbox = Sandbox::new();
    let manager = Manager::open(&sandbox.0).unwrap();
    let image = sandbox.fixture(&manager);
    let id = VmId::from(Uuid::new_v4());
    manager
        .create(id, image, "owner", FinishPolicy::Retain)
        .unwrap();
    assert!(
        manager
            .persistence_boot(id, std::path::Path::new("/unused"), "write", "bad")
            .is_err()
    );
    assert_eq!(manager.get(id).unwrap().state, VmState::Prepared);
}

#[test]
fn legacy_schema_migrates_without_losing_registration() {
    let sandbox = Sandbox::new();
    let manager = Manager::open(&sandbox.0).unwrap();
    let image = sandbox.fixture(&manager);
    let id = VmId::from(Uuid::new_v4());
    manager
        .create(id, image, "legacy-owner", FinishPolicy::Retain)
        .unwrap();
    drop(manager);
    let db = rusqlite::Connection::open(sandbox.0.join("state.sqlite3")).unwrap();
    db.execute_batch("PRAGMA user_version=0; DROP TABLE boots; UPDATE vms SET record=json_remove(record,'$.disk_copy');").unwrap();
    drop(db);
    let manager = Manager::open(&sandbox.0).unwrap();
    let vm = manager.get(id).unwrap();
    assert_eq!(vm.owner, "legacy-owner");
    assert!(vm.disk_copy.is_none());
    assert_eq!(vm.state, VmState::Prepared);
}

#[test]
fn unknown_boot_blocks_execution_even_with_matching_identity() {
    let sandbox = Sandbox::new();
    let manager = Manager::open(&sandbox.0).unwrap();
    let image = sandbox.fixture(&manager);
    let id = VmId::from(Uuid::new_v4());
    manager
        .create(id, image, "owner", FinishPolicy::Retain)
        .unwrap();
    let db = rusqlite::Connection::open(sandbox.0.join("state.sqlite3")).unwrap();
    db.execute(
        "UPDATE vms SET record=json_set(record,'$.state','unknown') WHERE id=?1",
        [id.to_string()],
    )
    .unwrap();
    let runner = sandbox.0.join("fake-runner");
    fs::write(&runner, [0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0, 0, 1]).unwrap();
    let error = manager
        .persistence_boot(id, &runner, "read", &"a".repeat(32))
        .unwrap_err();
    assert!(error.to_string().contains("reconciliation"));
    assert_eq!(manager.get(id).unwrap().state, VmState::Unknown);
}

#[test]
fn publication_failure_preserves_pending_intent_and_no_image_record() {
    let sandbox = Sandbox::new();
    let manager = Manager::open(&sandbox.0).unwrap();
    let db = rusqlite::Connection::open(sandbox.0.join("state.sqlite3")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_publication BEFORE INSERT ON images BEGIN SELECT RAISE(ABORT,'injected publication failure'); END;").unwrap();
    let input = sandbox.0.join("input");
    fs::write(&input, b"test").unwrap();
    assert!(
        manager
            .import_image(&input, &input, &input, "test MIT")
            .is_err()
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM images", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        0
    );
    assert_eq!(manager.operations().unwrap()[0]["status"], "pending");
    assert_eq!(fs::read_dir(sandbox.0.join("images")).unwrap().count(), 1);
}

#[test]
fn failed_spawn_keeps_unknown_boot_and_prevents_replay() {
    let sandbox = Sandbox::new();
    let manager = Manager::open(&sandbox.0).unwrap();
    let input = sandbox.0.join("input");
    let mut bytes = [0_u8; 64];
    bytes[56..60].copy_from_slice(b"ARM\x64");
    fs::write(&input, bytes).unwrap();
    let image = manager
        .import_image(&input, &input, &input, "test MIT")
        .unwrap();
    let id = VmId::from(Uuid::new_v4());
    manager
        .create(id, image, "owner", FinishPolicy::Retain)
        .unwrap();
    let runner = sandbox.0.join("unexecutable-runner");
    fs::write(&runner, [0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0, 0, 1]).unwrap();
    assert!(
        manager
            .persistence_boot(id, &runner, "read", &"a".repeat(32))
            .is_err()
    );
    assert_eq!(manager.get(id).unwrap().state, VmState::Unknown);
    assert!(
        manager
            .persistence_boot(id, &runner, "read", &"a".repeat(32))
            .unwrap_err()
            .to_string()
            .contains("reconciliation")
    );
    let db = rusqlite::Connection::open(sandbox.0.join("state.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM boots", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        1
    );
}
