use crate::{
    FinishPolicy, ImageId, Manager, OperationId, Result, VmId, VmState,
    disk::{copy_new, sync_directory},
};
use rusqlite::OptionalExtension;
use std::fs;
use uuid::Uuid;

impl Manager {
    pub(crate) fn recover_rollback(&self, id: VmId) -> Result<()> {
        let dir = self.root.join("vms").join(id.to_string());
        let current = dir.join("root.raw");
        let next = dir.join("root.rollback-new");
        let previous = dir.join("root.rollback-old");
        if previous.exists() {
            let original: Option<(String, Option<String>)> = self
                .db
                .query_row(
                    "SELECT image,disk_copy FROM rollback_intents WHERE vm=?1",
                    [id.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let status: Option<String> = self.db.query_row(
                "SELECT status FROM operations WHERE kind='rollback' AND target=?1 ORDER BY rowid DESC LIMIT 1",
                [id.to_string()], |row| row.get(0)).optional()?;
            if status.as_deref() == Some("complete") {
                fs::remove_file(previous)?;
                self.db
                    .execute("DELETE FROM rollback_intents WHERE vm=?1", [id.to_string()])?;
                sync_directory(&dir)?;
                return Ok(());
            }
            if current.exists() {
                let failed = dir.join("root.rollback-failed");
                fs::rename(&current, failed)?;
            }
            fs::rename(previous, &current)?;
            sync_directory(&dir)?;
            let mut vm = self.get(id)?;
            if let Some((image, disk_copy)) = original {
                vm.image = ImageId::from(
                    Uuid::parse_str(&image)
                        .map_err(|_| crate::Error::from("invalid rollback image"))?,
                );
                vm.disk_copy = serde_json::from_str(&disk_copy.unwrap_or("null".into()))?;
            }
            vm.state = VmState::RecoveryRequired;
            vm.active_lease = None;
            vm.suspend_state = None;
            self.save_vm(&mut vm)?;
            self.db
                .execute("DELETE FROM rollback_intents WHERE vm=?1", [id.to_string()])?;
            return Err(
                "interrupted rollback restored the previous disk and metadata; recovery approval is required"
                    .into(),
            );
        }
        if !current.exists() {
            return Err("working disk is missing".into());
        }
        if next.exists() {
            fs::remove_file(next)?;
            sync_directory(&dir)?;
        }
        Ok(())
    }

    pub fn save_candidate(&self, id: VmId) -> Result<ImageId> {
        let _lease = crate::lifecycle::working_disk_lease(self, id)?;
        self.recover_rollback(id)?;
        let vm = self.get(id)?;
        if !matches!(vm.state, VmState::Stopped | VmState::Suspended) || vm.active_lease.is_some() {
            return Err("candidate requires an inactive VM".into());
        }
        let image = self.root.join("images").join(vm.image.to_string());
        let disk = self.root.join("vms").join(id.to_string()).join("root.raw");
        self.append_journal(
            id,
            "candidate_intent",
            serde_json::json!({"generation": vm.generation}),
        )?;
        let candidate = self.import_record(&image.join("kernel"), &image.join("initrd"), &disk,
            "Unapproved guest-derived candidate; source provenance retained in parent metadata", false,
            serde_json::json!({"parent": vm.image, "source_vm": id, "source_generation": vm.generation}))?;
        self.append_journal(
            id,
            "candidate_complete",
            serde_json::json!({"candidate": candidate}),
        )?;
        Ok(candidate)
    }

    pub fn promote_candidate(&self, id: ImageId) -> Result<()> {
        self.verify_image(id)?;
        let mut manifest = self.image(id)?;
        if manifest["source_vm"].is_null() || manifest["approved"] == true {
            return Err("image is not an unapproved candidate".into());
        }
        let vm = VmId::from(
            Uuid::parse_str(
                manifest["source_vm"]
                    .as_str()
                    .ok_or("invalid candidate source")?,
            )
            .map_err(|_| crate::Error::from("invalid candidate source"))?,
        );
        self.append_journal(
            vm,
            "candidate_approval_intent",
            serde_json::json!({"candidate": id}),
        )?;
        manifest["approved"] = true.into();
        self.db.execute(
            "UPDATE images SET manifest=?1 WHERE id=?2",
            rusqlite::params![manifest.to_string(), id.to_string()],
        )?;
        self.append_journal(
            vm,
            "candidate_approved",
            serde_json::json!({"candidate": id}),
        )?;
        Ok(())
    }

    pub fn rollback(&self, id: VmId, baseline: ImageId) -> Result<()> {
        let _lease = crate::lifecycle::working_disk_lease(self, id)?;
        self.recover_rollback(id)?;
        let mut vm = self.get(id)?;
        if !matches!(vm.state, VmState::Stopped | VmState::Suspended) || vm.active_lease.is_some() {
            return Err("rollback requires an inactive VM".into());
        }
        if self.image(baseline)?["approved"] != true {
            return Err("rollback baseline is not approved".into());
        }
        self.verify_image(baseline)?;
        self.append_journal(
            id,
            "rollback_intent",
            serde_json::json!({"baseline": baseline}),
        )?;
        let operation = OperationId::from(Uuid::new_v4());
        self.begin(operation, "rollback", id)?;
        self.db.execute(
            "INSERT INTO rollback_intents VALUES (?1,?2,?3)",
            rusqlite::params![
                id.to_string(),
                vm.image.to_string(),
                serde_json::to_string(&vm.disk_copy)?
            ],
        )?;
        let dir = self.root.join("vms").join(id.to_string());
        let current = dir.join("root.raw");
        let next = dir.join("root.rollback-new");
        let previous = dir.join("root.rollback-old");
        let disk_copy = copy_new(
            &self
                .root
                .join("images")
                .join(baseline.to_string())
                .join("root.raw"),
            &next,
        )?;
        fs::rename(&current, &previous)?;
        fs::rename(&next, &current)?;
        sync_directory(&dir)?;
        vm.disk_copy = Some(disk_copy);
        vm.image = baseline;
        vm.state = VmState::Stopped;
        vm.active_attempt = None;
        vm.active_lease = None;
        vm.suspend_state = None;
        self.save_vm(&mut vm)?;
        self.db.execute(
            "UPDATE operations SET status='complete' WHERE id=?1",
            [operation.to_string()],
        )?;
        self.db
            .execute("DELETE FROM rollback_intents WHERE vm=?1", [id.to_string()])?;
        fs::remove_file(previous)?;
        self.append_journal(
            id,
            "rollback_complete",
            serde_json::json!({"baseline": baseline}),
        )?;
        Ok(())
    }

    pub fn branch(&self, source: ImageId, id: VmId, owner: &str) -> Result<crate::Vm> {
        if self.image(source)?["approved"] != true {
            return Err("branch source is not approved".into());
        }
        self.verify_image(source)?;
        let vm = self.create(id, source, owner, FinishPolicy::Retain)?;
        self.append_journal(id, "branch_complete", serde_json::json!({"source": source}))?;
        Ok(vm)
    }
}
