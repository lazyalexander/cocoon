//! Offline Linux guest profile. Validation is neither admission nor execution.
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpoolTask {
    pub schema: String,
    pub caller: crate::Caller,
    pub release_sha256: String,
    pub kernel_sha256: String,
    /// Immutable guest filesystem containing the selected Linux workload.
    pub initramfs_sha256: String,
    pub platform: String,
    pub argv: Vec<String>,
    pub limits: crate::Limits,
    pub network: bool,
}

impl SpoolTask {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != "cocoon-spool-task-v1" || self.platform != "linux-arm64" {
            return Err("unsupported spool execution profile");
        }
        for digest in [
            &self.release_sha256,
            &self.kernel_sha256,
            &self.initramfs_sha256,
        ] {
            if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("invalid pinned content digest");
            }
        }
        // Reuse the same resource and request bounds as the original profile.
        let task = crate::Task {
            schema: "cocoon-task-v1".into(),
            caller: self.caller.clone(),
            image: format!("spool@sha256:{}", self.release_sha256),
            argv: self.argv.clone(),
            limits: self.limits.clone(),
            network: self.network,
        };
        task.validate()?;
        if !self.argv[0].starts_with('/') || self.argv[0].split('/').any(|part| part == "..") {
            return Err("guest executable must be an absolute non-traversing path");
        }
        Ok(())
    }
}
