use cocoon_host::{ExecutionAuthority, FinishPolicy, Manager, Vm, VmId, VmState};
use std::{fs, path::PathBuf};
use uuid::Uuid;

struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("cocoon-authority-{}", Uuid::new_v4())))
    }
    fn fixture(&self, manager: &Manager) -> cocoon_host::ImageId {
        let input = self.0.join("fixture");
        fs::write(&input, b"fixture").unwrap();
        manager
            .import_image(&input, &input, &input, "test MIT")
            .unwrap()
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn authority_keeps_vm_identity_and_revokes_old_attempts() {
    let mut vm: Vm = serde_json::from_value(serde_json::json!({
        "id": Uuid::new_v4(), "owner": "owner", "image": Uuid::new_v4(),
        "policy": "retain", "state": "stopped", "network": false,
        "vcpus": 1, "memory_mib": 512
    }))
    .unwrap();
    assert_eq!(vm.generation, 0);
    let first = ExecutionAuthority::issue(&mut vm).unwrap();
    assert_eq!(first.attempt.vm, vm.id);
    assert_eq!(first.generation, 1);
    assert!(first.authorize(&vm).is_ok());
    vm.state = VmState::Stopped;
    let second = ExecutionAuthority::issue(&mut vm).unwrap();
    assert_eq!(second.generation, 2);
    assert!(first.authorize(&vm).is_err());
    assert!(second.authorize(&vm).is_ok());
    assert_eq!(vm.state, VmState::Starting);
}

#[test]
fn wal_is_ordered_persistent_and_tamper_evident() {
    let sandbox = Sandbox::new();
    let manager = Manager::open(&sandbox.0).unwrap();
    let image = sandbox.fixture(&manager);
    let vm = VmId::from(Uuid::new_v4());
    manager
        .create(vm, image, "owner", FinishPolicy::Retain)
        .unwrap();
    let first = manager
        .append_journal(vm, "start_intent", serde_json::json!({"generation": 1}))
        .unwrap();
    let second = manager
        .append_journal(vm, "start_complete", serde_json::json!({"generation": 1}))
        .unwrap();
    assert_eq!((first.sequence, second.sequence), (1, 2));
    assert_eq!(second.previous_sha256, first.sha256);
    assert!(manager.verify_journal().unwrap());
    drop(manager);
    let manager = Manager::open(&sandbox.0).unwrap();
    assert_eq!(manager.journal().unwrap().len(), 2);
    assert!(manager.verify_journal().unwrap());
    let db = rusqlite::Connection::open(sandbox.0.join("state.sqlite3")).unwrap();
    db.execute(
        "UPDATE journal SET payload='{\"generation\":2}' WHERE sequence=1",
        [],
    )
    .unwrap();
    assert!(!manager.verify_journal().unwrap());
}
