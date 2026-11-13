use crate::{
    ImageId, Manager, RequestId, Result, VmId,
    grants::{Admission, AuthorizedWork, WorkAction},
};
use cocoon_guest_wire::{receive, send};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::ErrorKind,
    os::unix::{
        fs::{FileTypeExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::sync_channel,
    },
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostOperation {
    Start {
        vm: VmId,
    },
    Stop {
        vm: VmId,
    },
    EmergencyStop {
        vm: VmId,
    },
    Show {
        vm: VmId,
    },
    Context {
        vm: VmId,
    },
    Reconcile {
        vm: VmId,
    },
    Suspend {
        vm: VmId,
    },
    Resume {
        vm: VmId,
    },
    RecoverCold {
        vm: VmId,
    },
    SaveCandidate {
        vm: VmId,
    },
    PromoteCandidate {
        image: ImageId,
    },
    Rollback {
        vm: VmId,
        baseline: ImageId,
    },
    Branch {
        source: ImageId,
        vm: VmId,
        owner: String,
    },
    Protect,
    ProtectionStatus,
    SetProtection {
        policy: crate::ProtectionPolicy,
    },
    SetPriority {
        vm: VmId,
        priority: i32,
    },
    Guest {
        request: cocoon_guest_wire::Request,
    },
    Export {
        request: cocoon_guest_wire::Request,
        guest_path: String,
        name: String,
    },
    IssueGrant {
        subject: String,
        vm: VmId,
        actions: Vec<WorkAction>,
        expires_unix: u64,
        max_export_bytes: u64,
        #[serde(default)]
        parent_token: String,
    },
    RevokeGrant {
        grant: Uuid,
    },
    OperationStatus {
        id: RequestId,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostRequest {
    pub request: RequestId,
    pub caller: String,
    pub credential: String,
    /// Work-grant token. Empty for management operations. Never written to the journal.
    #[serde(default)]
    pub grant: String,
    pub operation: HostOperation,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HostResponse {
    pub request: RequestId,
    pub caller: String,
    pub result: std::result::Result<serde_json::Value, String>,
}

impl Manager {
    pub fn api_socket_path(&self) -> PathBuf {
        use std::os::unix::ffi::OsStrExt;
        let digest: String = Sha256::digest(self.root.as_os_str().as_bytes())
            .into_iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        PathBuf::from(format!("/tmp/cocoon-host-{digest}.sock"))
    }

    fn api_credential(&self) -> Result<String> {
        let path = self.root.join("api-credential");
        if !path.exists() {
            let value = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)?;
            use std::io::Write;
            file.write_all(value.as_bytes())?;
            file.sync_all()?;
        }
        if fs::symlink_metadata(&path)?.permissions().mode() & 0o077 != 0 {
            return Err("API credential must be private".into());
        }
        let value = fs::read_to_string(path)?;
        if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("invalid API credential".into());
        }
        Ok(value)
    }

    pub fn handle_api(&self, request: HostRequest, runner: &Path) -> HostResponse {
        let result = self
            .dispatch(&request, runner)
            .map_err(|error| error.to_string());
        let result = match result {
            Err(message) if !request.grant.is_empty() && message.contains(&request.grant) => {
                Err("work authorization rejected".into())
            }
            other => other,
        };
        HostResponse {
            request: request.request,
            caller: request.caller,
            result,
        }
    }

    fn dispatch(&self, request: &HostRequest, runner: &Path) -> Result<serde_json::Value> {
        if request.caller.is_empty() || request.caller.len() > 128 || request.caller.contains('\0')
        {
            return Err("API authentication failed".into());
        }
        if let HostOperation::OperationStatus { id } = request.operation {
            let grant =
                self.authorize_presented(&request.grant, &request.caller, WorkAction::Inspect)?;
            let record = self.consumer_operation(id)?;
            if record["vm"].as_str() != Some(&grant.vm.to_string()) {
                return Err("work authorization vm mismatch".into());
            }
            return Ok(serde_json::json!({
                "status": record["status"],
                "result": record["result"],
            }));
        }
        let grant = if let Some(action) = work_action(&request.operation) {
            let vm = operation_vm(&request.operation)?;
            Some(self.authorize_work(&request.grant, &request.caller, vm, action)?)
        } else {
            let credential = self.api_credential()?;
            if request.credential != credential {
                return Err("API authentication failed".into());
            }
            None
        };
        if consumer_side_effect(&request.operation) {
            let digest = parameter_digest(&request.operation, grant.as_ref().map(|item| item.id))?;
            let (attempt, generation) = operation_identity(&request.operation);
            match self.admit_consumer(
                request.request,
                &digest,
                operation_vm(&request.operation).ok(),
                attempt.as_deref(),
                generation,
                grant.as_ref().map(|item| item.id),
            )? {
                Admission::Replay(result) => return result.map_err(crate::Error::from),
                Admission::Proceed => {}
            }
            let result = self.execute_authorized(request, runner, grant.as_ref());
            self.finish_consumer(request.request, &result)?;
            return result;
        }
        self.execute_authorized(request, runner, grant.as_ref())
    }

    fn execute_authorized(
        &self,
        request: &HostRequest,
        runner: &Path,
        grant: Option<&AuthorizedWork>,
    ) -> Result<serde_json::Value> {
        match request.operation.clone() {
            HostOperation::Start { vm } => {
                Ok(serde_json::to_value(self.start_with_runner(vm, runner)?)?)
            }
            HostOperation::Stop { vm } => {
                self.stop(vm)?;
                Ok(serde_json::Value::Null)
            }
            HostOperation::EmergencyStop { vm } => {
                self.emergency_stop(vm)?;
                Ok(serde_json::Value::Null)
            }
            HostOperation::Show { vm } => Ok(serde_json::to_value(self.get(vm)?)?),
            HostOperation::Context { vm } => {
                let grant = grant.ok_or("agent context requires a work grant")?;
                self.agent_context(vm, &request.caller, grant)
            }
            HostOperation::Reconcile { vm } => Ok(serde_json::to_value(self.reconcile(vm)?)?),
            HostOperation::Suspend { vm } => {
                self.suspend(vm)?;
                Ok(serde_json::Value::Null)
            }
            HostOperation::Resume { vm } => {
                Ok(serde_json::to_value(self.resume_with_runner(vm, runner)?)?)
            }
            HostOperation::RecoverCold { vm } => Ok(serde_json::to_value(
                self.recover_cold_with_runner(vm, runner)?,
            )?),
            HostOperation::SaveCandidate { vm } => {
                Ok(serde_json::to_value(self.save_candidate(vm)?)?)
            }
            HostOperation::PromoteCandidate { image } => {
                self.promote_candidate(image)?;
                Ok(serde_json::Value::Null)
            }
            HostOperation::Rollback { vm, baseline } => {
                self.rollback(vm, baseline)?;
                Ok(serde_json::Value::Null)
            }
            HostOperation::Branch { source, vm, owner } => {
                Ok(serde_json::to_value(self.branch(source, vm, &owner)?)?)
            }
            HostOperation::Protect => Ok(self.protect()?),
            HostOperation::ProtectionStatus => Ok(serde_json::to_value(self.protection_policy()?)?),
            HostOperation::SetProtection { policy } => {
                self.set_protection_policy(policy)?;
                Ok(serde_json::Value::Null)
            }
            HostOperation::SetPriority { vm, priority } => {
                self.set_priority(vm, priority)?;
                Ok(serde_json::Value::Null)
            }
            HostOperation::Guest { request: mut guest } => {
                guest.identity.caller = request.caller.clone();
                Ok(serde_json::to_value(self.guest(guest)?)?)
            }
            HostOperation::Export {
                request: mut guest,
                guest_path,
                name,
            } => {
                guest.identity.caller = request.caller.clone();
                let limit = grant.map_or(64 * 1024 * 1024, |item| item.max_export_bytes);
                Ok(serde_json::to_value(self.export_artifact(
                    guest,
                    &guest_path,
                    &name,
                    limit,
                )?)?)
            }
            HostOperation::IssueGrant {
                subject,
                vm,
                actions,
                expires_unix,
                max_export_bytes,
                parent_token,
            } => self.issue_work_grant(
                &subject,
                vm,
                &actions,
                expires_unix,
                max_export_bytes,
                &parent_token,
            ),
            HostOperation::RevokeGrant { grant: id } => {
                self.revoke_work_grant(id)?;
                Ok(serde_json::Value::Null)
            }
            HostOperation::OperationStatus { .. } => {
                Err("operation status is resolved before execution".into())
            }
        }
    }

    pub fn call_api(&self, mut request: HostRequest) -> Result<HostResponse> {
        request.credential = self.api_credential()?;
        let mut stream = std::os::unix::net::UnixStream::connect(self.api_socket_path())?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(60)))?;
        stream.set_write_timeout(Some(std::time::Duration::from_secs(60)))?;
        send(&request, &mut stream)?;
        Ok(receive(&mut stream)?)
    }

    pub fn serve_api(&self, runner: &Path) -> Result<()> {
        if !runner.is_absolute() {
            return Err("runner must be absolute".into());
        }
        self.api_credential()?;
        let path = self.api_socket_path();
        if path.exists() {
            if UnixStream::connect(&path).is_ok() {
                return Err("host API server is already running".into());
            }
            if !fs::symlink_metadata(&path)?.file_type().is_socket() {
                return Err("host API socket path is not a socket".into());
            }
            fs::remove_file(&path)?;
        }
        let listener = UnixListener::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        crate::store::mark_interrupted_operations(&self.db())?;
        listener.set_nonblocking(true)?;
        let shutdown = AtomicBool::new(false);
        let inflight = AtomicUsize::new(0);
        let inflight = &inflight;
        let (control_tx, control_rx) = sync_channel::<ConnectionJob>(16);
        let (work_tx, work_rx) = sync_channel::<ConnectionJob>(8);
        let work_rx = std::sync::Mutex::new(work_rx);
        thread::scope(|scope| {
            scope.spawn(|| {
                let mut last = Instant::now() - Duration::from_secs(1);
                while !shutdown.load(Ordering::Relaxed) {
                    if last.elapsed() >= Duration::from_secs(1) {
                        if let Err(error) = self.protect() {
                            eprintln!("cocoon protection: {error}");
                        }
                        last = Instant::now();
                    }
                    thread::sleep(Duration::from_millis(50));
                }
            });
            scope.spawn(move || {
                while let Ok(job) = control_rx.recv() {
                    finish_job(self, runner, job);
                }
            });
            for _ in 0..2 {
                scope.spawn(|| {
                    loop {
                        let job = {
                            let receiver = work_rx
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner());
                            receiver.recv()
                        };
                        let Ok(job) = job else { break };
                        self.wait_work_hold();
                        finish_job(self, runner, job);
                    }
                });
            }
            let accept_result = (|| -> Result<()> {
                loop {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let active = inflight.fetch_add(1, Ordering::Relaxed);
                            if active >= 16 {
                                inflight.fetch_sub(1, Ordering::Relaxed);
                                let _ = stream.shutdown(std::net::Shutdown::Both);
                                continue;
                            }
                            let control_tx = control_tx.clone();
                            let work_tx = work_tx.clone();
                            scope.spawn(move || {
                                struct Guard<'a>(&'a AtomicUsize);
                                impl Drop for Guard<'_> {
                                    fn drop(&mut self) {
                                        self.0.fetch_sub(1, Ordering::Relaxed);
                                    }
                                }
                                let _guard = Guard(inflight);
                                stream.set_nonblocking(false).ok();
                                stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
                                stream.set_write_timeout(Some(Duration::from_secs(60))).ok();
                                let request = receive::<HostRequest>(&mut stream)
                                    .map_err(|error| error.to_string());
                                let work = request
                                    .as_ref()
                                    .is_ok_and(|item| is_work_lane(&item.operation));
                                let job = ConnectionJob { stream, request };
                                let send_result = if work {
                                    work_tx.try_send(job)
                                } else {
                                    control_tx.try_send(job)
                                };
                                if let Err(error) = send_result {
                                    let mut job = match error {
                                        std::sync::mpsc::TrySendError::Full(job)
                                        | std::sync::mpsc::TrySendError::Disconnected(job) => job,
                                    };
                                    let response = HostResponse {
                                        request: RequestId::from(Uuid::nil()),
                                        caller: "unknown".into(),
                                        result: Err("host API queue is full".into()),
                                    };
                                    let _ = send(&response, &mut job.stream);
                                }
                            });
                        }
                        Err(error) if error.kind() == ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(20));
                        }
                        Err(error) if error.kind() == ErrorKind::Interrupted => {}
                        Err(error) => return Err(error.into()),
                    }
                }
            })();
            shutdown.store(true, Ordering::Relaxed);
            drop(control_tx);
            drop(work_tx);
            accept_result
        })
    }
}

struct ConnectionJob {
    stream: UnixStream,
    request: std::result::Result<HostRequest, String>,
}

fn finish_job(manager: &Manager, runner: &Path, mut job: ConnectionJob) {
    let response = match job.request {
        Ok(request) => manager.handle_api(request, runner),
        Err(error) => HostResponse {
            request: RequestId::from(Uuid::nil()),
            caller: "unknown".into(),
            result: Err(error),
        },
    };
    if let Err(error) = send(&response, &mut job.stream) {
        eprintln!("cocoon API client disconnected: {error}");
    }
}

fn is_work_lane(operation: &HostOperation) -> bool {
    matches!(
        operation,
        HostOperation::Guest { .. }
            | HostOperation::Export { .. }
            | HostOperation::Start { .. }
            | HostOperation::Resume { .. }
            | HostOperation::RecoverCold { .. }
            | HostOperation::Suspend { .. }
    )
}

fn work_action(operation: &HostOperation) -> Option<WorkAction> {
    match operation {
        HostOperation::Show { .. }
        | HostOperation::Context { .. }
        | HostOperation::OperationStatus { .. } => Some(WorkAction::Inspect),
        HostOperation::Guest { .. } => Some(WorkAction::Guest),
        HostOperation::Export { .. } => Some(WorkAction::Export),
        HostOperation::Stop { .. } => Some(WorkAction::Stop),
        _ => None,
    }
}

fn consumer_side_effect(operation: &HostOperation) -> bool {
    matches!(
        operation,
        HostOperation::Start { .. }
            | HostOperation::Stop { .. }
            | HostOperation::Guest { .. }
            | HostOperation::Export { .. }
    )
}

fn operation_vm(operation: &HostOperation) -> Result<VmId> {
    match operation {
        HostOperation::Show { vm }
        | HostOperation::Context { vm }
        | HostOperation::Stop { vm }
        | HostOperation::Start { vm }
        | HostOperation::EmergencyStop { vm }
        | HostOperation::Suspend { vm }
        | HostOperation::Resume { vm }
        | HostOperation::RecoverCold { vm } => Ok(*vm),
        HostOperation::Guest { request } | HostOperation::Export { request, .. } => {
            Uuid::parse_str(&request.identity.vm)
                .map(VmId::from)
                .map_err(|_| crate::Error::from("invalid VM identity"))
        }
        _ => Err("operation has no VM".into()),
    }
}

fn operation_identity(operation: &HostOperation) -> (Option<String>, Option<u64>) {
    match operation {
        HostOperation::Guest { request } | HostOperation::Export { request, .. } => (
            Some(request.identity.attempt.clone()),
            Some(request.identity.generation),
        ),
        _ => (None, None),
    }
}

fn parameter_digest(operation: &HostOperation, grant_id: Option<Uuid>) -> Result<String> {
    let bytes = serde_json::to_vec(&serde_json::json!({
        "grant_id": grant_id.map(|id| id.to_string()),
        "operation": operation,
    }))?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(test)]
mod lane_tests {
    use super::*;
    use crate::{FinishPolicy, VmId, VmState, WorkHold};
    use std::{
        fs,
        sync::{Arc, Condvar, Mutex},
        time::Instant,
    };
    use uuid::Uuid;

    #[test]
    fn stop_is_not_queued_behind_a_blocked_work_lane() {
        let root = std::env::temp_dir().join(format!("cocoon-lane-{}", Uuid::new_v4()));
        let manager = Manager::open(&root).unwrap();
        let image_input = root.join("fixture");
        fs::write(&image_input, b"fixture").unwrap();
        let image = manager
            .import_image(&image_input, &image_input, &image_input, "test MIT")
            .unwrap();
        let stopped = VmId::from(Uuid::new_v4());
        let other = VmId::from(Uuid::new_v4());
        manager
            .create(stopped, image, "owner", FinishPolicy::Retain)
            .unwrap();
        manager
            .create(other, image, "owner", FinishPolicy::Retain)
            .unwrap();
        let expires = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 600;
        let token = manager
            .issue_work_grant("agent", stopped, &[WorkAction::Stop], expires, 1024, "")
            .unwrap()["token"]
            .as_str()
            .unwrap()
            .to_string();
        let hold = Arc::new(WorkHold {
            entered: Mutex::new(false),
            entered_cv: Condvar::new(),
            release: Mutex::new(false),
            release_cv: Condvar::new(),
        });
        *manager
            .work_hold
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::clone(&hold));
        let socket = manager.api_socket_path();
        thread::spawn(move || {
            let _ = manager.serve_api(Path::new("/unused"));
        });
        for _ in 0..100 {
            if socket.exists() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let mut guest = UnixStream::connect(&socket).unwrap();
        send(
            &HostRequest {
                request: RequestId::from(Uuid::new_v4()),
                caller: "agent".into(),
                credential: String::new(),
                grant: String::new(),
                operation: HostOperation::Guest {
                    request: cocoon_guest_wire::Request {
                        identity: cocoon_guest_wire::Identity {
                            vm: other.to_string(),
                            caller: "agent".into(),
                            task: "task".into(),
                            attempt: Uuid::new_v4().simple().to_string(),
                            generation: 0,
                            request: 0,
                        },
                        operation: cocoon_guest_wire::Operation::Status,
                    },
                },
            },
            &mut guest,
        )
        .unwrap();
        let mut entered = hold
            .entered
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let started = Instant::now();
        while !*entered {
            let (guard, timeout) = hold
                .entered_cv
                .wait_timeout(entered, Duration::from_secs(2))
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            entered = guard;
            if timeout.timed_out() && !*entered {
                panic!("work lane did not enter the hold");
            }
        }
        drop(entered);
        let mut stop = UnixStream::connect(&socket).unwrap();
        let stop_started = Instant::now();
        send(
            &HostRequest {
                request: RequestId::from(Uuid::new_v4()),
                caller: "agent".into(),
                credential: String::new(),
                grant: token,
                operation: HostOperation::Stop { vm: stopped },
            },
            &mut stop,
        )
        .unwrap();
        let response: HostResponse = receive(&mut stop).unwrap();
        let elapsed = stop_started.elapsed();
        assert!(elapsed < Duration::from_secs(3), "stop took {elapsed:?}");
        assert!(response.result.is_err(), "{:?}", response.result);
        let reopened = Manager::open(&root).unwrap();
        assert_eq!(reopened.get(other).unwrap().state, VmState::Prepared);
        assert_eq!(reopened.get(stopped).unwrap().state, VmState::Prepared);
        {
            let mut release = hold
                .release
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *release = true;
            hold.release_cv.notify_all();
        }
        let _ = (started, guest, root);
    }
}
