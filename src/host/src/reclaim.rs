//! Operator-owned VM reclamation. Pending intents require explicit reconciliation.
use crate::{Manager, Result, VmId, VmState, disk::sync_directory, lifecycle::working_disk_lease};
use rusqlite::{OptionalExtension, params};
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
};

fn reclaim_lease(manager: &Manager, id: VmId) -> Result<fs::File> {
    let path = manager
        .root
        .join("artifacts")
        .join(format!("{id}.reclaim.lock"));
    let file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)?;
    file.try_lock()
        .map_err(|_| crate::Error::from("reclaim is already in progress"))?;
    Ok(file)
}

impl Manager {
    pub fn reclaim_status(&self, id: VmId) -> Result<Option<String>> {
        Ok(self
            .db()
            .query_row(
                "SELECT status FROM reclaim_intents WHERE vm=?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Reclaim a confirmed-stopped VM. Exported artifacts move to host-owned storage.
    pub fn reclaim(&self, id: VmId) -> Result<()> {
        let _reclaim_lease = reclaim_lease(self, id)?;
        let _lease = working_disk_lease(self, id)?;
        let vm = self.get(id)?;
        if !matches!(vm.state, VmState::Stopped | VmState::RecoveryRequired)
            || vm.active_lease.is_some()
        {
            return Err("reclaim requires a confirmed stopped VM".into());
        }
        let attempt = vm
            .active_attempt
            .ok_or("stopped VM has no execution attempt")?;
        let status = self.execution(attempt)?.status;
        if (vm.state == VmState::Stopped && status != "stopped")
            || (vm.state == VmState::RecoveryRequired && status != "recovery_required")
        {
            return Err("reclaim requires a confirmed stopped execution".into());
        }
        let db = self.db();
        let unresolved: i64 = db.query_row(
            "SELECT COUNT(*) FROM consumer_operations WHERE vm=?1 AND status IN ('intended','unknown')",
            [id.to_string()],
            |row| row.get(0),
        )?;
        let pending: i64 = db.query_row(
            "SELECT COUNT(*) FROM operations WHERE target=?1 AND status!='complete'",
            [id.to_string()],
            |row| row.get(0),
        )?;
        let rollback: i64 = db.query_row(
            "SELECT COUNT(*) FROM rollback_intents WHERE vm=?1",
            [id.to_string()],
            |row| row.get(0),
        )?;
        if unresolved != 0 || pending != 0 || rollback != 0 {
            return Err("reclaim requires reconciliation of pending operations".into());
        }
        db.execute(
            "INSERT INTO reclaim_intents(vm,status) VALUES (?1,'pending')",
            [id.to_string()],
        )?;
        drop(db);
        // A failed cleanup leaves the durable intent and blocks any new boot.
        self.finish_reclaim(id)
    }

    /// Explicitly finish an interrupted reclaim; never infer that deletion succeeded.
    pub fn reconcile_reclaim(&self, id: VmId) -> Result<()> {
        let _reclaim_lease = reclaim_lease(self, id)?;
        if self.reclaim_status(id)?.as_deref() != Some("pending") {
            return Err("no pending reclaim intent".into());
        }
        self.finish_reclaim(id)
    }

    fn finish_reclaim(&self, id: VmId) -> Result<()> {
        let vm_dir = self.root.join("vms").join(id.to_string());
        let archive = self.root.join("artifacts").join(id.to_string());
        if !archive.exists() {
            fs::DirBuilder::new().mode(0o700).create(&archive)?;
            sync_directory(&self.root.join("artifacts"))?;
        }
        if !fs::symlink_metadata(&archive)?.is_dir() {
            return Err("unsafe artifact directory during reclaim".into());
        }
        if vm_dir.exists() {
            if !fs::symlink_metadata(&vm_dir)?.is_dir() {
                return Err("unsafe VM directory during reclaim".into());
            }
            for entry in fs::read_dir(&vm_dir)? {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                let source = entry.path().join("exports");
                if !source.exists() {
                    continue;
                }
                if !fs::symlink_metadata(&source)?.is_dir() {
                    return Err("unsafe export directory during reclaim".into());
                }
                let destination = archive.join(entry.file_name());
                if destination.exists() {
                    return Err("export archive collision during reclaim".into());
                }
                fs::rename(&source, &destination)?;
                sync_directory(&archive)?;
                sync_directory(&entry.path())?;
            }
            fs::remove_dir_all(&vm_dir)?;
            sync_directory(&self.root.join("vms"))?;
        }
        let db = self.db();
        let tx = db.unchecked_transaction()?;
        tx.execute(
            "UPDATE work_grants SET revoked=1 WHERE vm=?1",
            [id.to_string()],
        )?;
        tx.execute("DELETE FROM vms WHERE id=?1", [id.to_string()])?;
        tx.execute(
            "UPDATE reclaim_intents SET status='complete' WHERE vm=?1 AND status='pending'",
            params![id.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FinishPolicy, HostOperation, HostRequest, RequestId};
    use std::path::Path;
    use uuid::Uuid;

    #[test]
    fn prepared_vm_and_work_grant_cannot_reclaim() {
        let root = std::env::temp_dir().join(format!("cocoon-reclaim-test-{}", Uuid::new_v4()));
        let manager = Manager::open(&root).unwrap();
        let input = root.join("input");
        fs::write(&input, b"not a bootable fixture").unwrap();
        let image = manager
            .import_image(&input, &input, &input, "Test fixture; MIT")
            .unwrap();
        let vm = VmId::from(Uuid::new_v4());
        manager
            .create(vm, image, "operator", FinishPolicy::Retain)
            .unwrap();
        assert!(manager.reclaim(vm).is_err());
        assert!(
            root.join("vms")
                .join(vm.to_string())
                .join("root.raw")
                .exists()
        );
        let response = manager.handle_api(
            HostRequest {
                request: RequestId::from(Uuid::new_v4()),
                caller: "agent".into(),
                credential: String::new(),
                grant: "0".repeat(64),
                operation: HostOperation::Reclaim { vm },
            },
            Path::new("/unused-runner"),
        );
        assert!(response.result.is_err());
        assert!(manager.reclaim_status(vm).unwrap().is_none());
        drop(manager);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interrupted_reclaim_keeps_exports_and_reserves_identity() {
        for phase in ["intent", "archived", "directory_removed"] {
            let root = std::env::temp_dir().join(format!("cocoon-reclaim-test-{}", Uuid::new_v4()));
            let manager = Manager::open(&root).unwrap();
            let input = root.join("input");
            fs::write(&input, b"not a bootable fixture").unwrap();
            let image = manager
                .import_image(&input, &input, &input, "Test fixture; MIT")
                .unwrap();
            let vm = VmId::from(Uuid::new_v4());
            manager
                .create(vm, image, "operator", FinishPolicy::Retain)
                .unwrap();
            let attempt = Uuid::new_v4().to_string();
            let vm_dir = root.join("vms").join(vm.to_string());
            let exports = vm_dir.join(&attempt).join("exports");
            fs::create_dir_all(&exports).unwrap();
            fs::write(exports.join("result.bin"), b"retained artifact").unwrap();
            manager
                .db()
                .execute(
                    "INSERT INTO reclaim_intents(vm,status) VALUES (?1,'pending')",
                    [vm.to_string()],
                )
                .unwrap();
            if phase != "intent" {
                let archive = root.join("artifacts").join(vm.to_string());
                fs::create_dir(&archive).unwrap();
                fs::rename(&exports, archive.join(&attempt)).unwrap();
            }
            if phase == "directory_removed" {
                fs::remove_dir_all(&vm_dir).unwrap();
            }
            manager.reconcile_reclaim(vm).unwrap();
            assert_eq!(
                fs::read(
                    root.join("artifacts")
                        .join(vm.to_string())
                        .join(&attempt)
                        .join("result.bin")
                )
                .unwrap(),
                b"retained artifact"
            );
            assert!(!vm_dir.exists());
            assert!(manager.get(vm).is_err());
            assert!(
                manager
                    .create(vm, image, "operator", FinishPolicy::Retain)
                    .is_err()
            );
            assert_eq!(
                manager.reclaim_status(vm).unwrap().as_deref(),
                Some("complete")
            );
            drop(manager);
            fs::remove_dir_all(root).unwrap();
        }
    }
}
