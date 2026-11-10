use serde::{Deserialize, Serialize};
use uuid::Uuid;
macro_rules! identity {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VmState {
    Prepared,
    Starting,
    Running,
    Stopping,
    Stopped,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FinishPolicy {
    Retain,
    Rollback,
    Branch,
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
