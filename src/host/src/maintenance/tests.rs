use super::*;
use crate::{FinishPolicy, ImageQuery, ImageRequest};
use std::{os::unix::fs::symlink, path::PathBuf};
use uuid::Uuid;

struct Fixture {
    root: PathBuf,
    manager: Manager,
    image: ImageId,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("cocoon-maintenance-{}", Uuid::new_v4()));
        let manager = Manager::open(&root).unwrap();
        let input = root.join("input");
        fs::write(&input, b"fixture, not a VM").unwrap();
        let image = manager
            .import_image(&input, &input, &input, "test MIT")
            .unwrap();
        Self {
            root,
            manager,
            image,
        }
    }
    fn request(&self) -> ImageRequest {
        ImageRequest {
            request: Uuid::new_v4().into(),
            caller: "operator".into(),
            credential: self.manager.api_credential().unwrap(),
            action: ImageAction::Retire {
                image: self.image,
                sha256: self.manager.image_content_digest(self.image).unwrap(),
            },
        }
    }
    fn query(&self, r: &ImageRequest) -> ImageQuery {
        ImageQuery {
            request: r.request,
            caller: r.caller.clone(),
            credential: r.credential.clone(),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn retirement_keeps_receipts_tombstone_and_external_bytes() {
    let f = Fixture::new();
    let request = f.request();
    let a = f.manager.perform_image_operation(&request).unwrap();
    assert_eq!(a["state"], "succeeded");
    assert_eq!(f.manager.perform_image_operation(&request).unwrap(), a);
    assert!(f.manager.image(f.image).unwrap()["retired"] == true);
    assert!(!f.root.join("images").join(f.image.to_string()).exists());
    assert!(f.root.join("input").exists());
    assert!(
        f.manager
            .create(Uuid::new_v4().into(), f.image, "task", FinishPolicy::Retain)
            .is_err()
    );
    assert_eq!(
        f.manager
            .cancel_image_operation(&f.query(&request))
            .unwrap()["state"],
        "succeeded"
    );
    let reopened = Manager::open(&f.root).unwrap();
    assert_eq!(
        reopened.image_operation_status(&f.query(&request)).unwrap()["state"],
        "succeeded"
    );
}
#[test]
fn late_vm_reference_and_unknown_owners_block_without_deleting() {
    let f = Fixture::new();
    let before = f.manager.resource_inventory().unwrap();
    assert_eq!(before["resources"][0]["reclaimable"], true);
    let vm = Uuid::new_v4().into();
    f.manager
        .create(vm, f.image, "task", FinishPolicy::Retain)
        .unwrap();
    assert_eq!(
        f.manager.perform_image_operation(&f.request()).unwrap()["state"],
        "failed"
    );
    let inventory = f.manager.resource_inventory().unwrap();
    assert_eq!(inventory["resources"][0]["reclaimable"], false);
    assert_eq!(inventory["resources"][1]["reclaimable"], false);
    assert!(f.root.join("images").join(f.image.to_string()).exists());
}
#[test]
fn symlink_and_hardlink_resources_remain_untouched() {
    for hard in [false, true] {
        let f = Fixture::new();
        let disk = f
            .root
            .join("images")
            .join(f.image.to_string())
            .join("root.raw");
        fs::remove_file(&disk).unwrap();
        if hard {
            fs::hard_link(f.root.join("input"), &disk).unwrap();
        } else {
            symlink(f.root.join("input"), &disk).unwrap();
        }
        assert_eq!(
            f.manager.perform_image_operation(&f.request()).unwrap()["state"],
            "failed"
        );
        assert!(disk.exists());
    }
}
#[test]
fn active_reader_and_cancel_before_admission_never_remove_data() {
    let f = Fixture::new();
    let request = f.request();
    let read = f.manager.image_use_lease(false).unwrap();
    assert_eq!(
        f.manager.perform_image_operation(&request).unwrap()["state"],
        "unknown"
    );
    drop(read);
    f.manager
        .cancel_image_operation(&f.query(&request))
        .unwrap();
    assert_eq!(
        f.manager
            .reconcile_image_operation(&f.query(&request))
            .unwrap()["state"],
        "cancelled"
    );
    let never = f.request();
    f.manager.cancel_image_operation(&f.query(&never)).unwrap();
    assert!(f.manager.perform_image_operation(&never).is_err());
    assert!(
        f.root
            .join("images")
            .join(f.image.to_string())
            .join("root.raw")
            .exists()
    );
}
#[test]
fn interrupted_delete_requires_original_reconcile_even_after_cancel() {
    // Exercise a reachable durable prefix, then remove one file as a crashed unlink would.
    let f = Fixture::new();
    let request = f.request();
    let sha =
        crate::image_management::digest(&json!({"caller":request.caller,"action":request.action}))
            .unwrap();
    f.manager
        .db()
        .execute(
            "INSERT INTO image_operations VALUES (?1,?2,?3,?4,'running',NULL,0)",
            params![
                request.request.to_string(),
                request.caller,
                sha,
                serde_json::to_string(&request.action).unwrap()
            ],
        )
        .unwrap();
    f.manager
        .persist_maintenance_intent(request.request, "retire", &f.image.to_string())
        .unwrap();
    fs::remove_file(
        f.root
            .join("images")
            .join(f.image.to_string())
            .join("kernel"),
    )
    .unwrap();
    let reopened = Manager::open(&f.root).unwrap();
    assert_eq!(
        reopened.cancel_image_operation(&f.query(&request)).unwrap()["state"],
        "unknown"
    );
    let settled = reopened
        .reconcile_image_operation(&f.query(&request))
        .unwrap();
    assert_eq!(settled["state"], "succeeded");
    assert_eq!(
        reopened.image_operation_status(&f.query(&request)).unwrap(),
        settled
    );
    assert_eq!(
        reopened.resource_inventory().unwrap()["resources"][0]["retired"],
        true
    );
}
