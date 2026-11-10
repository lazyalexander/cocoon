use crate::{
    AttemptId, BootId, ExecutionAuthority, Manager, Result, VmId, VmState,
    disk::{hash, sync_directory},
};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
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
    pub status: String,
}

fn process_alive(pid: u32) -> Result<bool> {
    let status = Command::new("/bin/kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    Ok(status.success())
}

fn runner_exited(manager: &Manager, attempt: AttemptId, pid: u32) -> Result<bool> {
    let mut children = manager
        .children
        .lock()
        .map_err(|_| crate::Error::from("child registry poisoned"))?;
    if let Some(child) = children.get_mut(&attempt) {
        return Ok(child.try_wait()?.is_some());
    }
    Ok(!process_alive(pid)?)
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

impl Manager {
    fn execution(&self, attempt: AttemptId) -> Result<ExecutionRecord> {
        let record: String = self.db.query_row(
            "SELECT record FROM executions WHERE attempt=?1",
            [attempt.to_string()],
            |row| row.get(0),
        )?;
        Ok(serde_json::from_str(&record)?)
    }

    fn save_execution(&self, record: &ExecutionRecord) -> Result<()> {
        self.db.execute(
            "UPDATE executions SET record=?1 WHERE attempt=?2",
            params![serde_json::to_string(record)?, record.attempt.to_string()],
        )?;
        Ok(())
    }

    fn save_vm(&self, vm: &crate::Vm) -> Result<()> {
        self.db.execute(
            "UPDATE vms SET record=?1 WHERE id=?2",
            params![serde_json::to_string(vm)?, vm.id.to_string()],
        )?;
        Ok(())
    }

    pub fn start_with_runner(&self, id: VmId, runner: &Path) -> Result<ExecutionAuthority> {
        if !runner.is_absolute() || !fs::symlink_metadata(runner)?.is_file() {
            return Err("runner must be an absolute regular file".into());
        }
        let mut header = [0_u8; 8];
        fs::File::open(runner)?.read_exact(&mut header)?;
        if header != [0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0, 0, 1] {
            return Err("runner must be a native ARM64 Mach-O executable".into());
        }
        let mut vm = self.get(id)?;
        self.verify_image(vm.image)?;
        let authority = ExecutionAuthority::issue(&mut vm)?;
        vm.state = VmState::Unknown;
        self.append_journal(id, "start_intent", serde_json::json!({"attempt": authority.attempt.id, "generation": authority.generation}))?;
        self.save_vm(&vm)?;
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
        fs::write(
            &request,
            serde_json::to_vec(&serde_json::json!({
                "kernel": image.join("kernel"), "initrd": image.join("initrd"), "token": token,
                "memoryMiB": vm.memory_mib, "seconds": 3600, "rootDisk": disk,
                "diskMode": "agent", "payload": token, "eventLog": events,
                "controlSocket": control,
            }))?,
        )?;
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
            status: "starting".into(),
        };
        self.db.execute(
            "INSERT INTO executions VALUES (?1,?2,?3)",
            params![
                execution.attempt.to_string(),
                id.to_string(),
                serde_json::to_string(&execution)?
            ],
        )?;
        let stdout = fs::File::create_new(run.join("stdout.json"))?;
        let stderr = fs::File::create_new(run.join("stderr.txt"))?;
        let mut child = Command::new(runner)
            .arg(&request)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()?;
        execution.pid = Some(child.id());
        self.save_execution(&execution)?;
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if event(&events, "guest_ready", &token)? {
                break;
            }
            if let Some(status) = child.try_wait()? {
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
        execution.status = "running".into();
        let tx = self.db.unchecked_transaction()?;
        tx.execute(
            "UPDATE vms SET record=?1 WHERE id=?2",
            params![serde_json::to_string(&vm)?, id.to_string()],
        )?;
        tx.execute(
            "UPDATE executions SET record=?1 WHERE attempt=?2",
            params![
                serde_json::to_string(&execution)?,
                execution.attempt.to_string()
            ],
        )?;
        tx.commit()?;
        self.append_journal(
            id,
            "start_complete",
            serde_json::json!({"attempt": execution.attempt, "pid": execution.pid}),
        )?;
        Ok(authority)
    }

    pub fn stop(&self, id: VmId) -> Result<()> {
        let mut vm = self.get(id)?;
        if vm.state != VmState::Running {
            return Err("VM is not running".into());
        }
        let attempt = vm.active_attempt.ok_or("missing active attempt")?;
        let mut execution = self.execution(attempt)?;
        let pid = execution.pid.ok_or("missing runner pid")?;
        vm.state = VmState::Stopping;
        execution.status = "stopping".into();
        self.save_vm(&vm)?;
        self.save_execution(&execution)?;
        self.append_journal(id, "stop_intent", serde_json::json!({"attempt": attempt}))?;
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
            self.save_vm(&vm)?;
            self.save_execution(&execution)?;
            return Err("runner stop delivery failed; termination unknown".into());
        }
        let token = attempt.raw().simple().to_string();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if event(&execution.events, "stopped", &token)? && runner_exited(self, attempt, pid)? {
                vm.state = VmState::Stopped;
                vm.active_lease = None;
                execution.status = "stopped".into();
                self.save_vm(&vm)?;
                self.save_execution(&execution)?;
                self.append_journal(id, "stop_complete", serde_json::json!({"attempt": attempt}))?;
                return Ok(());
            }
            if Instant::now() >= deadline {
                vm.state = VmState::Unknown;
                execution.status = "unknown".into();
                self.save_vm(&vm)?;
                self.save_execution(&execution)?;
                return Err("runner stop outcome is unknown".into());
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn reconcile(&self, id: VmId) -> Result<VmState> {
        let mut vm = self.get(id)?;
        if !matches!(
            vm.state,
            VmState::Starting
                | VmState::Running
                | VmState::Stopping
                | VmState::Suspending
                | VmState::Resuming
        ) {
            return Ok(vm.state);
        }
        let attempt = vm.active_attempt.ok_or("active VM has no attempt")?;
        let mut execution = self.execution(attempt)?;
        let token = attempt.raw().simple().to_string();
        let pid = execution.pid.ok_or("runner spawn outcome unknown")?;
        if event(&execution.events, "stopped", &token)? && runner_exited(self, attempt, pid)? {
            vm.state = VmState::Stopped;
            vm.active_lease = None;
            execution.status = "stopped".into();
            self.save_vm(&vm)?;
            self.save_execution(&execution)?;
        } else if runner_exited(self, attempt, pid)? {
            vm.state = VmState::Unknown;
            execution.status = "unknown".into();
            self.save_vm(&vm)?;
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
        Ok(response)
    }

    pub fn runner_sha256(&self, runner: &Path) -> Result<String> {
        hash(runner)
    }
}
