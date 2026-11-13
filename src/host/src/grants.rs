//! Work authorization and durable consumer operation identity.
//! Tokens are stored only as SHA-256 digests. Callers see a token once, at issue.
use crate::{Manager, RequestId, Result, VmId};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkAction {
    Inspect,
    Guest,
    Export,
    Stop,
}

#[derive(Debug)]
pub(crate) struct AuthorizedWork {
    pub(crate) id: Uuid,
    pub(crate) vm: VmId,
    pub(crate) max_export_bytes: u64,
}

pub(crate) enum Admission {
    Proceed,
    Replay(std::result::Result<serde_json::Value, String>),
}

struct GrantRow {
    id: String,
    parent: Option<String>,
    subject: String,
    vm: String,
    generation: i64,
    actions: String,
    expires_unix: i64,
    max_export_bytes: i64,
    revoked: i64,
}

fn unix_now() -> Result<i64> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| crate::Error::from("system clock is before the unix epoch"))?
        .as_secs();
    i64::try_from(seconds).map_err(|_| crate::Error::from("clock value exceeds SQLite integer"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sql_u64(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| crate::Error::from("value exceeds SQLite integer"))
}

fn token_digest(token: &str) -> Result<String> {
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("work authorization rejected".into());
    }
    Ok(sha256_hex(token.as_bytes()))
}

impl Manager {
    /// Give one primary caller the facts needed to direct its VM. A subagent
    /// inside the guest inherits guest permissions; it receives no host grant.
    pub(crate) fn agent_context(
        &self,
        vm_id: VmId,
        caller: &str,
        grant: &AuthorizedWork,
    ) -> Result<serde_json::Value> {
        if grant.vm != vm_id {
            return Err("work authorization vm mismatch".into());
        }
        let row = self.grant_by_id(&grant.id.to_string())?;
        if row.subject != caller {
            return Err("work authorization subject mismatch".into());
        }
        let vm = self.get(vm_id)?;
        let actions = parent_actions(&row)?;
        Ok(serde_json::json!({
            "schema": "cocoon-agent-context-v1",
            "vm": vm.id,
            "state": vm.state,
            "attempt": vm.active_attempt,
            "generation": vm.generation,
            "resources": {
                "vcpus": vm.vcpus,
                "memory_mib": vm.memory_mib,
                "network": vm.network,
            },
            "work": {
                "caller": caller,
                "actions": actions,
                "expires_unix": row.expires_unix,
                "max_export_bytes": grant.max_export_bytes,
                "guest_ready": vm.state == crate::VmState::Running
                    && actions.contains(&WorkAction::Guest),
            },
            "agent_model": {
                "host_caller": "primary",
                "guest_subagents": "share_vm_and_guest_permissions",
                "guest_observations_authorize_host": false,
            },
        }))
    }

    pub fn issue_work_grant(
        &self,
        subject: &str,
        vm: VmId,
        actions: &[WorkAction],
        expires_unix: u64,
        max_export_bytes: u64,
        parent_token: &str,
    ) -> Result<serde_json::Value> {
        if subject.is_empty() || subject.len() > 128 || subject.contains('\0') {
            return Err("work authorization subject mismatch".into());
        }
        let mut unique = Vec::new();
        for action in actions {
            if !unique.contains(action) {
                unique.push(*action);
            }
        }
        if unique.is_empty() {
            return Err("work authorization has no actions".into());
        }
        let expires = sql_u64(expires_unix)?;
        let export_limit = sql_u64(max_export_bytes)?;
        if export_limit == 0 {
            return Err("work authorization resource scope is empty".into());
        }
        let now = unix_now()?;
        if expires <= now {
            return Err("work authorization expired".into());
        }
        let current = self.get(vm)?;
        let generation = sql_u64(current.generation)?;
        let parent_id = if parent_token.is_empty() {
            None
        } else {
            let parent = self.grant_by_token(parent_token)?;
            let inherited = parent_actions(&parent)?;
            if parent.vm != vm.to_string()
                || parent.generation != generation
                || expires > parent.expires_unix
                || export_limit > parent.max_export_bytes
                || unique.iter().any(|action| !inherited.contains(action))
            {
                return Err("derived work authorization is wider than its parent".into());
            }
            self.grant_chain(parent.id.clone(), now)?;
            Some(parent.id)
        };
        let id = Uuid::new_v4();
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        self.db().execute(
            "INSERT INTO work_grants VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,0)",
            params![
                id.to_string(),
                sha256_hex(token.as_bytes()),
                parent_id,
                subject,
                vm.to_string(),
                generation,
                serde_json::to_string(&unique)?,
                expires,
                export_limit,
            ],
        )?;
        self.append_journal(
            vm,
            "work_grant_issued",
            serde_json::json!({
                "grant": id.to_string(),
                "subject": subject,
                "generation": current.generation,
                "actions": unique,
                "expires_unix": expires_unix,
                "max_export_bytes": max_export_bytes,
                "parent": parent_id,
            }),
        )?;
        Ok(serde_json::json!({
            "grant": id.to_string(),
            "token": token,
            "expires_unix": expires_unix,
            "generation": current.generation,
        }))
    }

    pub fn revoke_work_grant(&self, id: Uuid) -> Result<()> {
        let row = self.grant_by_id(&id.to_string())?;
        let vm = VmId::from(
            Uuid::parse_str(&row.vm)
                .map_err(|_| crate::Error::from("work authorization rejected"))?,
        );
        if self.db().execute(
            "UPDATE work_grants SET revoked=1 WHERE id=?1 AND revoked=0",
            [id.to_string()],
        )? != 1
        {
            return Err("work authorization revoked".into());
        }
        self.append_journal(
            vm,
            "work_grant_revoked",
            serde_json::json!({ "grant": id.to_string() }),
        )?;
        Ok(())
    }

    pub(crate) fn authorize_work(
        &self,
        token: &str,
        caller: &str,
        vm: VmId,
        action: WorkAction,
    ) -> Result<AuthorizedWork> {
        let row = self.grant_by_token(token)?;
        self.accept_grant(&row, caller, Some(vm), action)
    }

    pub(crate) fn authorize_presented(
        &self,
        token: &str,
        caller: &str,
        action: WorkAction,
    ) -> Result<AuthorizedWork> {
        let row = self.grant_by_token(token)?;
        self.accept_grant(&row, caller, None, action)
    }

    fn accept_grant(
        &self,
        row: &GrantRow,
        caller: &str,
        expected_vm: Option<VmId>,
        action: WorkAction,
    ) -> Result<AuthorizedWork> {
        if caller != row.subject {
            return Err("work authorization subject mismatch".into());
        }
        let vm = VmId::from(
            Uuid::parse_str(&row.vm)
                .map_err(|_| crate::Error::from("work authorization rejected"))?,
        );
        if expected_vm.is_some_and(|expected| expected != vm) {
            return Err("work authorization vm mismatch".into());
        }
        let current = self.get(vm)?;
        if sql_u64(current.generation)? != row.generation {
            return Err("work authorization generation mismatch".into());
        }
        let now = unix_now()?;
        if row.expires_unix <= now {
            return Err("work authorization expired".into());
        }
        if !parent_actions(row)?.contains(&action) {
            return Err("work authorization does not allow this action".into());
        }
        self.grant_chain(row.id.clone(), now)?;
        Ok(AuthorizedWork {
            id: Uuid::parse_str(&row.id)
                .map_err(|_| crate::Error::from("work authorization rejected"))?,
            vm,
            max_export_bytes: u64::try_from(row.max_export_bytes)
                .map_err(|_| crate::Error::from("work authorization rejected"))?,
        })
    }

    fn grant_chain(&self, id: String, now: i64) -> Result<()> {
        let mut current = Some(id);
        for _ in 0..8 {
            let Some(grant_id) = current else {
                return Ok(());
            };
            let row = self.grant_by_id(&grant_id)?;
            if row.revoked != 0 {
                return Err("work authorization revoked".into());
            }
            if row.expires_unix <= now {
                return Err("work authorization expired".into());
            }
            current = row.parent;
        }
        Err("work authorization rejected".into())
    }

    fn grant_by_token(&self, token: &str) -> Result<GrantRow> {
        let digest = token_digest(token)?;
        self.grant_where("token_sha256=?1", &digest)
    }

    fn grant_by_id(&self, id: &str) -> Result<GrantRow> {
        self.grant_where("id=?1", id)
    }

    fn grant_where(&self, clause: &str, key: &str) -> Result<GrantRow> {
        let sql = format!(
            "SELECT id,parent,subject,vm,generation,actions,expires_unix,max_export_bytes,revoked FROM work_grants WHERE {clause}"
        );
        self.db()
            .query_row(&sql, [key], |row| {
                Ok(GrantRow {
                    id: row.get(0)?,
                    parent: row.get(1)?,
                    subject: row.get(2)?,
                    vm: row.get(3)?,
                    generation: row.get(4)?,
                    actions: row.get(5)?,
                    expires_unix: row.get(6)?,
                    max_export_bytes: row.get(7)?,
                    revoked: row.get(8)?,
                })
            })
            .optional()?
            .ok_or_else(|| crate::Error::from("work authorization rejected"))
    }

    pub(crate) fn admit_consumer(
        &self,
        id: RequestId,
        digest: &str,
        vm: Option<VmId>,
        attempt: Option<&str>,
        generation: Option<u64>,
        grant_id: Option<Uuid>,
    ) -> Result<Admission> {
        let generation = generation.map(sql_u64).transpose()?;
        let db = self.db();
        let tx = db.unchecked_transaction()?;
        let existing: Option<(String, String, Option<String>)> = tx
            .query_row(
                "SELECT parameter_sha256,status,result FROM consumer_operations WHERE id=?1",
                [id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((stored, status, result)) = existing {
            if stored != digest {
                return Err("consumer operation parameters conflict".into());
            }
            return Ok(Admission::Replay(replay(&status, result)));
        }
        tx.execute(
            "INSERT INTO consumer_operations VALUES (?1,?2,'intended',NULL,?3,?4,?5,?6)",
            params![
                id.to_string(),
                digest,
                vm.map(|value| value.to_string()),
                attempt,
                generation,
                grant_id.map(|value| value.to_string()),
            ],
        )?;
        tx.commit()?;
        Ok(Admission::Proceed)
    }

    pub(crate) fn finish_consumer(
        &self,
        id: RequestId,
        result: &Result<serde_json::Value>,
    ) -> Result<()> {
        let (status, body) = match result {
            Ok(value) => ("succeeded", value.to_string()),
            Err(error) => ("failed", error.to_string()),
        };
        if self.db().execute(
            "UPDATE consumer_operations SET status=?1, result=?2 WHERE id=?3 AND status='intended'",
            params![status, body, id.to_string()],
        )? != 1
        {
            return Err("consumer operation outcome is unknown".into());
        }
        Ok(())
    }

    pub(crate) fn consumer_operation(&self, id: RequestId) -> Result<serde_json::Value> {
        let row: Option<(Option<String>, String, Option<String>)> = self
            .db()
            .query_row(
                "SELECT vm,status,result FROM consumer_operations WHERE id=?1",
                [id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((vm, status, result)) = row else {
            return Err("consumer operation was not found".into());
        };
        let result = match status.as_str() {
            "succeeded" => serde_json::from_str(&result.unwrap_or_else(|| "null".into()))?,
            "failed" => serde_json::Value::String(
                result.unwrap_or_else(|| "consumer operation failed".into()),
            ),
            _ => serde_json::Value::Null,
        };
        Ok(serde_json::json!({ "vm": vm, "status": status, "result": result }))
    }
}

fn parent_actions(row: &GrantRow) -> Result<Vec<WorkAction>> {
    Ok(serde_json::from_str(&row.actions)?)
}

fn replay(status: &str, result: Option<String>) -> std::result::Result<serde_json::Value, String> {
    match status {
        "succeeded" => serde_json::from_str(&result.unwrap_or_else(|| "null".into()))
            .map_err(|error| error.to_string()),
        "failed" => Err(result.unwrap_or_else(|| "consumer operation failed".into())),
        "intended" => Err("consumer operation is already in progress".into()),
        _ => Err("consumer operation outcome is unknown".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HostOperation;
    use std::fs;

    struct DropDir(std::path::PathBuf);
    impl Drop for DropDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn digest(operation: &HostOperation, grant: Option<Uuid>) -> String {
        let value = serde_json::json!({
            "grant_id": grant.map(|id| id.to_string()),
            "operation": operation,
        });
        sha256_hex(&serde_json::to_vec(&value).unwrap())
    }

    #[test]
    fn interrupted_operation_is_unknown_after_server_claim_and_is_not_replayed() {
        let path = std::env::temp_dir().join(format!("cocoon-op-{}", Uuid::new_v4()));
        let _dir = DropDir(path.clone());
        let id = RequestId::from(Uuid::new_v4());
        let vm = VmId::from(Uuid::new_v4());
        let operation = HostOperation::Stop { vm };
        let digest = digest(&operation, None);
        {
            let manager = Manager::open(&path).unwrap();
            match manager
                .admit_consumer(id, &digest, Some(vm), None, Some(0), None)
                .unwrap()
            {
                Admission::Proceed => {}
                Admission::Replay(_) => panic!("first admission executed"),
            }
        }
        let manager = Manager::open(&path).unwrap();
        assert_eq!(
            manager.consumer_operation(id).unwrap()["status"],
            "intended"
        );
        crate::store::mark_interrupted_operations(&manager.db()).unwrap();
        let status = manager.consumer_operation(id).unwrap();
        assert_eq!(status["status"], "unknown");
        match manager
            .admit_consumer(id, &digest, Some(vm), None, Some(0), None)
            .unwrap()
        {
            Admission::Replay(Err(message)) => {
                assert_eq!(message, "consumer operation outcome is unknown");
            }
            Admission::Replay(Ok(_)) | Admission::Proceed => panic!("operation was executed again"),
        }
    }
}
