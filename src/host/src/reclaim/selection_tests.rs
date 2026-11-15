use super::*;
use crate::FinishPolicy;
use uuid::Uuid;

#[test]
fn durable_reclaim_fence_rechecks_revision_generation_and_cancellation() {
    // Metadata-only fixture: this tests the transaction, not VM termination qualification.
    for conflict in ["revision", "generation", "cancellation", "none"] {
        let root =
            std::env::temp_dir().join(format!("cocoon-reclaim-selection-{}", Uuid::new_v4()));
        let manager = Manager::open(&root).unwrap();
        let input = root.join("input");
        fs::write(&input, b"fixture").unwrap();
        let image = manager
            .import_image(&input, &input, &input, "test MIT")
            .unwrap();
        let vm = VmId::from(Uuid::new_v4());
        manager
            .create(vm, image, "operator", FinishPolicy::Retain)
            .unwrap();
        let selected = manager.get(vm).unwrap();
        let request = RequestId::from(Uuid::new_v4());
        manager.db().execute("INSERT INTO image_operations VALUES (?1,'operator','fixture','null','running',NULL,0)",[request.to_string()]).unwrap();
        match conflict {
            "revision" => manager.set_priority(vm, 1).unwrap(),
            "generation" => {
                manager
                    .db()
                    .execute(
                        "UPDATE vms SET record=json_set(record,'$.generation',1) WHERE id=?1",
                        [vm.to_string()],
                    )
                    .unwrap();
            }
            "cancellation" => {
                manager
                    .db()
                    .execute(
                        "UPDATE image_operations SET cancel_requested=1 WHERE id=?1",
                        [request.to_string()],
                    )
                    .unwrap();
            }
            _ => {}
        }
        {
            let db = manager.db();
            let tx = db.unchecked_transaction().unwrap();
            let result = Manager::bind_reclaim_selection(
                &tx,
                vm,
                &ReclaimSelection {
                    request,
                    generation: selected.generation,
                    revision: selected.revision,
                },
            );
            if conflict == "none" {
                result.unwrap();
                tx.execute(
                    "INSERT INTO reclaim_intents VALUES (?1,'pending')",
                    [vm.to_string()],
                )
                .unwrap();
                assert_eq!(
                    tx.query_row(
                        "SELECT COUNT(*) FROM maintenance_intents WHERE request=?1",
                        [request.to_string()],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                    1
                );
            } else {
                assert!(
                    result.is_err(),
                    "stale or cancelled selection crossed the durable fence"
                );
            }
            tx.rollback().unwrap();
        }
        assert!(!manager.maintenance_started(request).unwrap());
        assert!(manager.reclaim_status(vm).unwrap().is_none());
        assert!(
            root.join("vms")
                .join(vm.to_string())
                .join("root.raw")
                .is_file()
        );
        drop(manager);
        fs::remove_dir_all(root).unwrap();
    }
}
