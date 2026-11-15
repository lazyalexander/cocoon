//! Explicit host-owned retirement. Durable intents survive interruption; receipts never disappear.
use crate::{ImageAction, ImageId, Manager, RequestId, Result, disk::sync_directory};
use rusqlite::params;
use serde_json::{Value, json};
use std::{fs, os::unix::fs::OpenOptionsExt};
mod inventory;

impl Manager {
    pub fn maintenance_capabilities(&self) -> Value {
        json!({"schema":"cocoon-image-maintenance-v1","manual_reclaim":true,"automatic_gc":false})
    }
    pub(crate) fn image_use_lease(&self, exclusive: bool) -> Result<fs::File> {
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(self.root.join("image-use.lock"))?;
        if exclusive {
            file.try_lock()
        } else {
            file.try_lock_shared()
        }
        .map_err(|_| crate::Error::from("image data is in use; preserve original operation"))?;
        Ok(file)
    }
    pub(crate) fn maintenance_started(&self, request: RequestId) -> Result<bool> {
        Ok(self.db().query_row(
            "SELECT EXISTS(SELECT 1 FROM maintenance_intents WHERE request=?1)",
            [request.to_string()],
            |r| r.get(0),
        )?)
    }
    fn pending_maintenance_blockers(&self, request: Option<RequestId>) -> Result<bool> {
        Ok(self.db().query_row("SELECT EXISTS(SELECT 1 FROM image_operations WHERE state IN ('running','unknown') AND id!=?1)
            OR EXISTS(SELECT 1 FROM operations WHERE status!='complete')
            OR EXISTS(SELECT 1 FROM consumer_operations WHERE status IN ('intended','unknown'))
            OR EXISTS(SELECT 1 FROM rollback_intents)",
            [request.map(|r|r.to_string()).unwrap_or_default()], |r| r.get(0))?)
    }
    pub(crate) fn retirement_blockers(
        &self,
        image: ImageId,
        request: Option<RequestId>,
    ) -> Result<Vec<String>> {
        let manifest = self.image(image)?;
        let mut reasons = Vec::new();
        if self.unregistered_resource_count()? != 0 {
            reasons.push("unregistered_resources".into());
        }
        if manifest["retired"] == true {
            reasons.push("retired".into());
        }
        if self.pending_maintenance_blockers(request)? {
            reasons.push("unresolved_operations".into());
        }
        if self.list()?.iter().any(|vm| vm.image == image) {
            reasons.push("registered_vm_reference".into());
        }
        let directory = self.root.join("images").join(image.to_string());
        if !fs::symlink_metadata(self.root.join("images"))?.is_dir()
            || !fs::symlink_metadata(&directory).is_ok_and(|m| m.is_dir())
        {
            reasons.push("unsafe_or_missing_directory".into());
        } else {
            let names = ["kernel", "initrd", "root.raw"];
            for entry in fs::read_dir(directory)? {
                let entry = entry?;
                let metadata = entry.metadata()?;
                if !names.iter().any(|name| entry.file_name() == *name)
                    || !entry.file_type()?.is_file()
                    || metadata.nlink() != 1
                {
                    reasons.push("unexpected_or_shared_files".into());
                    break;
                }
            }
            for name in names {
                if !fs::symlink_metadata(
                    self.root.join("images").join(image.to_string()).join(name),
                )
                .is_ok_and(|m| m.is_file() && m.nlink() == 1)
                {
                    reasons.push("unsafe_or_missing_file".into());
                    break;
                }
            }
        }
        Ok(reasons)
    }
    pub(crate) fn validate_maintenance_action(
        &self,
        request: RequestId,
        action: &ImageAction,
    ) -> Result<()> {
        match action {
            ImageAction::Retire { image, sha256 } => {
                if self.image_content_digest(*image)? != *sha256
                    || !self.retirement_blockers(*image, Some(request))?.is_empty()
                {
                    return Err(
                        "image retirement blocked; refresh references and original receipts".into(),
                    );
                }
            }
            ImageAction::ReclaimVm {
                vm,
                generation,
                revision,
            } => {
                let current = self.get(*vm)?;
                if current.generation != *generation || current.revision != *revision {
                    return Err("VM reclaim approval changed".into());
                }
                if self.pending_maintenance_blockers(Some(request))? {
                    return Err("reclaim requires original operation reconciliation".into());
                }
                self.validate_reclaim(*vm)?;
            }
            _ => {}
        }
        Ok(())
    }
    pub(crate) fn begin_maintenance(&self, request: RequestId, action: &ImageAction) -> Result<()> {
        let _exclusive = self.image_use_lease(true)?;
        self.validate_maintenance_action(request, action)?;
        self.check_image_cancel(request)?;
        match action {
            ImageAction::Retire { image, .. } => {
                self.persist_maintenance_intent(request, "retire", &image.to_string())?
            }
            ImageAction::ReclaimVm {
                vm,
                generation,
                revision,
            } => self.reclaim_image_request(*vm, request, *generation, *revision)?,
            _ => return Err("maintenance action required".into()),
        }
        // Past this durable fence cancellation cannot promise to restore removed bytes.
        self.finish_maintenance_locked(request, action)
    }
    fn persist_maintenance_intent(
        &self,
        request: RequestId,
        kind: &str,
        target: &str,
    ) -> Result<()> {
        {
            let db = self.db();
            let tx = db.unchecked_transaction()?;
            tx.execute(
                "INSERT INTO maintenance_intents VALUES (?1,?2,?3,'pending')",
                params![request.to_string(), kind, target],
            )?;
            if kind == "retire" {
                tx.execute("UPDATE images SET manifest=json_set(manifest,'$.retired',json('true'),'$.approved',json('false')) WHERE id=?1", [target])?;
            }
            tx.commit()?;
        }
        Ok(())
    }
    pub(crate) fn finish_maintenance(
        &self,
        request: RequestId,
        action: &ImageAction,
    ) -> Result<()> {
        let _exclusive = self.image_use_lease(true)?;
        self.finish_maintenance_locked(request, action)
    }
    fn finish_maintenance_locked(&self, request: RequestId, action: &ImageAction) -> Result<()> {
        let (kind, target): (String, String) = self.db().query_row(
            "SELECT kind,target FROM maintenance_intents WHERE request=?1 AND status='pending'",
            [request.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let result = match action {
            ImageAction::Retire { image, sha256 }
                if kind == "retire" && target == image.to_string() =>
            {
                let directory = self.root.join("images").join(image.to_string());
                if !fs::symlink_metadata(self.root.join("images"))?.is_dir() {
                    return Err("unsafe image parent".into());
                }
                if directory.try_exists()? {
                    if !fs::symlink_metadata(&directory)?.is_dir() {
                        return Err("unsafe retirement directory".into());
                    }
                    // Do not traverse unexpected content, symlinks, or shared hard links.
                    for entry in fs::read_dir(&directory)? {
                        let entry = entry?;
                        if !["kernel", "initrd", "root.raw"]
                            .iter()
                            .any(|n| entry.file_name() == *n)
                            || !entry.file_type()?.is_file()
                            || entry.metadata()?.nlink() != 1
                        {
                            return Err("retirement encountered unexpected files".into());
                        }
                    }
                    for name in ["kernel", "initrd", "root.raw"] {
                        match fs::remove_file(directory.join(name)) {
                            Ok(()) => {}
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                            Err(e) => return Err(e.into()),
                        }
                    }
                    fs::remove_dir(&directory)?;
                }
                sync_directory(&self.root.join("images"))?;
                json!({"image":image,"sha256":sha256,"retired":true,"artifacts_preserved":true})
            }
            ImageAction::ReclaimVm { vm, .. }
                if kind == "reclaim_vm" && target == vm.to_string() =>
            {
                match self.reclaim_status(*vm)?.as_deref() {
                    Some("complete") => {}
                    Some("pending") => self.reconcile_reclaim(*vm)?,
                    _ => return Err("original VM reclaim fence unavailable".into()),
                }
                json!({"vm":vm,"reclaimed":true,"artifacts_preserved":true})
            }
            _ => return Err("original maintenance intent mismatch".into()),
        };
        let db = self.db();
        let tx = db.unchecked_transaction()?;
        tx.execute(
            "UPDATE image_operations SET state='running' WHERE id=?1 AND state='unknown'",
            [request.to_string()],
        )?;
        crate::image_management::commit_receipt(&tx, request, &result)?;
        tx.execute(
            "UPDATE maintenance_intents SET status='complete' WHERE request=?1",
            [request.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }
}
use std::os::unix::fs::MetadataExt;

#[cfg(test)]
mod tests;
