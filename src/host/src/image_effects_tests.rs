use super::*;
use crate::{ImageQuery, ImageRequest};
use std::cell::Cell;

struct Fixture {
    root: std::path::PathBuf,
    manager: Manager,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("cocoon-image-window-{}", Uuid::new_v4()));
        let manager = Manager::open(&root).unwrap();
        manager.initialize_image_api().unwrap();
        Self { root, manager }
    }
    fn image(&self, bytes: &[u8]) -> ImageId {
        let source = self.root.join(Uuid::new_v4().to_string());
        fs::write(&source, bytes).unwrap();
        self.manager
            .import_image(&source, &source, &source, "nonbootable fault fixture")
            .unwrap()
    }
    fn admit(&self, action: ImageAction) -> ImageRequest {
        let request = ImageRequest {
            request: Uuid::new_v4().into(),
            caller: "test-host".into(),
            credential: fs::read_to_string(self.root.join("api-credential")).unwrap(),
            action,
        };
        self.manager
            .db()
            .execute(
                "INSERT INTO image_operations VALUES (?1,?2,?3,?4,'running',NULL,0)",
                rusqlite::params![
                    request.request.to_string(),
                    request.caller,
                    digest(&json!({"caller":request.caller,"action":request.action})).unwrap(),
                    serde_json::to_string(&request.action).unwrap()
                ],
            )
            .unwrap();
        request
    }
    fn cancel(&self, request: &ImageRequest) {
        let query = ImageQuery {
            request: request.request,
            caller: request.caller.clone(),
            credential: request.credential.clone(),
        };
        assert_eq!(
            self.manager.image_operation_status(&query).unwrap()["state"],
            "unknown"
        );
        self.manager.cancel_image_operation(&query).unwrap();
        assert_eq!(
            self.manager.reconcile_image_operation(&query).unwrap()["state"],
            "cancelled"
        );
        assert_eq!(
            self.manager.perform_image_operation(request).unwrap()["state"],
            "cancelled"
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn branch_copy_error_after_bytes_exist_cannot_publish_registration_or_receipt() {
    let f = Fixture::new();
    let image = f.image(b"original");
    let vm = VmId::from(Uuid::new_v4());
    let request = f.admit(ImageAction::Branch {
        image,
        sha256: f.manager.image_content_digest(image).unwrap(),
        vm,
        owner: "test-host".into(),
    });
    let staged = f.root.join("vms").join(vm.to_string()).join("root.raw");
    let hit = Cell::new(false);
    let check = || {
        if staged.exists() {
            hit.set(true);
            return Err(std::io::Error::from_raw_os_error(28).into());
        }
        Ok(())
    };
    assert!(
        f.manager
            .create_controlled(
                vm,
                image,
                "test-host",
                FinishPolicy::Retain,
                Some(request.request),
                &check
            )
            .is_err()
    );
    assert!(hit.get() && staged.exists());
    assert!(f.manager.get(vm).is_err());
    f.cancel(&request);
    assert!(f.manager.get(vm).is_err());
}

#[test]
fn rollback_copy_error_closes_only_the_original_intent_after_exact_old_disk_proof() {
    let f = Fixture::new();
    let image = f.image(b"original");
    let vm = VmId::from(Uuid::new_v4());
    f.manager
        .create(vm, image, "test-host", FinishPolicy::Retain)
        .unwrap();
    let disk = f.root.join("vms").join(vm.to_string()).join("root.raw");
    fs::write(&disk, b"working state").unwrap();
    let old = crate::disk::hash(&disk).unwrap();
    let request = f.admit(ImageAction::Rollback {
        vm,
        generation: 0,
        disk_sha256: old.clone(),
        image,
        sha256: f.manager.image_content_digest(image).unwrap(),
    });
    let staged = disk.with_file_name("root.rollback-new");
    let hit = Cell::new(false);
    let check = || {
        if staged.exists() {
            hit.set(true);
            return Err(std::io::Error::from_raw_os_error(28).into());
        }
        Ok(())
    };
    assert!(
        f.manager
            .rollback_controlled(request.request, &request.action, &check)
            .is_err()
    );
    assert!(hit.get());
    f.cancel(&request);
    assert_eq!(crate::disk::hash(&disk).unwrap(), old);
    let pending: i64 = f
        .manager
        .db()
        .query_row(
            "SELECT count(*) FROM rollback_intents WHERE vm=?1",
            [vm.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pending, 0);
    assert_eq!(f.manager.get(vm).unwrap().state, VmState::Prepared);
}

#[test]
fn uncommitted_disk_swap_restores_previous_bytes_without_a_success_receipt() {
    let f = Fixture::new();
    let image = f.image(b"original");
    let vm = VmId::from(Uuid::new_v4());
    let record = f
        .manager
        .create(vm, image, "test-host", FinishPolicy::Retain)
        .unwrap();
    let directory = f.root.join("vms").join(vm.to_string());
    let disk = directory.join("root.raw");
    fs::write(&disk, b"working state").unwrap();
    let old = crate::disk::hash(&disk).unwrap();
    let request = f.admit(ImageAction::Rollback {
        vm,
        generation: 0,
        disk_sha256: old.clone(),
        image,
        sha256: f.manager.image_content_digest(image).unwrap(),
    });
    f.manager
        .begin(OperationId::from(request.request.raw()), "rollback", vm)
        .unwrap();
    f.manager
        .db()
        .execute(
            "INSERT INTO rollback_intents VALUES (?1,?2,?3)",
            rusqlite::params![
                vm.to_string(),
                image.to_string(),
                serde_json::to_string(&record.disk_copy).unwrap()
            ],
        )
        .unwrap();
    fs::rename(&disk, directory.join("root.rollback-old")).unwrap();
    fs::write(&disk, b"replacement not committed").unwrap();
    f.cancel(&request);
    assert_eq!(crate::disk::hash(&disk).unwrap(), old);
    assert_eq!(f.manager.get(vm).unwrap().state, VmState::RecoveryRequired);
    assert!(f.manager.working_disk_summary(vm).is_err());
}
