//! Disk publication tests use nonbootable fixtures and make no VM execution claim.
use cocoon_host::{ImageAction, ImageId, ImageQuery, ImageRequest, Manager, RequestId, VmId};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};
use uuid::Uuid;

struct Fixture {
    root: PathBuf,
    manager: Manager,
    credential: String,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("cocoon-images-{}", Uuid::new_v4()));
        let manager = Manager::open(&root).unwrap();
        assert_eq!(
            manager.initialize_image_api().unwrap()["memory_restore_qualified"],
            false
        );
        let credential = fs::read_to_string(root.join("api-credential")).unwrap();
        Self {
            root,
            manager,
            credential,
        }
    }
    fn request(&self, action: ImageAction) -> ImageRequest {
        ImageRequest {
            request: Uuid::new_v4().into(),
            caller: "test-host".into(),
            credential: self.credential.clone(),
            action,
        }
    }
    fn perform(&self, action: ImageAction) -> Value {
        let result = self
            .manager
            .perform_image_operation(&self.request(action))
            .unwrap();
        assert_eq!(result["state"], "succeeded", "{result}");
        result["result"].clone()
    }
    fn import(&self) -> ImageId {
        let input = self.root.join(format!("input-{}", Uuid::new_v4()));
        fs::write(&input, b"original").unwrap();
        let sha = hash(b"original");
        let result = self.perform(ImageAction::ImportCandidate {
            environment: "a".repeat(64),
            kernel: input.clone(),
            initrd: input.clone(),
            disk: input,
            kernel_sha256: sha.clone(),
            initrd_sha256: sha.clone(),
            disk_sha256: sha,
        });
        Uuid::parse_str(result["image"].as_str().unwrap())
            .unwrap()
            .into()
    }
    fn approve(&self, image: ImageId) {
        self.perform(ImageAction::PromoteCandidate {
            image,
            sha256: self.manager.image_content_digest(image).unwrap(),
        });
    }
    fn branch(&self, image: ImageId) -> VmId {
        let vm = Uuid::new_v4().into();
        self.perform(ImageAction::Branch {
            image,
            sha256: self.manager.image_content_digest(image).unwrap(),
            vm,
            owner: "new-task".into(),
        });
        vm
    }
    fn query(&self, request: RequestId) -> ImageQuery {
        ImageQuery {
            request,
            caller: "test-host".into(),
            credential: self.credential.clone(),
        }
    }
    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.root.join("state.sqlite3")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn environment_project_branches_and_rollback_survive_reopen() {
    let f = Fixture::new();
    let environment = f.import();
    let denied = f.request(ImageAction::Branch {
        image: environment,
        sha256: f.manager.image_content_digest(environment).unwrap(),
        vm: Uuid::new_v4().into(),
        owner: "task".into(),
    });
    assert_eq!(
        f.manager.perform_image_operation(&denied).unwrap()["state"],
        "failed"
    );
    f.approve(environment);
    let a = f.branch(environment);
    let b = f.branch(environment);
    let disk = f.root.join("vms").join(a.to_string()).join("root.raw");
    fs::write(&disk, b"private project").unwrap();
    let source = f.manager.working_disk_summary(a).unwrap();
    let saved = f.perform(ImageAction::SaveCandidate {
        vm: a,
        generation: 0,
        disk_sha256: source["disk_sha256"].as_str().unwrap().into(),
    });
    let project: ImageId = Uuid::parse_str(saved["image"].as_str().unwrap())
        .unwrap()
        .into();
    assert_eq!(
        f.manager.image_summary(project).unwrap()["source_environment"],
        Value::Null
    );
    f.approve(project);
    let c = f.branch(project);
    assert_eq!(
        fs::read(f.root.join("vms").join(c.to_string()).join("root.raw")).unwrap(),
        b"private project"
    );
    assert_eq!(
        fs::read(f.root.join("vms").join(b.to_string()).join("root.raw")).unwrap(),
        b"original"
    );
    let request = f.request(ImageAction::Rollback {
        vm: a,
        generation: 0,
        disk_sha256: source["disk_sha256"].as_str().unwrap().into(),
        image: environment,
        sha256: f.manager.image_content_digest(environment).unwrap(),
    });
    let first = f.manager.perform_image_operation(&request).unwrap();
    assert_eq!(first["state"], "succeeded");
    assert_eq!(fs::read(disk).unwrap(), b"original");
    let reopened = Manager::open(&f.root).unwrap();
    assert_eq!(
        reopened
            .image_operation_status(&f.query(request.request))
            .unwrap(),
        first
    );
    assert_eq!(reopened.perform_image_operation(&request).unwrap(), first);
    assert_eq!(
        reopened
            .cancel_image_operation(&f.query(request.request))
            .unwrap(),
        first
    );
    let changed = ImageRequest {
        action: ImageAction::PromoteCandidate {
            image: environment,
            sha256: "0".repeat(64),
        },
        ..request
    };
    assert!(reopened.perform_image_operation(&changed).is_err());
    assert_eq!(reopened.list().unwrap().len(), 3);
}

#[test]
fn authentication_integrity_generation_and_active_source_fail_closed() {
    let f = Fixture::new();
    let image = f.import();
    f.approve(image);
    let vm = f.branch(image);
    let mut query = f.query(Uuid::new_v4().into());
    query.credential = "wrong".into();
    assert!(f.manager.image_operation_status(&query).is_err());
    for generation in [1, 100] {
        let request = f.request(ImageAction::SaveCandidate {
            vm,
            generation,
            disk_sha256: "a".repeat(64),
        });
        assert_eq!(
            f.manager.perform_image_operation(&request).unwrap()["state"],
            "failed"
        );
    }
    let mut record = serde_json::to_value(f.manager.get(vm).unwrap()).unwrap();
    record["state"] = json!("running");
    f.db()
        .execute(
            "UPDATE vms SET record=?1 WHERE id=?2",
            [record.to_string(), vm.to_string()],
        )
        .unwrap();
    assert!(f.manager.working_disk_summary(vm).is_err());
    let request = f.request(ImageAction::SaveCandidate {
        vm,
        generation: 0,
        disk_sha256: hash(b"original"),
    });
    assert_eq!(
        f.manager.perform_image_operation(&request).unwrap()["state"],
        "failed"
    );
    let disk = f
        .root
        .join("images")
        .join(image.to_string())
        .join("root.raw");
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&disk, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(disk, b"corrupt").unwrap();
    let request = f.request(ImageAction::Branch {
        image,
        sha256: f.manager.image_content_digest(image).unwrap(),
        vm: Uuid::new_v4().into(),
        owner: "task".into(),
    });
    assert_eq!(
        f.manager.perform_image_operation(&request).unwrap()["state"],
        "failed"
    );
}

#[test]
fn interrupted_original_is_queried_and_cancelled_without_replay() {
    let f = Fixture::new();
    let image = f.import();
    f.approve(image);
    let vm = Uuid::new_v4().into();
    let request = f.request(ImageAction::Branch {
        image,
        sha256: f.manager.image_content_digest(image).unwrap(),
        vm,
        owner: "task".into(),
    });
    let sha = hash(
        &serde_json::to_vec(&json!({"caller":request.caller,"action":request.action})).unwrap(),
    );
    f.db()
        .execute(
            "INSERT INTO image_operations VALUES (?1,?2,?3,?4,'running',NULL,0)",
            rusqlite::params![
                request.request.to_string(),
                request.caller,
                sha,
                serde_json::to_string(&request.action).unwrap()
            ],
        )
        .unwrap();
    assert_eq!(
        f.manager.perform_image_operation(&request).unwrap()["state"],
        "unknown"
    );
    assert!(f.manager.list().unwrap().is_empty());
    let wrong = ImageQuery {
        caller: "another-host".into(),
        ..f.query(request.request)
    };
    assert!(f.manager.image_operation_status(&wrong).is_err());
    assert_eq!(
        f.manager
            .cancel_image_operation(&f.query(request.request))
            .unwrap()["state"],
        "unknown"
    );
    assert_eq!(
        f.manager
            .reconcile_image_operation(&f.query(request.request))
            .unwrap()["state"],
        "cancelled"
    );
    assert_eq!(
        f.manager.perform_image_operation(&request).unwrap()["state"],
        "cancelled"
    );
    assert!(f.manager.list().unwrap().is_empty());
}

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn cancellation_before_admission_seals_the_original_identity() {
    let f = Fixture::new();
    let image = f.import();
    f.approve(image);
    let request = f.request(ImageAction::Branch {
        image,
        sha256: f.manager.image_content_digest(image).unwrap(),
        vm: Uuid::new_v4().into(),
        owner: "late-worker".into(),
    });
    assert_eq!(
        f.manager
            .cancel_image_operation(&f.query(request.request))
            .unwrap()["state"],
        "cancelled"
    );
    assert!(f.manager.perform_image_operation(&request).is_err());
    assert!(f.manager.list().unwrap().is_empty());
}
