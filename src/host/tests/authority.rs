use cocoon_host::{
    ExecutionAuthority, FinishPolicy, HostOperation, HostRequest, HostResponse, Manager, RequestId,
    Vm, VmId, VmState,
};
use std::{
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        Self(PathBuf::from(format!("/tmp/cocoon-a-{}", Uuid::new_v4())))
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

#[test]
fn production_spawn_failure_requires_explicit_recovery_without_replay() {
    let sandbox = Sandbox::new();
    let manager = Manager::open(&sandbox.0).unwrap();
    let input = sandbox.0.join("linux");
    let mut bytes = [0_u8; 64];
    bytes[56..60].copy_from_slice(b"ARM\x64");
    fs::write(&input, bytes).unwrap();
    let image = manager
        .import_image(&input, &input, &input, "test MIT")
        .unwrap();
    let vm = VmId::from(Uuid::new_v4());
    manager
        .create(vm, image, "owner", FinishPolicy::Retain)
        .unwrap();
    let runner = sandbox.0.join("not-executable");
    fs::write(&runner, [0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0, 0, 1]).unwrap();
    assert!(manager.start_with_runner(vm, &runner).is_err());
    assert_eq!(manager.get(vm).unwrap().state, VmState::RecoveryRequired);
    assert!(manager.start_with_runner(vm, &runner).is_err());
    assert!(
        manager
            .journal()
            .unwrap()
            .iter()
            .any(|entry| entry.kind == "start_intent")
    );
}

#[test]
fn api_requires_private_credential_and_preserves_attribution() {
    let sandbox = Sandbox::new();
    let manager = Manager::open(&sandbox.0).unwrap();
    let image = sandbox.fixture(&manager);
    let vm = VmId::from(Uuid::new_v4());
    manager
        .create(vm, image, "owner", FinishPolicy::Retain)
        .unwrap();
    let request = HostRequest {
        request: RequestId::from(Uuid::new_v4()),
        caller: "agent".into(),
        credential: "bad".into(),
        operation: HostOperation::Show { vm },
    };
    let response = manager.handle_api(request, Path::new("/unused"));
    assert!(response.result.is_err());
    let credential = fs::read_to_string(sandbox.0.join("api-credential")).unwrap();
    let request = HostRequest {
        request: RequestId::from(Uuid::new_v4()),
        caller: "agent".into(),
        credential,
        operation: HostOperation::Show { vm },
    };
    let response = manager.handle_api(request, Path::new("/unused"));
    assert!(response.result.is_ok());
    assert_eq!(response.caller, "agent");
}

#[test]
fn private_unix_socket_roundtrip_enforces_authentication() {
    use std::{os::unix::net::UnixStream, thread, time::Duration};
    let sandbox = Sandbox::new();
    let manager = Manager::open(&sandbox.0).unwrap();
    let image = sandbox.fixture(&manager);
    let vm = VmId::from(Uuid::new_v4());
    manager
        .create(vm, image, "owner", FinishPolicy::Retain)
        .unwrap();
    let socket = manager.api_socket_path();
    drop(manager);
    let root = sandbox.0.clone();
    thread::spawn(move || {
        Manager::open(&root)
            .unwrap()
            .serve_api(Path::new("/unused"))
            .unwrap();
    });
    for _ in 0..100 {
        if socket.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let credential = fs::read_to_string(sandbox.0.join("api-credential")).unwrap();
    let mut stream = UnixStream::connect(socket).unwrap();
    let request = HostRequest {
        request: RequestId::from(Uuid::new_v4()),
        caller: "socket-agent".into(),
        credential,
        operation: HostOperation::Show { vm },
    };
    cocoon_guest_wire::send(&request, &mut stream).unwrap();
    let response: HostResponse = cocoon_guest_wire::receive(&mut stream).unwrap();
    assert!(response.result.is_ok(), "{:?}", response.result);
    assert_eq!(response.caller, "socket-agent");
}
