use super::*;

fn task() -> Task {
    serde_json::from_str(include_str!("../../../examples/task.json")).unwrap()
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
