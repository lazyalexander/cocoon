use crate::{
    FinishPolicy, ImageId, Manager, OperationId, Result, VmId, VmState,
    disk::{copy_new, sync_directory},
};
use std::fs;
use uuid::Uuid;

impl Manager {
    pub fn save_candidate(&self, id: VmId) -> Result<ImageId> {
        let _lease = crate::lifecycle::working_disk_lease(self, id)?;
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
        let dir = self.root.join("vms").join(id.to_string());
        let current = dir.join("root.raw");
        let next = dir.join("root.rollback-new");
        let previous = dir.join("root.rollback-old");
        copy_new(
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
        vm.image = baseline;
        vm.state = VmState::Stopped;
        vm.active_attempt = None;
        vm.active_lease = None;
        vm.suspend_state = None;
        self.save_vm(&vm)?;
        self.db.execute(
            "UPDATE operations SET status='complete' WHERE id=?1",
            [operation.to_string()],
        )?;
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
