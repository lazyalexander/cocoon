//! Image bytes stay private until registration and the original receipt commit together.
use crate::{
    FinishPolicy, ImageAction, ImageId, Manager, OperationId, RequestId, Result, VmId, VmState,
    disk::{copy_controlled, hash_controlled, sync_directory},
    image_management::{commit_receipt, digest},
};
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};
use uuid::Uuid;

pub(crate) struct Publication {
    pub id: ImageId,
    pub approved: bool,
    pub metadata: Value,
    pub receipt: Option<RequestId>,
}
impl Manager {
    pub(crate) fn verify_image_controlled(
        &self,
        id: ImageId,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<()> {
        let manifest = self.image(id)?;
        for name in ["kernel", "initrd", "root.raw"] {
            if manifest["files"][name]["sha256"]
                != hash_controlled(
                    &self.root.join("images").join(id.to_string()).join(name),
                    check,
                )?
            {
                return Err("image integrity mismatch".into());
            }
        }
        Ok(())
    }
    pub(crate) fn import_controlled(
        &self,
        sources: [&Path; 3],
        provenance: &str,
        publication: Publication,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<ImageId> {
        if provenance.trim().is_empty() {
            return Err("provenance and license information required".into());
        }
        check()?;
        let id = publication.id;
        let operation = OperationId::from(Uuid::new_v4());
        self.begin(operation, "import_image", id)?;
        let directory = self.root.join("images").join(id.to_string());
        fs::create_dir(&directory)?;
        let mut files = serde_json::Map::new();
        for (name, source) in ["kernel", "initrd", "root.raw"].into_iter().zip(sources) {
            let target = directory.join(name);
            let copy = copy_controlled(source, &target, check)?;
            let sha = hash_controlled(&target, check)?;
            if let Some(expected) = publication.metadata.get("expected_files")
                && expected[name] != sha
            {
                return Err("image bytes changed during publication".into());
            }
            files.insert(name.into(), json!({"sha256":sha,"copy":copy}));
            fs::set_permissions(target, fs::Permissions::from_mode(0o400))?;
        }
        let mut manifest = json!({"id":id,"files":files,"provenance":provenance,"trusted":false,"approved":publication.approved,"boot_qualified":false});
        if let Some(fields) = publication.metadata.as_object() {
            for (key, value) in fields {
                if key != "expected_files" {
                    manifest[key] = value.clone();
                }
            }
        }
        sync_directory(&directory)?;
        sync_directory(&self.root.join("images"))?;
        check()?;
        let sha = digest(
            &json!({"kernel":manifest["files"]["kernel"]["sha256"],"initrd":manifest["files"]["initrd"]["sha256"],"root.raw":manifest["files"]["root.raw"]["sha256"]}),
        )?;
        let db = self.db();
        let tx = db.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO images VALUES (?1,?2)",
            rusqlite::params![id.to_string(), manifest.to_string()],
        )?;
        tx.execute(
            "UPDATE operations SET status='complete' WHERE id=?1",
            [operation.to_string()],
        )?;
        if let Some(request) = publication.receipt {
            commit_receipt(
                &tx,
                request,
                &json!({"image":id,"sha256":sha,"approved":publication.approved}),
            )?;
        }
        tx.commit()?;
        Ok(id)
    }
    pub(crate) fn image_source_ready(&self, id: VmId) -> Result<()> {
        let vm = self.get(id)?;
        if vm.active_lease.is_some() {
            return Err("image source still leased".into());
        }
        let unsettled:bool=self.db().query_row("SELECT EXISTS(SELECT 1 FROM operations WHERE kind='rollback' AND target=?1 AND status!='complete') OR EXISTS(SELECT 1 FROM rollback_intents WHERE vm=?1)",[id.to_string()],|r|r.get(0))?;
        if unsettled {
            return Err("original rollback requires reconciliation".into());
        }
        if vm.state == VmState::Stopped && vm.active_attempt.is_none() {
            let restored:bool=self.db().query_row("SELECT EXISTS(SELECT 1 FROM image_operations WHERE state='succeeded' AND json_extract(action,'$.operation')='rollback' AND json_extract(action,'$.vm')=?1 AND json_extract(action,'$.generation')=?2 AND json_extract(result,'$.image')=?3 AND json_extract(result,'$.vm')=?1)",rusqlite::params![id.to_string(),i64::try_from(vm.generation).map_err(|_|crate::Error::from("image generation exceeds store limit"))?,vm.image.to_string()],|r|r.get(0))?;
            if restored {
                return Ok(());
            }
        }
        if matches!(vm.state, VmState::Prepared | VmState::Stopped)
            && vm.generation == 0
            && vm.active_attempt.is_none()
        {
            let created:bool=self.db().query_row("SELECT EXISTS(SELECT 1 FROM operations WHERE id=?1 AND kind='create_vm' AND status='complete')",[id.to_string()],|r|r.get(0))?;
            if created {
                return Ok(());
            }
        }
        if vm.state == VmState::Stopped
            && let Some(attempt) = vm.active_attempt
            && self.execution_stop_confirmed(&self.execution(attempt)?)?
        {
            return Ok(());
        }
        Err("image source lacks original termination evidence".into())
    }
    pub(crate) fn apply_image_action(
        &self,
        request: RequestId,
        action: &ImageAction,
    ) -> Result<()> {
        let check = || self.check_image_cancel(request);
        match action {
            ImageAction::ImportCandidate {
                environment,
                kernel,
                initrd,
                disk,
                kernel_sha256,
                initrd_sha256,
                disk_sha256,
            } => {
                self.import_controlled([kernel,initrd,disk],"Reviewed offline environment candidate",
                    Publication {id:ImageId::from(request.raw()),approved:false,receipt:Some(request),
                        metadata:json!({"source_environment":environment,"expected_files":{"kernel":kernel_sha256,"initrd":initrd_sha256,"root.raw":disk_sha256}})},&check)?;
            }
            ImageAction::SaveCandidate {
                vm,
                generation,
                disk_sha256,
            } => {
                let _lease = crate::lifecycle::working_disk_lease(self, *vm)?;
                self.image_source_ready(*vm)?;
                let source = self.get(*vm)?;
                let disk = self.root.join("vms").join(vm.to_string()).join("root.raw");
                if source.generation != *generation
                    || hash_controlled(&disk, &check)? != *disk_sha256
                {
                    return Err("image source changed".into());
                }
                let parent = self.image(source.image)?;
                let directory = self.root.join("images").join(source.image.to_string());
                self.import_controlled([&directory.join("kernel"),&directory.join("initrd"),&disk],"Private project candidate",
                    Publication {id:ImageId::from(request.raw()),approved:false,receipt:Some(request),metadata:json!({"parent":source.image,"source_vm":vm,"source_generation":generation,
                        "expected_files":{"kernel":parent["files"]["kernel"]["sha256"],"initrd":parent["files"]["initrd"]["sha256"],"root.raw":disk_sha256}})},&check)?;
            }
            ImageAction::PromoteCandidate { image, sha256 } => {
                self.verify_image_controlled(*image, &check)?;
                let mut manifest = self.image(*image)?;
                manifest["approved"] = json!(true);
                check()?;
                let db = self.db();
                let tx = db.unchecked_transaction()?;
                tx.execute(
                    "UPDATE images SET manifest=?1 WHERE id=?2",
                    rusqlite::params![manifest.to_string(), image.to_string()],
                )?;
                commit_receipt(
                    &tx,
                    request,
                    &json!({"image":image,"sha256":sha256,"approved":true}),
                )?;
                tx.commit()?;
            }
            ImageAction::Branch {
                image, vm, owner, ..
            } => {
                self.create_controlled(
                    *vm,
                    *image,
                    owner,
                    FinishPolicy::Retain,
                    Some(request),
                    &check,
                )?;
            }
            ImageAction::Rollback { .. } => self.rollback_controlled(request, action, &check)?,
        }
        Ok(())
    }
    fn rollback_controlled(
        &self,
        request: RequestId,
        action: &ImageAction,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<()> {
        let ImageAction::Rollback {
            vm: id,
            generation,
            disk_sha256: expected,
            image,
            ..
        } = action
        else {
            return Err("rollback action required".into());
        };
        let (id, generation, image) = (*id, *generation, *image);
        let _lease = crate::lifecycle::working_disk_lease(self, id)?;
        self.recover_rollback(id)?;
        self.image_source_ready(id)?;
        let mut vm = self.get(id)?;
        let directory = self.root.join("vms").join(id.to_string());
        let current = directory.join("root.raw");
        if vm.generation != generation || hash_controlled(&current, check)? != *expected {
            return Err("rollback target changed".into());
        }
        self.verify_image_controlled(image, check)?;
        let operation = OperationId::from(request.raw());
        self.begin(operation, "rollback", id)?;
        self.db().execute(
            "INSERT INTO rollback_intents VALUES (?1,?2,?3)",
            rusqlite::params![
                id.to_string(),
                vm.image.to_string(),
                serde_json::to_string(&vm.disk_copy)?
            ],
        )?;
        let next = directory.join("root.rollback-new");
        let previous = directory.join("root.rollback-old");
        let copy = copy_controlled(
            &self
                .root
                .join("images")
                .join(image.to_string())
                .join("root.raw"),
            &next,
            check,
        )?;
        check()?;
        fs::rename(&current, &previous)?;
        fs::rename(&next, &current)?;
        sync_directory(&directory)?;
        vm.image = image;
        vm.disk_copy = Some(copy);
        vm.state = VmState::Stopped;
        vm.active_attempt = None;
        vm.active_lease = None;
        vm.suspend_state = None;
        let db = self.db();
        let tx = db.unchecked_transaction()?;
        Self::write_vm(&tx, &mut vm)?;
        tx.execute(
            "UPDATE operations SET status='complete' WHERE id=?1",
            [operation.to_string()],
        )?;
        tx.execute("DELETE FROM rollback_intents WHERE vm=?1", [id.to_string()])?;
        commit_receipt(&tx, request, &json!({"vm":id,"image":image}))?;
        tx.commit()?;
        // The committed receipt wins over late cancellation or optional old-disk cleanup.
        let _ = fs::remove_file(previous);
        Ok(())
    }
}

#[cfg(test)]
#[path = "image_effects_tests.rs"]
mod tests;
