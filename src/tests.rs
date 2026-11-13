use super::*;

fn task() -> Task {
    serde_json::from_str(include_str!("../examples/task.json")).unwrap()
}

#[test]
fn requests_cannot_expand_initial_authority() {
    let mut t = task();
    t.validate().unwrap();
    t.network = true;
    assert!(t.validate().is_err());
    t.network = false;
    t.image = "alpine:latest".into();
    assert!(t.validate().is_err());
    t = task();
    t.limits.wall_seconds = 0;
    assert!(t.validate().is_err());
    let mut wire = serde_json::to_value(task()).unwrap();
    wire["host_mounts"] = serde_json::json!(["/"]);
    assert!(serde_json::from_value::<Task>(wire).is_err());
}

#[test]
fn cancellation_and_owner_loss_are_not_termination() {
    let mut a = Attempt::prepare(Uuid::new_v4());
    a.starting().unwrap();
    a.running().unwrap();
    a.request_stop().unwrap();
    assert!(!a.may_settle());
    a.owner_lost().unwrap();
    assert!(!a.may_settle());
    assert!(a.starting().is_err()); // No automatic replay.
    assert!(a.confirm_stopped(Uuid::new_v4()).is_err());
    assert!(!a.may_settle());
    a.confirm_stopped(a.id()).unwrap();
    assert!(a.may_settle());
    assert!(a.running().is_err());
}

#[test]
fn retries_have_separate_attempt_identity() {
    let task_id = Uuid::new_v4();
    let first = Attempt::prepare(task_id);
    let second = Attempt::prepare(task_id);
    assert_ne!(first.id(), second.id());
    assert!(!first.may_settle());
    assert!(!second.may_settle());
}

#[test]
fn spool_profile_rejects_platform_network_and_unpinned_inputs() {
    let mut wire = serde_json::json!({
        "schema": "cocoon-spool-task-v1",
        "caller": {"subject": "host", "kind": "service"},
        "release_sha256": "a".repeat(64),
        "kernel_sha256": "b".repeat(64),
        "initramfs_sha256": "c".repeat(64),
        "platform": "linux-arm64", "argv": ["/spool/run"],
        "limits": {"vcpus": 1, "memory_mib": 256, "writable_mib": 1,
                   "wall_seconds": 10, "output_bytes": 4096},
        "network": false
    });
    let valid = wire.clone();
    let check = |v| {
        serde_json::from_value::<spool::SpoolTask>(v)
            .unwrap()
            .validate()
    };
    assert!(check(wire.clone()).is_ok());
    for (key, value) in [
        ("platform", serde_json::json!("macos-arm64")),
        ("network", serde_json::json!(true)),
        ("kernel_sha256", serde_json::json!("latest")),
        ("argv", serde_json::json!(["/spool/../host"])),
        ("argv", serde_json::json!([])),
    ] {
        wire = valid.clone();
        wire[key] = value;
        assert!(check(wire.clone()).is_err());
    }
    wire = valid;
    wire["host_mounts"] = serde_json::json!(["/"]);
    assert!(serde_json::from_value::<spool::SpoolTask>(wire).is_err());
}
