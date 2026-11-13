use crate::{
    AttemptId, BootId, ExecutionAuthority, Manager, Result, VmId, VmState,
    disk::{hash, sync_directory},
};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionRecord {
    pub vm: VmId,
    pub attempt: AttemptId,
    pub boot: BootId,
    pub generation: u64,
    pub lease: String,
    pub pid: Option<u32>,
    pub request: PathBuf,
    pub events: PathBuf,
    #[serde(default)]
    pub control: Option<PathBuf>,
    #[serde(default)]
    pub restore_from: Option<PathBuf>,
    pub status: String,
}

pub(crate) fn working_disk_lease(manager: &Manager, id: VmId) -> Result<fs::File> {
    if manager.reclaim_status(id)?.is_some() {
        return Err("VM is being reclaimed or has been reclaimed".into());
    }
    let path = manager
        .root
        .join("vms")
        .join(id.to_string())
        .join("root.lease");
    let lease = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    lease
        .try_lock()
        .map_err(|_| crate::Error::from("working disk is already owned"))?;
    Ok(lease)
}

fn process_alive(pid: u32, request: &Path) -> Result<bool> {
    let output = Command::new("/bin/ps")
        .arg("-p")
        .arg(pid.to_string())
        .args(["-o", "args="])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Ok(false);
    }
    let args = String::from_utf8(output.stdout)
        .map_err(|_| crate::Error::from("runner process identity is not UTF-8"))?;
    Ok(args.contains(&request.to_string_lossy().into_owned()))
}

fn runner_exited(manager: &Manager, attempt: AttemptId, pid: u32, request: &Path) -> Result<bool> {
    let mut children = manager
        .children
        .lock()
        .map_err(|_| crate::Error::from("child registry poisoned"))?;
    if let Some(child) = children.get_mut(&attempt) {
        return Ok(child.try_wait()?.is_some());
    }
    Ok(!process_alive(pid, request)?)
}

fn event(path: &Path, expected: &str, token: &str) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?.take(65537).read_to_end(&mut bytes)?;
    if bytes.len() > 65536 {
        return Err("runner event log exceeds limit".into());
    }
    for line in std::str::from_utf8(&bytes)
        .map_err(|_| crate::Error::from("invalid runner event UTF-8"))?
        .lines()
    {
        let value: serde_json::Value = serde_json::from_str(line)?;
        if value["event"] == expected && value["token"] == token {
            return Ok(true);
        }
    }
    Ok(false)
}

fn retained_exit_is_recoverable(execution: &ExecutionRecord) -> bool {
    let path = execution
        .request
        .parent()
        .map(|dir| dir.join("stdout.json"));
    let report = path
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    report.is_some_and(|report| {
        report["termination_confirmed"] == true
            || [
                "invalid_request",
                "setup_failed",
                "blocked",
                "start_failed",
                "snapshot_setup_failed",
                "snapshot_restore_failed",
                "snapshot_restore_claimed",
            ]
            .contains(&report["status"].as_str().unwrap_or(""))
    })
}

impl Manager {
    pub(crate) fn execution(&self, attempt: AttemptId) -> Result<ExecutionRecord> {
        let record: String = self.db().query_row(
            "SELECT record FROM executions WHERE attempt=?1",
            [attempt.to_string()],
            |row| row.get(0),
        )?;
        Ok(serde_json::from_str(&record)?)
    }

    fn save_execution(&self, record: &ExecutionRecord) -> Result<()> {
        self.db().execute(
            "UPDATE executions SET record=?1 WHERE attempt=?2",
            params![serde_json::to_string(record)?, record.attempt.to_string()],
        )?;
        Ok(())
    }

    pub(crate) fn save_vm(&self, vm: &mut crate::Vm) -> Result<()> {
        let db = self.db();
        let tx = db.unchecked_transaction()?;
        let current_record: String = tx.query_row(
            "SELECT record FROM vms WHERE id=?1",
            [vm.id.to_string()],
            |row| row.get(0),
        )?;
        let current: crate::Vm = serde_json::from_str(&current_record)?;
        if current.revision != vm.revision {
            return Err("stale VM state update".into());
        }
        vm.revision = vm.revision.checked_add(1).ok_or("VM revision exhausted")?;
        let changed = tx.execute(
            "UPDATE vms SET record=?1 WHERE id=?2 AND record=?3",
            params![
                serde_json::to_string(&*vm)?,
                vm.id.to_string(),
                current_record
            ],
        )?;
        if changed != 1 {
            return Err("VM state changed concurrently".into());
        }
        tx.commit()?;
        Ok(())
    }

    pub fn start_with_runner(&self, id: VmId, runner: &Path) -> Result<ExecutionAuthority> {
        self.launch(id, runner, None, false)
    }

    fn launch(
        &self,
        id: VmId,
        runner: &Path,
        restore: Option<PathBuf>,
        allow_recovery: bool,
    ) -> Result<ExecutionAuthority> {
        if !runner.is_absolute() || !fs::symlink_metadata(runner)?.is_file() {
            return Err("runner must be an absolute regular file".into());
        }
        let mut header = [0_u8; 8];
        fs::File::open(runner)?.read_exact(&mut header)?;
        if header != [0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0, 0, 1] {
            return Err("runner must be a native ARM64 Mach-O executable".into());
        }
        let _lease = working_disk_lease(self, id)?;
        self.recover_rollback(id)?;
        if !self.admission_allowed()? {
            return Err("host protection backpressure rejects new execution".into());
        }
        let mut vm = self.get(id)?;
        if self.image(vm.image)?["approved"] != true {
            return Err("execution requires an approved image".into());
        }
        self.verify_image(vm.image)?;
        let authority = if allow_recovery {
            ExecutionAuthority::issue_recovery(&mut vm)?
        } else {
            ExecutionAuthority::issue(&mut vm)?
        };
        vm.state = VmState::Unknown;
        self.append_journal(id, if allow_recovery { "cold_recovery_intent" } else { "start_intent" }, serde_json::json!({"attempt": authority.attempt.id, "generation": authority.generation}))?;
        self.save_vm(&mut vm)?;
        let run = self
            .root
            .join("vms")
            .join(id.to_string())
            .join(authority.attempt.id.to_string());
        fs::create_dir(&run)?;
        let image = self.root.join("images").join(vm.image.to_string());
        let token = authority.attempt.id.raw().simple().to_string();
        let events = run.join("events.jsonl");
        let control = PathBuf::from(format!("/tmp/cocoon-{token}.sock"));
        let request = run.join("request.json");
        let disk = self.root.join("vms").join(id.to_string()).join("root.raw");
        let state = run.join("state.vz");
        let mut definition = serde_json::json!({
            "kernel": image.join("kernel"), "initrd": image.join("initrd"), "token": token,
            "memoryMiB": vm.memory_mib, "seconds": 3600, "rootDisk": disk,
            "diskMode": "agent", "payload": token, "eventLog": events,
            "controlSocket": control, "snapshotMode": "save", "snapshotPath": state,
        });
        if let Some(source) = restore.as_ref() {
            definition["snapshotMode"] = "restore".into();
            definition["restorePath"] = serde_json::to_value(source)?;
        }
        fs::write(&request, serde_json::to_vec(&definition)?)?;
        fs::File::open(&request)?.sync_all()?;
        sync_directory(&run)?;
        let mut execution = ExecutionRecord {
            vm: id,
            attempt: authority.attempt.id,
            boot: BootId::from(Uuid::new_v4()),
            generation: authority.generation,
            lease: authority.lease.to_string(),
            pid: None,
            request: request.clone(),
            events: events.clone(),
            control: Some(control),
            restore_from: restore.clone(),
            status: if restore.is_some() {
                "resuming".into()
            } else {
                "starting".into()
            },
        };
        self.db().execute(
            "INSERT INTO executions VALUES (?1,?2,?3)",
            params![
                execution.attempt.to_string(),
                id.to_string(),
                serde_json::to_string(&execution)?
            ],
        )?;
        let stdout = fs::File::create_new(run.join("stdout.json"))?;
        let stderr = fs::File::create_new(run.join("stderr.txt"))?;
        let mut child = match Command::new(runner)
            .arg(&request)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                vm.state = VmState::RecoveryRequired;
                vm.active_lease = None;
                execution.status = "spawn_failed".into();
                self.save_vm(&mut vm)?;
                self.save_execution(&execution)?;
                return Err(format!("runner spawn failed: {error}").into());
            }
        };
        execution.pid = Some(child.id());
        self.save_execution(&execution)?;
        let deadline = Instant::now() + Duration::from_secs(20);
        let ready_event = if restore.is_some() {
            "vm_resumed"
        } else {
            "guest_ready"
        };
        loop {
            if event(&events, ready_event, &token)? {
                break;
            }
            if let Some(status) = child.try_wait()? {
                if retained_exit_is_recoverable(&execution) {
                    vm.state = VmState::RecoveryRequired;
                    vm.active_lease = None;
                    execution.status = "recovery_required".into();
                    self.save_vm(&mut vm)?;
                } else {
                    vm.state = VmState::Unknown;
                    execution.status = "unknown".into();
                    if restore.is_some() {
                        self.save_vm(&mut vm)?;
                    }
                }
                self.save_execution(&execution)?;
                return Err(format!("runner exited before guest readiness: {status}").into());
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(
                    "guest readiness deadline expired; execution requires reconciliation".into(),
                );
            }
            thread::sleep(Duration::from_millis(10));
        }
        self.children
            .lock()
            .map_err(|_| crate::Error::from("child registry poisoned"))?
            .insert(authority.attempt.id, child);
        vm.state = VmState::Running;
        if restore.is_some() {
            vm.suspend_state = None;
        }
        execution.status = "running".into();
        self.save_vm(&mut vm)?;
        self.save_execution(&execution)?;
        {
            let challenge = cocoon_guest_wire::Request {
                identity: cocoon_guest_wire::Identity {
                    vm: id.to_string(),
                    caller: "host-resume".into(),
                    task: "resume-challenge".into(),
                    attempt: token.clone(),
                    generation: authority.generation,
                    request: 0,
                },
                operation: cocoon_guest_wire::Operation::Exec {
                    argv: vec!["/bin/busybox".into(), "true".into()],
                    cwd: "/workspace".into(),
                },
            };
            let valid = self
                .guest(challenge)
                .ok()
                .and_then(|response| response.result.ok())
                .is_some_and(|output| {
                    matches!(
                        output,
                        cocoon_guest_wire::Output::Exited { code: Some(0), .. }
                    )
                });
            if !valid {
                vm.state = VmState::Unknown;
                execution.status = "unknown".into();
                self.save_vm(&mut vm)?;
                self.save_execution(&execution)?;
                return Err("restored guest failed the host challenge".into());
            }
        }
        self.append_journal(
            id,
            if restore.is_some() {
                "resume_complete"
            } else {
                "start_complete"
            },
            serde_json::json!({"attempt": execution.attempt, "pid": execution.pid}),
        )?;
        Ok(authority)
    }

    pub fn suspend(&self, id: VmId) -> Result<()> {
        let _lease = working_disk_lease(self, id)?;
        let mut vm = self.get(id)?;
        if vm.state != VmState::Running {
            return Err("VM is not running".into());
        }
        let attempt = vm.active_attempt.ok_or("missing active attempt")?;
        let mut execution = self.execution(attempt)?;
        let pid = execution.pid.ok_or("missing runner pid")?;
        let state = execution
            .request
            .parent()
            .ok_or("invalid execution directory")?
            .join("state.vz");
        vm.state = VmState::Suspending;
        execution.status = "suspending".into();
        self.save_vm(&mut vm)?;
        self.save_execution(&execution)?;
        self.append_journal(
            id,
            "suspend_intent",
            serde_json::json!({"attempt": attempt}),
        )?;
        if !process_alive(pid, &execution.request)? {
            vm.state = VmState::Unknown;
            execution.status = "unknown".into();
            self.save_vm(&mut vm)?;
            self.save_execution(&execution)?;
            return Err("runner identity mismatch; no snapshot signal sent".into());
        }
        let status = Command::new("/bin/kill")
            .arg("-USR1")
            .arg(pid.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if !status.success() {
            vm.state = VmState::Unknown;
            execution.status = "unknown".into();
            self.save_vm(&mut vm)?;
            self.save_execution(&execution)?;
            return Err("snapshot signal delivery failed".into());
        }
        let token = attempt.raw().simple().to_string();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let saved = event(&execution.events, "snapshot_saved", &token)?;
            let stopped = event(&execution.events, "stopped", &token)?
                && runner_exited(self, attempt, pid, &execution.request)?;
            if saved && stopped {
                vm.state = VmState::Suspended;
                vm.active_lease = None;
                vm.suspend_state = Some(state);
                execution.status = "suspended".into();
                self.save_vm(&mut vm)?;
                self.save_execution(&execution)?;
                self.append_journal(
                    id,
                    "suspend_complete",
                    serde_json::json!({"attempt": attempt}),
                )?;
                return Ok(());
            }
            if stopped {
                vm.state = VmState::RecoveryRequired;
                vm.active_lease = None;
                execution.status = "recovery_required".into();
                self.save_vm(&mut vm)?;
                self.save_execution(&execution)?;
                self.append_journal(
                    id,
                    "suspend_failed",
                    serde_json::json!({"attempt": attempt}),
                )?;
                return Err(
                    "snapshot failed after confirmed stop; cold recovery requires approval".into(),
                );
            }
            if Instant::now() >= deadline {
                vm.state = VmState::Unknown;
                execution.status = "unknown".into();
                self.save_vm(&mut vm)?;
                self.save_execution(&execution)?;
                return Err("snapshot outcome is unknown".into());
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn resume_with_runner(&self, id: VmId, runner: &Path) -> Result<ExecutionAuthority> {
        let vm = self.get(id)?;
        if vm.state != VmState::Suspended {
            return Err("VM is not suspended".into());
        }
        let source = vm
            .suspend_state
            .clone()
            .ok_or("missing suspended checkpoint")?;
        self.launch(id, runner, Some(source), false)
    }

    pub fn recover_cold_with_runner(&self, id: VmId, runner: &Path) -> Result<ExecutionAuthority> {
        if self.get(id)?.state != VmState::RecoveryRequired {
            return Err("cold recovery requires an explicit recovery-required state".into());
        }
        self.append_journal(id, "cold_recovery_approved", serde_json::json!({}))?;
        self.launch(id, runner, None, true)
    }

    pub fn stop(&self, id: VmId) -> Result<()> {
        self.stop_execution(id, false)
    }

    pub fn emergency_stop(&self, id: VmId) -> Result<()> {
        self.stop_execution(id, true)
    }

    fn stop_execution(&self, id: VmId, emergency: bool) -> Result<()> {
        let _lease = working_disk_lease(self, id)?;
        let mut vm = self.get(id)?;
        let previous = vm.state;
        if !emergency && vm.state != VmState::Running {
            return Err("VM is not running".into());
        }
        if emergency
            && !matches!(
                previous,
                VmState::Starting
                    | VmState::Running
                    | VmState::Stopping
                    | VmState::Suspending
                    | VmState::Resuming
                    | VmState::Unknown
            )
        {
            return Err("no execution is eligible for emergency stop".into());
        }
        let attempt = vm.active_attempt.ok_or("missing active attempt")?;
        let mut execution = self.execution(attempt)?;
        let pid = execution.pid.ok_or("missing runner pid")?;
        let token = attempt.raw().simple().to_string();
        if event(&execution.events, "stopped", &token)?
            && runner_exited(self, attempt, pid, &execution.request)?
        {
            vm.state = if matches!(
                previous,
                VmState::Running | VmState::Stopping | VmState::Unknown
            ) {
                VmState::Stopped
            } else {
                VmState::RecoveryRequired
            };
            vm.active_lease = None;
            execution.status = "stopped".into();
            self.save_vm(&mut vm)?;
            self.save_execution(&execution)?;
            return Ok(());
        }
        vm.state = VmState::Stopping;
        execution.status = "stopping".into();
        self.save_vm(&mut vm)?;
        self.save_execution(&execution)?;
        self.append_journal(id, "stop_intent", serde_json::json!({"attempt": attempt}))?;
        if !process_alive(pid, &execution.request)? {
            vm.state = VmState::Unknown;
            execution.status = "unknown".into();
            self.save_vm(&mut vm)?;
            self.save_execution(&execution)?;
            return Err("runner identity mismatch; no signal sent".into());
        }
        let status = Command::new("/bin/kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if !status.success() {
            vm.state = VmState::Unknown;
            execution.status = "unknown".into();
            self.save_vm(&mut vm)?;
            self.save_execution(&execution)?;
            return Err("runner stop delivery failed; termination unknown".into());
        }
        let token = attempt.raw().simple().to_string();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if event(&execution.events, "stopped", &token)?
                && runner_exited(self, attempt, pid, &execution.request)?
            {
                vm.state = if matches!(previous, VmState::Running | VmState::Stopping) {
                    VmState::Stopped
                } else {
                    VmState::RecoveryRequired
                };
                vm.active_lease = None;
                execution.status = if vm.state == VmState::Stopped {
                    "stopped".into()
                } else {
                    "recovery_required".into()
                };
                self.save_vm(&mut vm)?;
                self.save_execution(&execution)?;
                self.append_journal(id, "stop_complete", serde_json::json!({"attempt": attempt}))?;
                return Ok(());
            }
            if Instant::now() >= deadline {
                vm.state = VmState::Unknown;
                execution.status = "unknown".into();
                self.save_vm(&mut vm)?;
                self.save_execution(&execution)?;
                return Err("runner stop outcome is unknown".into());
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn reconcile(&self, id: VmId) -> Result<VmState> {
        let _lease = working_disk_lease(self, id)?;
        self.recover_rollback(id)?;
        let mut vm = self.get(id)?;
        if matches!(
            vm.state,
            VmState::Prepared | VmState::Stopped | VmState::Suspended | VmState::RecoveryRequired
        ) {
            return Ok(vm.state);
        }
        let Some(attempt) = vm.active_attempt else {
            return Ok(vm.state);
        };
        let previous = vm.state;
        let mut execution = self.execution(attempt)?;
        let token = attempt.raw().simple().to_string();
        let Some(pid) = execution.pid else {
            if execution.status == "spawn_failed" {
                vm.state = VmState::RecoveryRequired;
                vm.active_lease = None;
                self.save_vm(&mut vm)?;
            }
            return Ok(vm.state);
        };
        let exited = runner_exited(self, attempt, pid, &execution.request)?;
        let stopped = event(&execution.events, "stopped", &token)? && exited;
        if stopped {
            vm.active_lease = None;
            match previous {
                VmState::Suspending => {
                    let saved = event(&execution.events, "snapshot_saved", &token)?;
                    if saved {
                        vm.state = VmState::Suspended;
                        vm.suspend_state =
                            execution.request.parent().map(|dir| dir.join("state.vz"));
                        execution.status = "suspended".into();
                    } else {
                        vm.state = VmState::RecoveryRequired;
                        execution.status = "recovery_required".into();
                    }
                }
                VmState::Stopping | VmState::Running => {
                    vm.state = VmState::Stopped;
                    execution.status = "stopped".into();
                }
                _ => {
                    vm.state = VmState::RecoveryRequired;
                    execution.status = "recovery_required".into();
                }
            }
            self.save_vm(&mut vm)?;
            self.save_execution(&execution)?;
        } else if exited {
            if retained_exit_is_recoverable(&execution) {
                vm.state = VmState::RecoveryRequired;
                vm.active_lease = None;
                execution.status = "recovery_required".into();
            } else {
                vm.state = VmState::Unknown;
                execution.status = "unknown".into();
            }
            self.save_vm(&mut vm)?;
            self.save_execution(&execution)?;
        }
        Ok(vm.state)
    }

    pub fn guest(
        &self,
        request: cocoon_guest_wire::Request,
    ) -> Result<cocoon_guest_wire::Response> {
        request.validate()?;
        let vm_id = VmId::from(
            Uuid::parse_str(&request.identity.vm)
                .map_err(|_| crate::Error::from("invalid VM identity"))?,
        );
        let attempt = AttemptId::from(
            Uuid::parse_str(&request.identity.attempt)
                .map_err(|_| crate::Error::from("invalid attempt identity"))?,
        );
        let vm = self.get(vm_id)?;
        if vm.state != VmState::Running
            || vm.active_attempt != Some(attempt)
            || vm.generation != request.identity.generation
            || request.identity.request != 0
        {
            return Err("stale or unauthorized guest request".into());
        }
        let execution = self.execution(attempt)?;
        let control = execution
            .control
            .ok_or("runner has no guest control channel")?;
        let mut stream = std::os::unix::net::UnixStream::connect(control)?;
        cocoon_guest_wire::send(&request, &mut stream)?;
        let response: cocoon_guest_wire::Response = cocoon_guest_wire::receive(&mut stream)?;
        if response.identity != request.identity {
            return Err("guest response identity mismatch".into());
        }
        let vm = self.get(vm_id)?;
        if vm.state != VmState::Running
            || vm.active_attempt != Some(attempt)
            || vm.generation != request.identity.generation
        {
            return Err("stale or unauthorized guest request".into());
        }
        Ok(response)
    }

    pub fn export_artifact(
        &self,
        request: cocoon_guest_wire::Request,
        guest_path: &str,
        name: &str,
        max_bytes: u64,
    ) -> Result<serde_json::Value> {
        if name.is_empty()
            || name.len() > 128
            || Path::new(name).components().count() != 1
            || !matches!(
                Path::new(name).components().next(),
                Some(std::path::Component::Normal(_))
            )
        {
            return Err("artifact name must be one normal path component".into());
        }
        let vm_id = VmId::from(
            Uuid::parse_str(&request.identity.vm)
                .map_err(|_| crate::Error::from("invalid VM identity"))?,
        );
        let attempt = AttemptId::from(
            Uuid::parse_str(&request.identity.attempt)
                .map_err(|_| crate::Error::from("invalid attempt identity"))?,
        );
        let execution = self.execution(attempt)?;
        let exports = execution
            .request
            .parent()
            .ok_or("invalid execution directory")?
            .join("exports");
        if !exports.exists() {
            fs::DirBuilder::new().mode(0o700).create(&exports)?;
        }
        let destination = exports.join(name);
        if destination.exists() {
            let intended = self.journal()?.into_iter().any(|entry| {
                entry.kind == "artifact_intent"
                    && entry.payload["attempt"] == attempt.to_string()
                    && entry.payload["name"] == name
            });
            if !intended {
                return Err("existing artifact has no matching export intent".into());
            }
            let bytes = fs::metadata(&destination)?.len();
            let sha256 = hash(&destination)?;
            self.append_journal(
                vm_id,
                "artifact_complete",
                serde_json::json!({"attempt": attempt, "name": name, "bytes": bytes}),
            )?;
            return Ok(
                serde_json::json!({"name": name, "bytes": bytes, "sha256": sha256,
                "automatically_executed": false, "reconciled": true}),
            );
        }
        self.append_journal(
            vm_id,
            "artifact_intent",
            serde_json::json!({"attempt": attempt, "name": name}),
        )?;
        let partial = exports.join(format!(".{name}.{}.partial", Uuid::new_v4()));
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&partial)?;
        let result = (|| -> Result<u64> {
            let mut offset = 0_u64;
            loop {
                let mut chunk = request.clone();
                chunk.operation = cocoon_guest_wire::Operation::ReadChunk {
                    path: guest_path.into(),
                    offset,
                    max_bytes: 8192,
                };
                let response = self.guest(chunk)?;
                let response_output = response.result.map_err(crate::Error::from)?;
                let cocoon_guest_wire::Output::Chunk {
                    content_hex,
                    next_offset,
                    eof,
                } = response_output
                else {
                    return Err("guest returned a non-chunk artifact response".into());
                };
                let bytes = cocoon_guest_wire::decode_bytes(&content_hex)?;
                if next_offset != offset + bytes.len() as u64 || (!eof && bytes.is_empty()) {
                    return Err("invalid artifact chunk progression".into());
                }
                if offset + bytes.len() as u64 > max_bytes.min(64 * 1024 * 1024) {
                    return Err("artifact exceeds the authorized export limit".into());
                }
                std::io::Write::write_all(&mut output, &bytes)?;
                offset = next_offset;
                if eof {
                    output.sync_all()?;
                    return Ok(offset);
                }
            }
        })();
        match result {
            Ok(bytes) => {
                drop(output);
                fs::hard_link(&partial, &destination)?;
                fs::remove_file(&partial)?;
                sync_directory(&exports)?;
                self.append_journal(
                    vm_id,
                    "artifact_complete",
                    serde_json::json!({"attempt": attempt, "name": name, "bytes": bytes}),
                )?;
                Ok(
                    serde_json::json!({"name": name, "bytes": bytes, "sha256": hash(&destination)?, "automatically_executed": false}),
                )
            }
            Err(error) => {
                drop(output);
                if let Err(cleanup) = fs::remove_file(partial) {
                    return Err(
                        format!("{error}; partial artifact cleanup failed: {cleanup}").into(),
                    );
                }
                Err(error)
            }
        }
    }

    pub fn runner_sha256(&self, runner: &Path) -> Result<String> {
        hash(runner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_alone_is_not_runner_identity() {
        let mut child = Command::new("/bin/sleep").arg("1").spawn().unwrap();
        assert!(
            !process_alive(child.id(), Path::new("/definitely-not-the-runner-request")).unwrap()
        );
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn stale_manager_records_are_rejected() {
        let root = PathBuf::from(format!("/tmp/cocoon-cas-{}", Uuid::new_v4()));
        let first = Manager::open(&root).unwrap();
        let second = Manager::open(&root).unwrap();
        let input = root.join("input");
        fs::write(&input, b"fixture").unwrap();
        let image = first
            .import_image(&input, &input, &input, "test MIT")
            .unwrap();
        let id = VmId::from(Uuid::new_v4());
        first
            .create(id, image, "owner", crate::FinishPolicy::Retain)
            .unwrap();
        let mut stale = first.get(id).unwrap();
        let mut current = second.get(id).unwrap();
        current.priority = 50;
        second.save_vm(&mut current).unwrap();
        stale.priority = 75;
        assert!(first.save_vm(&mut stale).is_err());
        assert_eq!(first.get(id).unwrap().priority, 50);
        fs::remove_dir_all(root).unwrap();
    }
}
