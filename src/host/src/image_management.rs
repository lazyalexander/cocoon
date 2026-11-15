//! Host-authenticated image mutations. Publication and its receipt share one transaction.
use crate::{ImageId, Manager, RequestId, Result, VmId};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, os::unix::fs::OpenOptionsExt, path::PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum ImageAction {
    Retire {
        image: ImageId,
        sha256: String,
    },
    ReclaimVm {
        vm: VmId,
        generation: u64,
        revision: u64,
    },
    ImportCandidate {
        environment: String,
        kernel: PathBuf,
        initrd: PathBuf,
        disk: PathBuf,
        kernel_sha256: String,
        initrd_sha256: String,
        disk_sha256: String,
    },
    SaveCandidate {
        vm: VmId,
        generation: u64,
        disk_sha256: String,
    },
    PromoteCandidate {
        image: ImageId,
        sha256: String,
    },
    Branch {
        image: ImageId,
        sha256: String,
        vm: VmId,
        owner: String,
    },
    Rollback {
        vm: VmId,
        generation: u64,
        disk_sha256: String,
        image: ImageId,
        sha256: String,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageRequest {
    pub request: RequestId,
    pub caller: String,
    pub credential: String,
    pub action: ImageAction,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageQuery {
    pub request: RequestId,
    pub caller: String,
    pub credential: String,
}

pub(crate) fn digest(value: &impl Serialize) -> Result<String> {
    Ok(Sha256::digest(serde_json::to_vec(value)?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
pub(crate) fn commit_receipt(
    tx: &rusqlite::Transaction<'_>,
    id: RequestId,
    result: &Value,
) -> Result<()> {
    if tx.execute(
        "UPDATE image_operations SET state='succeeded',result=?1 WHERE id=?2 AND state='running'",
        params![result.to_string(), id.to_string()],
    )? != 1
    {
        return Err("image operation receipt conflict".into());
    }
    Ok(())
}

impl Manager {
    pub fn initialize_image_api(&self) -> Result<Value> {
        self.api_credential()?;
        Ok(
            json!({"schema":"cocoon-image-api-v1","disk_images":true,"memory_restore_qualified":false}),
        )
    }
    pub(crate) fn image_auth(&self, caller: &str, credential: &str) -> Result<()> {
        if caller.is_empty()
            || caller.len() > 128
            || caller.contains('\0')
            || credential != self.api_credential()?
        {
            return Err("image management authentication failed".into());
        }
        Ok(())
    }
    fn image_lease(&self) -> Result<fs::File> {
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(self.root.join("image-management.lock"))?;
        file.try_lock()
            .map_err(|_| crate::Error::from("another image operation owns the store"))?;
        Ok(file)
    }
    fn image_record(&self, id: RequestId, caller: &str) -> Result<Value> {
        let row:Option<(String,String,String,Option<String>,i64)>=self.db().query_row(
            "SELECT caller,parameter_sha256,state,result,cancel_requested FROM image_operations WHERE id=?1",
            [id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        let (owner, sha, state, result, cancelled) =
            row.ok_or("original image operation not found")?;
        if owner != caller {
            return Err("image operation caller mismatch".into());
        }
        Ok(json!({"request":id,"parameter_sha256":sha,"state":state,
            "result":result.map(|s|serde_json::from_str::<Value>(&s)).transpose()?,"cancel_requested":cancelled!=0}))
    }
    pub fn image_operation_status(&self, query: &ImageQuery) -> Result<Value> {
        self.image_auth(&query.caller, &query.credential)?;
        let mut record = self.image_record(query.request, &query.caller)?;
        if record["state"] == "running"
            && let Ok(_lease) = self.image_lease()
        {
            self.db().execute(
                "UPDATE image_operations SET state='unknown' WHERE id=?1 AND state='running'",
                [query.request.to_string()],
            )?;
            record = self.image_record(query.request, &query.caller)?;
        }
        Ok(record)
    }
    pub fn cancel_image_operation(&self, query: &ImageQuery) -> Result<Value> {
        self.image_auth(&query.caller, &query.credential)?;
        let exists: bool = self.db().query_row(
            "SELECT EXISTS(SELECT 1 FROM image_operations WHERE id=?1)",
            [query.request.to_string()],
            |r| r.get(0),
        )?;
        if !exists {
            // Seal an original ID before admission; a late worker cannot resurrect it.
            let _lease = self.image_lease()?;
            self.db().execute("INSERT OR IGNORE INTO image_operations VALUES (?1,?2,'unadmitted','null','cancelled',?3,1)",params![query.request.to_string(),query.caller,json!({"admitted":false}).to_string()])?;
        }
        self.image_record(query.request, &query.caller)?;
        self.db().execute("UPDATE image_operations SET cancel_requested=1 WHERE id=?1 AND state IN ('running','unknown')",[query.request.to_string()])?;
        self.image_operation_status(query)
    }
    pub(crate) fn check_image_cancel(&self, id: RequestId) -> Result<()> {
        let cancel: i64 = self.db().query_row(
            "SELECT cancel_requested FROM image_operations WHERE id=?1",
            [id.to_string()],
            |r| r.get(0),
        )?;
        if cancel != 0 {
            return Err("image operation cancelled before publication".into());
        }
        Ok(())
    }
    pub fn perform_image_operation(&self, request: &ImageRequest) -> Result<Value> {
        self.image_auth(&request.caller, &request.credential)?;
        let sha = digest(&json!({"caller":request.caller,"action":request.action}))?;
        if let Ok(record) = self.image_record(request.request, &request.caller) {
            if record["parameter_sha256"] != sha {
                return Err("image operation parameters conflict".into());
            }
            return self.image_operation_status(&ImageQuery {
                request: request.request,
                caller: request.caller.clone(),
                credential: request.credential.clone(),
            });
        }
        let _lease = self.image_lease()?;
        let pending: i64 = self.db().query_row(
            "SELECT COUNT(*) FROM image_operations WHERE state IN ('running','unknown')",
            [],
            |r| r.get(0),
        )?;
        if pending != 0 {
            return Err("reconcile the original image operation first".into());
        }
        self.db().execute(
            "INSERT INTO image_operations VALUES (?1,?2,?3,?4,'running',NULL,0)",
            params![
                request.request.to_string(),
                request.caller,
                sha,
                serde_json::to_string(&request.action)?
            ],
        )?;
        let checked = self.validate_image_action(request.request, &request.action);
        if let Err(error) = checked {
            let cancelled =
                self.image_record(request.request, &request.caller)?["cancel_requested"] == true;
            self.db().execute(
                "UPDATE image_operations SET state=?1,result=?2 WHERE id=?3",
                params![
                    if cancelled { "cancelled" } else { "failed" },
                    json!({"error":error.to_string()}).to_string(),
                    request.request.to_string()
                ],
            )?;
            return self.image_record(request.request, &request.caller);
        }
        let result = self.apply_image_action(request.request, &request.action);
        let record = self.image_record(request.request, &request.caller)?;
        if record["state"] == "succeeded" {
            return Ok(record);
        }
        let error = result
            .err()
            .ok_or("image publication lacks a committed receipt")?;
        let state = if record["cancel_requested"] == true
            && self.image_no_publication(request.request, &request.action)?
        {
            "cancelled"
        } else {
            "unknown"
        };
        self.db().execute(
            "UPDATE image_operations SET state=?1,result=?2 WHERE id=?3 AND state='running'",
            params![
                state,
                json!({"error":error.to_string()}).to_string(),
                request.request.to_string()
            ],
        )?;
        self.image_record(request.request, &request.caller)
    }
    pub fn image_content_digest(&self, id: ImageId) -> Result<String> {
        let image = self.image(id)?;
        digest(
            &json!({"kernel":image["files"]["kernel"]["sha256"],"initrd":image["files"]["initrd"]["sha256"],"root.raw":image["files"]["root.raw"]["sha256"]}),
        )
    }
    pub fn image_summary(&self, id: ImageId) -> Result<Value> {
        let image = self.image(id)?;
        Ok(
            json!({"image":id,"sha256":self.image_content_digest(id)?,"approved":image["approved"],"parent":image["parent"],"source_vm":image["source_vm"],"source_generation":image["source_generation"],"source_environment":image["source_environment"],"retired":image["retired"]}),
        )
    }
    pub fn working_disk_summary(&self, id: VmId) -> Result<Value> {
        let _lease = crate::lifecycle::working_disk_lease(self, id)?;
        self.image_source_ready(id)?;
        let vm = self.get(id)?;
        Ok(
            json!({"vm":id,"generation":vm.generation,"disk_sha256":crate::disk::hash(&self.root.join("vms").join(id.to_string()).join("root.raw"))?}),
        )
    }
    fn image_no_publication(&self, request: RequestId, action: &ImageAction) -> Result<bool> {
        // A cancelled copy may leave private staging bytes; it never grants reuse.
        match action {
            ImageAction::Retire { .. } | ImageAction::ReclaimVm { .. } => {
                Ok(!self.maintenance_started(request)?)
            }
            ImageAction::Rollback {
                vm, disk_sha256, ..
            } => {
                let _lease = crate::lifecycle::working_disk_lease(self, *vm)?;
                let _ = self.recover_rollback(*vm);
                if crate::disk::hash(&self.root.join("vms").join(vm.to_string()).join("root.raw"))?
                    != *disk_sha256
                {
                    return Ok(false);
                }
                let db = self.db();
                let tx = db.unchecked_transaction()?;
                tx.execute("DELETE FROM rollback_intents WHERE vm=?1", [vm.to_string()])?;
                tx.execute("UPDATE operations SET status='complete' WHERE id=?1 AND kind='rollback' AND target=?2", params![request.to_string(),vm.to_string()])?;
                tx.commit()?;
                Ok(true)
            }
            _ => Ok(true), // Registration and success receipt are one SQLite commit.
        }
    }
    pub fn reconcile_image_operation(&self, query: &ImageQuery) -> Result<Value> {
        self.image_auth(&query.caller, &query.credential)?;
        let _lease = self.image_lease()?;
        let record = self.image_record(query.request, &query.caller)?;
        if matches!(record["state"].as_str(), Some("running" | "unknown")) {
            let text: String = self.db().query_row(
                "SELECT action FROM image_operations WHERE id=?1",
                [query.request.to_string()],
                |r| r.get(0),
            )?;
            let action: ImageAction = serde_json::from_str(&text)?;
            if self.maintenance_started(query.request)? {
                self.finish_maintenance(query.request, &action)?;
                return self.image_record(query.request, &query.caller);
            }
            let state = if record["cancel_requested"] == true
                && self.image_no_publication(query.request, &action)?
            {
                "cancelled"
            } else {
                "unknown"
            };
            self.db().execute(
                "UPDATE image_operations SET state=?1 WHERE id=?2",
                params![state, query.request.to_string()],
            )?;
        }
        self.image_record(query.request, &query.caller)
    }
}

mod validation;
