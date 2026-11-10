use crate::{Manager, Result, VmId, VmState};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    process::{Command, Stdio},
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtectionPolicy {
    pub enabled: bool,
    pub minimum_free_percent: u8,
    pub emergency_stop: bool,
    pub max_suspend_per_pass: u8,
}

impl Default for ProtectionPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            minimum_free_percent: 20,
            emergency_stop: true,
            max_suspend_per_pass: 1,
        }
    }
}

impl Manager {
    pub fn protection_policy(&self) -> Result<ProtectionPolicy> {
        let path = self.root.join("protection.json");
        if !path.exists() {
            return Ok(ProtectionPolicy::default());
        }
        Ok(serde_json::from_slice(&fs::read(path)?)?)
    }

    pub fn set_protection_policy(&self, policy: ProtectionPolicy) -> Result<()> {
        if !(1..=100).contains(&policy.minimum_free_percent)
            || !(1..=16).contains(&policy.max_suspend_per_pass)
        {
            return Err("invalid protection policy".into());
        }
        let path = self.root.join("protection.json");
        let temporary = self
            .root
            .join(format!(".protection-{}.tmp", uuid::Uuid::new_v4()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(serde_json::to_vec(&policy)?.as_slice())?;
        file.sync_all()?;
        drop(file);
        fs::rename(temporary, &path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        crate::disk::sync_directory(&self.root)?;
        Ok(())
    }

    pub fn set_priority(&self, id: VmId, priority: i32) -> Result<()> {
        if !(-1000..=1000).contains(&priority) {
            return Err("priority outside bounds".into());
        }
        let mut vm = self.get(id)?;
        vm.priority = priority;
        self.save_vm(&mut vm)
    }

    pub fn admission_allowed(&self) -> Result<bool> {
        let policy = self.protection_policy()?;
        Ok(!policy.enabled || current_free_percent()? >= policy.minimum_free_percent)
    }

    pub fn protect(&self) -> Result<serde_json::Value> {
        let policy = self.protection_policy()?;
        if !policy.enabled {
            return Ok(serde_json::json!({"enabled": false, "action": "none"}));
        }
        let free = current_free_percent()?;
        if free >= policy.minimum_free_percent {
            return Ok(
                serde_json::json!({"enabled": true, "free_percent": free, "action": "none"}),
            );
        }
        let mut candidates: Vec<_> = self
            .list()?
            .into_iter()
            .filter(|vm| vm.state == VmState::Running)
            .collect();
        candidates.sort_by_key(|vm| std::cmp::Reverse(vm.priority));
        let mut suspended = Vec::new();
        let mut errors = Vec::new();
        for mut vm in candidates
            .into_iter()
            .take(policy.max_suspend_per_pass as usize)
        {
            match self.suspend(vm.id) {
                Ok(()) => suspended.push(vm.id),
                Err(error) => {
                    errors.push(format!("{}: {error}", vm.id));
                    vm = self.get(vm.id)?;
                    if policy.emergency_stop
                        && !matches!(
                            vm.state,
                            VmState::Prepared
                                | VmState::Stopped
                                | VmState::Suspended
                                | VmState::RecoveryRequired
                        )
                        && let Err(stop_error) = self.emergency_stop(vm.id)
                    {
                        errors.push(format!("{}: emergency stop: {stop_error}", vm.id));
                    }
                }
            }
        }
        Ok(serde_json::json!({"enabled": true, "free_percent": free,
            "action": "backpressure", "suspended": suspended, "errors": errors}))
    }
}

fn current_free_percent() -> Result<u8> {
    let output = Command::new("/usr/bin/memory_pressure")
        .arg("-Q")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err("memory pressure query failed".into());
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|_| crate::Error::from("memory pressure output is not UTF-8"))?;
    let marker = "System-wide memory free percentage: ";
    let line = text
        .lines()
        .find(|line| line.starts_with(marker))
        .ok_or("missing memory pressure value")?;
    let value = line[marker.len()..]
        .trim()
        .trim_end_matches('%')
        .parse::<u8>()
        .map_err(|_| crate::Error::from("invalid memory pressure number"))?;
    if value > 100 {
        return Err("invalid memory pressure value".into());
    }
    Ok(value)
}
