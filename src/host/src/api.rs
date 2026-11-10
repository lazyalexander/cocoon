use crate::{ImageId, Manager, RequestId, Result, VmId};
use cocoon_guest_wire::{receive, send};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::{
        fs::{OpenOptionsExt, PermissionsExt},
        net::UnixListener,
    },
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostOperation {
    Start {
        vm: VmId,
    },
    Stop {
        vm: VmId,
    },
    Show {
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
    Guest {
        request: cocoon_guest_wire::Request,
    },
    Export {
        request: cocoon_guest_wire::Request,
        guest_path: String,
        name: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostRequest {
    pub request: RequestId,
    pub caller: String,
    pub credential: String,
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
        let result = (|| -> Result<serde_json::Value> {
            let credential = self.api_credential()?;
            if request.caller.is_empty()
                || request.caller.len() > 128
                || request.credential != credential
            {
                return Err("API authentication failed".into());
            }
            match request.operation {
                HostOperation::Start { vm } => {
                    Ok(serde_json::to_value(self.start_with_runner(vm, runner)?)?)
                }
                HostOperation::Stop { vm } => {
                    self.stop(vm)?;
                    Ok(serde_json::Value::Null)
                }
                HostOperation::Show { vm } => Ok(serde_json::to_value(self.get(vm)?)?),
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
                    Ok(serde_json::to_value(self.export_artifact(
                        guest,
                        &guest_path,
                        &name,
                    )?)?)
                }
            }
        })()
        .map_err(|error: crate::Error| error.to_string());
        HostResponse {
            request: request.request,
            caller: request.caller,
            result,
        }
    }

    pub fn call_api(&self, mut request: HostRequest) -> Result<HostResponse> {
        request.credential = self.api_credential()?;
        let mut stream = std::os::unix::net::UnixStream::connect(self.api_socket_path())?;
        send(&request, &mut stream)?;
        Ok(receive(&mut stream)?)
    }

    pub fn serve_api(&self, runner: &Path) -> Result<()> {
        if !runner.is_absolute() {
            return Err("runner must be absolute".into());
        }
        self.api_credential()?;
        let path = self.api_socket_path();
        let listener = UnixListener::bind(&path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        for stream in listener.incoming() {
            let mut stream = stream?;
            let request = receive::<HostRequest>(&mut stream);
            match request {
                Ok(request) => {
                    let response = self.handle_api(request, runner);
                    send(&response, &mut stream)?;
                }
                Err(error) => {
                    let response = HostResponse {
                        request: RequestId::from(Uuid::nil()),
                        caller: "unknown".into(),
                        result: Err(error.to_string()),
                    };
                    send(&response, &mut stream)?;
                }
            }
        }
        Ok(())
    }
}
