use cocoon_host::{FinishPolicy, HostOperation, HostRequest, Manager, RequestId, VmId, WorkAction};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

#[test]
fn primary_agent_context_is_scoped_and_omits_host_secrets() {
    let root = std::env::temp_dir().join(format!("cocoon-context-{}", Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let manager = Manager::open(&root).unwrap();
    let fixture = root.join("fixture");
    fs::write(&fixture, b"fixture").unwrap();
    let image = manager
        .import_image(&fixture, &fixture, &fixture, "test MIT")
        .unwrap();
    let first = VmId::from(Uuid::new_v4());
    let second = VmId::from(Uuid::new_v4());
    for vm in [first, second] {
        manager
            .create(vm, image, "owner", FinishPolicy::Retain)
            .unwrap();
    }
    let expiry = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    let issued = manager
        .issue_work_grant("primary", first, &[WorkAction::Inspect], expiry, 1024, "")
        .unwrap();
    let token = issued["token"].as_str().unwrap();
    let call = |vm, caller: &str| {
        manager.handle_api(
            HostRequest {
                request: RequestId::from(Uuid::new_v4()),
                caller: caller.into(),
                credential: String::new(),
                grant: token.into(),
                operation: HostOperation::Context { vm },
            },
            Path::new("/unused"),
        )
    };
    let response = call(first, "primary");
    let context = response.result.unwrap();
    assert_eq!(context["schema"], "cocoon-agent-context-v1");
    assert_eq!(context["vm"], first.to_string());
    assert_eq!(context["state"], "prepared");
    assert_eq!(context["work"]["actions"], serde_json::json!(["inspect"]));
    assert_eq!(context["work"]["guest_ready"], false);
    assert_eq!(context["agent_model"]["host_caller"], "primary");
    let encoded = context.to_string();
    assert!(!encoded.contains(token));
    assert!(!encoded.contains(&root.to_string_lossy().into_owned()));
    assert!(call(second, "primary").result.is_err());
    assert!(call(first, "other").result.is_err());
    drop(manager);
    fs::remove_dir_all(root).unwrap();
}
