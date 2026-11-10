use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;
macro_rules! identity {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);
        impl $name {
            pub fn raw(self) -> Uuid {
                self.0
            }
        }
        impl From<Uuid> for $name {
            fn from(value: Uuid) -> Self {
                Self(value)
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}
identity!(VmId);
identity!(ImageId);
identity!(BootId);
identity!(OperationId);
identity!(AttemptId);
identity!(LeaseId);
identity!(CheckpointId);
identity!(DiskGenerationId);
identity!(RequestId);
identity!(JournalId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VmState {
    Prepared,
    Starting,
    Running,
    Stopping,
    Suspending,
    Suspended,
    Resuming,
    RecoveryRequired,
    Stopped,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionAttempt {
    pub vm: VmId,
    pub id: AttemptId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionAuthority {
    pub attempt: ExecutionAttempt,
    pub generation: u64,
    pub lease: LeaseId,
}

impl ExecutionAuthority {
    pub fn issue(vm: &mut Vm) -> Result<Self, &'static str> {
        if !matches!(
            vm.state,
            VmState::Prepared | VmState::Stopped | VmState::Suspended
        ) {
            return Err("VM is not available for a new execution attempt");
        }
        vm.generation = vm
            .generation
            .checked_add(1)
            .ok_or("VM generation exhausted")?;
        let authority = Self {
            attempt: ExecutionAttempt {
                vm: vm.id,
                id: AttemptId::from(Uuid::new_v4()),
            },
            generation: vm.generation,
            lease: LeaseId::from(Uuid::new_v4()),
        };
        vm.active_attempt = Some(authority.attempt.id);
        vm.active_lease = Some(authority.lease);
        vm.state = VmState::Starting;
        Ok(authority)
    }

    pub fn issue_recovery(vm: &mut Vm) -> Result<Self, &'static str> {
        if vm.state != VmState::RecoveryRequired {
            return Err("cold recovery is not approved for this state");
        }
        vm.state = VmState::Stopped;
        Self::issue(vm)
    }

    pub fn authorize(&self, vm: &Vm) -> Result<(), &'static str> {
        if self.attempt.vm != vm.id
            || vm.active_attempt != Some(self.attempt.id)
            || vm.active_lease != Some(self.lease)
            || vm.generation != self.generation
        {
            return Err("stale or mismatched execution authority");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FinishPolicy {
    Retain,
    Rollback,
    Branch,
}

fn default_priority() -> i32 {
    100
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vm {
    pub id: VmId,
    pub owner: String,
    pub image: ImageId,
    pub policy: FinishPolicy,
    pub state: VmState,
    pub network: bool,
    pub vcpus: u8,
    pub memory_mib: u32,
    #[serde(default = "default_priority")]
    pub priority: i32,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub generation: u64,
    #[serde(default)]
    pub active_attempt: Option<AttemptId>,
    #[serde(default)]
    pub active_lease: Option<LeaseId>,
    #[serde(default)]
    pub suspend_state: Option<PathBuf>,
    #[serde(default)]
    pub disk_copy: Option<DiskCopy>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiskCopyMethod {
    Clone,
    Sparse,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct DiskCopy {
    pub method: DiskCopyMethod,
    pub logical_bytes: u64,
    pub allocated_bytes: u64,
}
