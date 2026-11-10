use cocoon_host::{FinishPolicy, Manager, VmId, VmState};
use std::{fs, path::PathBuf};
use uuid::Uuid;

#[test]
fn candidates_require_approval_and_branches_are_independent() {
    let root = PathBuf::from(format!("/tmp/cocoon-v-{}", Uuid::new_v4()));
    let manager = Manager::open(&root).unwrap();
    let input = root.join("input");
    fs::write(&input, b"baseline").unwrap();
    let base = manager
        .import_image(&input, &input, &input, "test MIT")
        .unwrap();
    let vm = VmId::from(Uuid::new_v4());
    manager
        .create(vm, base, "owner", FinishPolicy::Retain)
        .unwrap();
    let work = root.join("vms").join(vm.to_string()).join("root.raw");
    fs::write(&work, b"better-version").unwrap();
    let db = rusqlite::Connection::open(root.join("state.sqlite3")).unwrap();
    db.execute(
        "UPDATE vms SET record=json_set(record,'$.state','stopped') WHERE id=?1",
        [vm.to_string()],
    )
    .unwrap();
    let candidate = manager.save_candidate(vm).unwrap();
    assert_eq!(manager.image(candidate).unwrap()["approved"], false);
    let branch = VmId::from(Uuid::new_v4());
    assert!(manager.branch(candidate, branch, "owner").is_err());
    assert!(
        manager
            .create(
                VmId::from(Uuid::new_v4()),
                candidate,
                "owner",
                FinishPolicy::Retain
            )
            .is_err()
    );
    manager.promote_candidate(candidate).unwrap();
    let branched = manager.branch(candidate, branch, "owner").unwrap();
    assert_eq!(branched.state, VmState::Prepared);
    let branch_disk = root.join("vms").join(branch.to_string()).join("root.raw");
    assert_eq!(fs::read(&branch_disk).unwrap(), b"better-version");
    fs::write(&branch_disk, b"branch-change").unwrap();
    assert_eq!(
        fs::read(
            root.join("images")
                .join(candidate.to_string())
                .join("root.raw")
        )
        .unwrap(),
        b"better-version"
    );
    manager.rollback(vm, base).unwrap();
    assert_eq!(fs::read(work).unwrap(), b"baseline");
    assert!(manager.verify_journal().unwrap());
    drop(manager);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn interrupted_rollback_restores_previous_disk_and_requires_recovery() {
    let root = PathBuf::from(format!("/tmp/cocoon-r-{}", Uuid::new_v4()));
    let manager = Manager::open(&root).unwrap();
    let input = root.join("input");
    fs::write(&input, b"baseline").unwrap();
    let base = manager
        .import_image(&input, &input, &input, "test MIT")
        .unwrap();
    let vm = VmId::from(Uuid::new_v4());
    manager
        .create(vm, base, "owner", FinishPolicy::Retain)
        .unwrap();
    let dir = root.join("vms").join(vm.to_string());
    fs::write(dir.join("root.raw"), b"important-work").unwrap();
    let other_input = root.join("other-input");
    fs::write(&other_input, b"other-baseline").unwrap();
    let other = manager
        .import_image(&other_input, &other_input, &other_input, "test MIT")
        .unwrap();
    let db = rusqlite::Connection::open(root.join("state.sqlite3")).unwrap();
    db.execute(
        "INSERT INTO operations VALUES ('rollback-test','rollback',?1,'pending')",
        [vm.to_string()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO rollback_intents VALUES (?1,?2,?3)",
        rusqlite::params![
            vm.to_string(),
            base.to_string(),
            serde_json::to_string(&manager.get(vm).unwrap().disk_copy).unwrap()
        ],
    )
    .unwrap();
    fs::rename(dir.join("root.raw"), dir.join("root.rollback-old")).unwrap();
    fs::copy(other_input, dir.join("root.raw")).unwrap();
    db.execute("UPDATE vms SET record=json_set(record,'$.image',?1,'$.state','suspended','$.suspend_state','/old-state') WHERE id=?2",
        rusqlite::params![other.to_string(), vm.to_string()]).unwrap();
    assert!(manager.rollback(vm, other).is_err());
    assert_eq!(fs::read(dir.join("root.raw")).unwrap(), b"important-work");
    assert_eq!(manager.get(vm).unwrap().state, VmState::RecoveryRequired);
    assert_eq!(manager.get(vm).unwrap().image, base);
    assert!(manager.get(vm).unwrap().suspend_state.is_none());
    drop(manager);
    fs::remove_dir_all(root).unwrap();
}
