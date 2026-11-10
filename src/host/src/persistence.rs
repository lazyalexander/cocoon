//! Explicit experimental root-disk acceptance. Not a general VM execution API.
use crate::{BootId, Manager, Result, VmId, VmState, disk::hash};
use rusqlite::params;
use std::{
    fs,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

impl Manager {
    /// Own a bounded fixed-workload boot; guest output is never termination evidence.
    pub fn persistence_boot(
        &self,
        id: VmId,
        runner: &Path,
        mode: &str,
        payload: &str,
    ) -> Result<serde_json::Value> {
        if !["write", "read"].contains(&mode) {
            return Err("invalid persistence mode".into());
        }
        self.fixture_boot(id, runner, mode, payload)
    }

    /// Explicit fixed code workflow with no external grant or daemon API implied.
    pub fn project_boot(&self, id: VmId, runner: &Path) -> Result<serde_json::Value> {
        self.fixture_boot(id, runner, "project", &Uuid::new_v4().simple().to_string())
    }

    fn fixture_boot(
        &self,
        id: VmId,
        runner: &Path,
        mode: &str,
        payload: &str,
    ) -> Result<serde_json::Value> {
        if payload.len() != 32 || !payload.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("invalid fixture payload".into());
        }
        if !runner.is_absolute() || !fs::symlink_metadata(runner)?.is_file() {
            return Err("runner must be an explicit absolute regular file".into());
        }
        let mut executable_header = [0_u8; 8];
        fs::File::open(runner)?.read_exact(&mut executable_header)?;
        if executable_header != [0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0, 0, 1] {
            return Err("runner must be a native ARM64 Mach-O executable".into());
        }
        let mut vm = self.get(id)?;
        if !matches!(vm.state, VmState::Prepared | VmState::Stopped) {
            return Err("VM needs reconciliation before disk reuse".into());
        }
        self.verify_image(vm.image)?;
        let disk = self.root.join("vms").join(id.to_string()).join("root.raw");
        let lease = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(disk.with_extension("lease"))?;
        lease
            .try_lock()
            .map_err(|_| crate::Error::from("working disk is already owned"))?;
        // Recheck under the file lock, so two callers cannot race a stale state.
        vm = self.get(id)?;
        if !matches!(vm.state, VmState::Prepared | VmState::Stopped) {
            return Err("VM needs reconciliation before disk reuse".into());
        }
        let boot = BootId::from(Uuid::new_v4());
        let token = boot.raw().simple().to_string();
        let run_dir = self
            .root
            .join("vms")
            .join(id.to_string())
            .join(boot.to_string());
        fs::create_dir(&run_dir)?;
        let image = self.root.join("images").join(vm.image.to_string());
        let project_requests = crate::project::requests(&token);
        let mut request = serde_json::json!({"kernel": image.join("kernel"), "initrd":image.join("initrd"),
            "token":token,"memoryMiB":512,"seconds":10,"rootDisk":disk,"diskMode":mode,"payload":payload});
        if mode == "project" {
            for request in &project_requests {
                request.validate()?;
            }
            let path = run_dir.join("project.json");
            fs::write(&path, serde_json::to_vec(&project_requests)?)?;
            fs::File::open(&path)?.sync_all()?;
            request["project"] = serde_json::to_value(path)?;
        }
        let request_file = run_dir.join("request.json");
        fs::write(&request_file, serde_json::to_vec(&request)?)?;
        fs::File::open(&request_file)?.sync_all()?;
        crate::disk::sync_directory(&run_dir)?;
        crate::disk::sync_directory(run_dir.parent().ok_or("invalid run directory")?)?;
        let mut header = [0; 64];
        fs::File::open(image.join("kernel"))?.read_exact(&mut header)?;
        if &header[56..60] != b"ARM\x64" {
            return Err("not a Linux ARM64 kernel".into());
        }
        let tx = self.db.unchecked_transaction()?;
        // Persist uncertainty before spawning; any early return keeps reuse blocked.
        vm.state = VmState::Unknown;
        tx.execute(
            "UPDATE vms SET record=?1 WHERE id=?2",
            params![serde_json::to_string(&vm)?, id.to_string()],
        )?;
        tx.execute(
            "INSERT INTO boots VALUES (?1,?2,'unknown',NULL)",
            params![boot.to_string(), id.to_string()],
        )?;
        tx.commit()?;
        let runner_hash = hash(runner)?;
        let mut child = Command::new(runner)
            .arg(&request_file)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = child.stdout.take().ok_or("missing runner stdout")?;
        let stderr = child.stderr.take().ok_or("missing runner stderr")?;
        // Readers retain at most 256 KiB per stream, then close. The watchdog
        // bounds a runner blocked by malformed or excessive transport output.
        let out = thread::spawn(move || {
            let mut bytes = Vec::new();
            stdout.take(262145).read_to_end(&mut bytes).map(|_| bytes)
        });
        let err = thread::spawn(move || {
            let mut bytes = Vec::new();
            stderr.take(262145).read_to_end(&mut bytes).map(|_| bytes)
        });
        let deadline = Instant::now() + Duration::from_secs(20);
        let (status, timed_out) = loop {
            if let Some(status) = child.try_wait()? {
                break (status, false);
            }
            if Instant::now() >= deadline {
                child.kill()?;
                break (child.wait()?, true);
            }
            thread::sleep(Duration::from_millis(10));
        };
        let stdout = out
            .join()
            .map_err(|_| crate::Error::from("stdout reader failed"))??;
        let stderr = err
            .join()
            .map_err(|_| crate::Error::from("stderr reader failed"))??;
        fs::write(run_dir.join("stderr.txt"), &stderr)?;
        let observed: serde_json::Value = if stdout.len() <= 262144 {
            serde_json::from_slice(&stdout).unwrap_or(serde_json::Value::Null)
        } else {
            serde_json::Value::Null
        };
        let terminated = !timed_out
            && status.success()
            && observed["schema"] == "cocoon-vz-experiment-v1"
            && observed["boot_token"] == token
            && observed["termination_confirmed"] == true
            && observed["status"] == "stopped";
        let serial = observed["guest_serial"]
            .as_str()
            .unwrap_or("")
            .replace("\r\n", "\n");
        let root = serial
            .lines()
            .any(|s| s == format!("COCOON_ROOT:ext4:/dev/vda:{token}"));
        let persisted = serial
            .lines()
            .any(|s| s == format!("COCOON_PERSIST:{mode}:{payload}:{token}"));
        let project_verified = mode == "project"
            && crate::project::verify(&project_requests, &observed["guest_transport"]);
        let passed = terminated
            && root
            && observed["guest_ready"] == true
            && if mode == "project" {
                project_verified && observed["stop_reason"] == "project_complete"
            } else {
                persisted && observed["stop_reason"] == "guest_done"
            };
        // The receiver chooses the name and directory; guest paths never become host paths.
        // Only a fully checked response and confirmed VM stop permit publication.
        let artifact = if passed && mode == "project" {
            use std::{io::Write, os::unix::fs::OpenOptionsExt};
            let path = run_dir.join("artifact.txt");
            let content = observed["guest_transport"][5]["result"]["Ok"]["content"]
                .as_str()
                .ok_or("missing artifact")?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
            crate::disk::sync_directory(&run_dir)?;
            serde_json::json!({"name":"artifact.txt", "bytes":content.len(), "sha256":hash(&path)?, "automatically_executed":false})
        } else {
            serde_json::Value::Null
        };
        vm.state = if terminated {
            VmState::Stopped
        } else {
            VmState::Unknown
        };
        let report = serde_json::json!({"schema":if mode == "project" { "cocoon-project-v1" } else { "cocoon-persistence-v1" },"vm":id,"boot":boot,"mode":mode,
            "runner_sha256":runner_hash,"passed":passed,"termination_confirmed":terminated,
            "project_verified":project_verified,"artifact":artifact,"root_verified":root,"payload_verified":persisted,"timed_out":timed_out,"runner":observed});
        let tx = self.db.unchecked_transaction()?;
        tx.execute(
            "UPDATE vms SET record=?1 WHERE id=?2",
            params![serde_json::to_string(&vm)?, id.to_string()],
        )?;
        tx.execute(
            "UPDATE boots SET status=?1,report=?2 WHERE id=?3",
            params![
                if terminated { "stopped" } else { "unknown" },
                report.to_string(),
                boot.to_string()
            ],
        )?;
        tx.commit()?;
        fs::write(
            run_dir.join("result.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
        Ok(report)
    }
}
