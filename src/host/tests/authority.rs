use cocoon_host::{
    ExecutionAuthority, FinishPolicy, HostOperation, HostRequest, HostResponse, Manager, RequestId,
    Vm, VmId, VmState, WorkAction,
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
        grant: String::new(),
        operation: HostOperation::ProtectionStatus,
    };
    let response = manager.handle_api(request, Path::new("/unused"));
    assert!(response.result.is_err());
    let credential = fs::read_to_string(sandbox.0.join("api-credential")).unwrap();
    let request = HostRequest {
        request: RequestId::from(Uuid::new_v4()),
        caller: "agent".into(),
        credential,
        grant: String::new(),
        operation: HostOperation::ProtectionStatus,
    };
    let response = manager.handle_api(request, Path::new("/unused"));
    assert!(response.result.is_ok());
    assert_eq!(response.caller, "agent");
    let show = HostRequest {
        request: RequestId::from(Uuid::new_v4()),
        caller: "agent".into(),
        credential: fs::read_to_string(sandbox.0.join("api-credential")).unwrap(),
        grant: String::new(),
        operation: HostOperation::Show { vm },
    };
    let rejected = manager.handle_api(show, Path::new("/unused"));
    assert!(
        rejected.result.is_err(),
        "management credential is not a work grant"
    );
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
    let mut stream = UnixStream::connect(socket).unwrap();
    let issued = Manager::open(&sandbox.0)
        .unwrap()
        .issue_work_grant(
            "socket-agent",
            vm,
            &[WorkAction::Inspect],
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 600,
            1024,
            "",
        )
        .unwrap();
    let request = HostRequest {
        request: RequestId::from(Uuid::new_v4()),
        caller: "socket-agent".into(),
        credential: String::new(),
        grant: issued["token"].as_str().unwrap().to_string(),
        operation: HostOperation::Show { vm },
    };
    cocoon_guest_wire::send(&request, &mut stream).unwrap();
    let response: HostResponse = cocoon_guest_wire::receive(&mut stream).unwrap();
    assert!(response.result.is_ok(), "{:?}", response.result);
    assert_eq!(response.caller, "socket-agent");
}

fn expires_in(seconds: u64) -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + seconds
}

fn work_request(caller: &str, grant: &str, operation: HostOperation) -> HostRequest {
    HostRequest {
        request: RequestId::from(Uuid::new_v4()),
        caller: caller.into(),
        credential: String::new(),
        grant: grant.into(),
        operation,
    }
}

#[test]
fn work_grants_reject_missing_wrong_expired_revoked_stale_and_wider_uses() {
    let sandbox = Sandbox::new();
    let manager = Manager::open(&sandbox.0).unwrap();
    let image = sandbox.fixture(&manager);
    let vm = VmId::from(Uuid::new_v4());
    let other = VmId::from(Uuid::new_v4());
    manager
        .create(vm, image, "owner", FinishPolicy::Retain)
        .unwrap();
    manager
        .create(other, image, "owner", FinishPolicy::Retain)
        .unwrap();
    let token = manager
        .issue_work_grant(
            "agent",
            vm,
            &[WorkAction::Inspect, WorkAction::Stop],
            expires_in(600),
            2048,
            "",
        )
        .unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    let missing = manager.handle_api(
        work_request("agent", "", HostOperation::Show { vm }),
        Path::new("/unused"),
    );
    assert!(missing.result.unwrap_err().contains("work authorization"));
    let wrong_subject = manager.handle_api(
        work_request("intruder", &token, HostOperation::Show { vm }),
        Path::new("/unused"),
    );
    assert!(
        wrong_subject
            .result
            .unwrap_err()
            .contains("subject mismatch")
    );
    let wrong_vm = manager.handle_api(
        work_request("agent", &token, HostOperation::Show { vm: other }),
        Path::new("/unused"),
    );
    assert!(wrong_vm.result.unwrap_err().contains("vm mismatch"));
    let inspect_only = manager
        .issue_work_grant(
            "agent",
            vm,
            &[WorkAction::Inspect],
            expires_in(600),
            2048,
            "",
        )
        .unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    let insufficient = manager.handle_api(
        work_request("agent", &inspect_only, HostOperation::Stop { vm }),
        Path::new("/unused"),
    );
    assert!(insufficient.result.unwrap_err().contains("does not allow"));
    assert!(
        manager
            .issue_work_grant(
                "agent",
                vm,
                &[WorkAction::Stop],
                expires_in(600),
                2048,
                &inspect_only,
            )
            .unwrap_err()
            .to_string()
            .contains("wider")
    );
    assert!(
        manager
            .issue_work_grant(
                "agent",
                other,
                &[WorkAction::Inspect],
                expires_in(600),
                2048,
                &token,
            )
            .unwrap_err()
            .to_string()
            .contains("wider")
    );
    let child_token = manager
        .issue_work_grant(
            "agent",
            vm,
            &[WorkAction::Inspect],
            expires_in(300),
            1024,
            &token,
        )
        .unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        manager
            .handle_api(
                work_request("agent", &child_token, HostOperation::Show { vm }),
                Path::new("/unused"),
            )
            .result
            .is_ok()
    );
    let parent = manager
        .issue_work_grant(
            "agent",
            vm,
            &[WorkAction::Inspect],
            expires_in(600),
            2048,
            "",
        )
        .unwrap();
    let parent_token = parent["token"].as_str().unwrap().to_string();
    let parent_grant = Uuid::parse_str(parent["grant"].as_str().unwrap()).unwrap();
    let nested = manager
        .issue_work_grant(
            "agent",
            vm,
            &[WorkAction::Inspect],
            expires_in(100),
            512,
            &parent_token,
        )
        .unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    manager.revoke_work_grant(parent_grant).unwrap();
    let revoked = manager.handle_api(
        work_request("agent", &nested, HostOperation::Show { vm }),
        Path::new("/unused"),
    );
    assert!(revoked.result.unwrap_err().contains("revoked"));
    let journal = serde_json::to_string(&manager.journal().unwrap()).unwrap();
    assert!(!journal.contains(&token));
    assert!(!journal.contains(&child_token));
    assert!(!journal.contains(&nested));
    let runner = sandbox.0.join("runner");
    fs::write(&runner, [0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0, 0, 1]).unwrap();
    let fresh = manager
        .issue_work_grant(
            "agent",
            other,
            &[WorkAction::Inspect],
            expires_in(600),
            1024,
            "",
        )
        .unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(manager.start_with_runner(other, &runner).is_err());
    let stale = manager.handle_api(
        work_request("agent", &fresh, HostOperation::Show { vm: other }),
        Path::new("/unused"),
    );
    assert!(stale.result.unwrap_err().contains("generation mismatch"));
    assert!(
        manager
            .issue_work_grant("agent", vm, &[WorkAction::Inspect], 1, 1024, "")
            .unwrap_err()
            .to_string()
            .contains("expired")
    );
    let short = manager
        .issue_work_grant("agent", vm, &[WorkAction::Inspect], expires_in(1), 1024, "")
        .unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    std::thread::sleep(std::time::Duration::from_secs(2));
    let expired = manager.handle_api(
        work_request("agent", &short, HostOperation::Show { vm }),
        Path::new("/unused"),
    );
    let message = expired.result.unwrap_err();
    assert!(message.contains("expired"), "{message}");
    assert!(!message.contains(&short));
}

#[test]
fn repeated_start_uses_the_saved_outcome_and_rejects_parameter_changes() {
    let sandbox = Sandbox::new();
    let manager = Manager::open(&sandbox.0).unwrap();
    let image = sandbox.fixture(&manager);
    let first = VmId::from(Uuid::new_v4());
    let second = VmId::from(Uuid::new_v4());
    manager
        .create(first, image, "owner", FinishPolicy::Retain)
        .unwrap();
    manager
        .create(second, image, "owner", FinishPolicy::Retain)
        .unwrap();
    let _ = manager.handle_api(
        HostRequest {
            request: RequestId::from(Uuid::new_v4()),
            caller: "operator".into(),
            credential: "create-credential".into(),
            grant: String::new(),
            operation: HostOperation::ProtectionStatus,
        },
        Path::new("/unused"),
    );
    let credential = fs::read_to_string(sandbox.0.join("api-credential")).unwrap();
    let runner = sandbox.0.join("runner");
    fs::write(&runner, [0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0, 0, 1]).unwrap();
    let id = RequestId::from(Uuid::new_v4());
    let mut request = HostRequest {
        request: id,
        caller: "operator".into(),
        credential: credential.clone(),
        grant: String::new(),
        operation: HostOperation::Start { vm: first },
    };
    assert!(manager.handle_api(request.clone(), &runner).result.is_err());
    let intents = manager
        .journal()
        .unwrap()
        .into_iter()
        .filter(|entry| entry.kind == "start_intent")
        .count();
    assert_eq!(intents, 1);
    request.credential = credential.clone();
    assert!(manager.handle_api(request.clone(), &runner).result.is_err());
    assert_eq!(
        manager
            .journal()
            .unwrap()
            .into_iter()
            .filter(|entry| entry.kind == "start_intent")
            .count(),
        1
    );
    request.operation = HostOperation::Start { vm: second };
    let conflict = manager.handle_api(request, &runner).result.unwrap_err();
    assert!(conflict.contains("parameters conflict"), "{conflict}");
    assert_eq!(manager.get(second).unwrap().state, VmState::Prepared);
    let status = manager.handle_api(
        HostRequest {
            request: RequestId::from(Uuid::new_v4()),
            caller: "operator".into(),
            credential,
            grant: manager
                .issue_work_grant(
                    "operator",
                    first,
                    &[WorkAction::Inspect],
                    expires_in(600),
                    1024,
                    "",
                )
                .unwrap()["token"]
                .as_str()
                .unwrap()
                .to_string(),
            operation: HostOperation::OperationStatus { id },
        },
        &runner,
    );
    let body = status.result.unwrap();
    assert_eq!(body["status"], "failed");
}
