//! Proposed execution contracts. Validation never grants execution authority.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Caller {
    /// Attribution supplied by the client; authentication belongs to admission.
    pub subject: String,
    pub kind: CallerKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallerKind {
    Agent,
    Human,
    Service,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub vcpus: u8,
    pub memory_mib: u32,
    pub writable_mib: u32,
    pub wall_seconds: u32,
    pub output_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub schema: String,
    pub caller: Caller,
    /// A pinned Linux ARM64 OCI image identity, never a mutable tag.
    pub image: String,
    /// Executable and arguments as separate values; no implicit host shell.
    pub argv: Vec<String>,
    pub limits: Limits,
    /// First profile supports no guest network.
    pub network: bool,
}

impl Task {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != "cocoon-task-v1" {
            return Err("unsupported task schema");
        }
        if self.caller.subject.is_empty() || self.caller.subject.len() > 256 {
            return Err("caller subject must contain 1..256 bytes");
        }
        let Some((repository, digest)) = self.image.rsplit_once("@sha256:") else {
            return Err("image must be pinned by sha256 digest");
        };
        if repository.is_empty()
            || self.image.len() > 1024
            || self.image.chars().any(char::is_whitespace)
            || digest.len() != 64
            || !digest.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("invalid image identity");
        }
        if self.argv.is_empty()
            || self.argv.len() > 128
            || self.argv[0].is_empty()
            || self.argv.iter().any(|s| s.len() > 8192 || s.contains('\0'))
        {
            return Err("invalid argv");
        }
        let l = &self.limits;
        if !(1..=8).contains(&l.vcpus)
            || !(256..=8192).contains(&l.memory_mib)
            || !(1..=8192).contains(&l.writable_mib)
            || !(1..=3600).contains(&l.wall_seconds)
            || !(1..=64 * 1024 * 1024).contains(&l.output_bytes)
        {
            return Err("limits outside initial profile bounds");
        }
        if self.network {
            return Err("network unsupported by initial profile");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Prepared,
    Starting,
    Running,
    StopRequested,
    Unconfirmed,
    Stopped,
}

/// An in-memory lifecycle model, not a durable execution supervisor.
/// It intentionally has no Deserialize implementation for state adoption.
#[derive(Debug, Serialize)]
pub struct Attempt {
    id: Uuid,
    task_id: Uuid,
    state: State,
}

impl Attempt {
    pub fn prepare(task_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            task_id,
            state: State::Prepared,
        }
    }

    pub fn id(&self) -> Uuid {
        self.id
    }
    pub fn state(&self) -> State {
        self.state
    }

    /// Call only after host admission. This models intent, not VM creation.
    pub fn starting(&mut self) -> Result<(), &'static str> {
        self.transition(&[State::Prepared], State::Starting)
    }

    /// Only the future trusted backend adapter may supply observations.
    pub fn running(&mut self) -> Result<(), &'static str> {
        self.transition(&[State::Starting], State::Running)
    }

    pub fn request_stop(&mut self) -> Result<(), &'static str> {
        self.transition(
            &[State::Starting, State::Running, State::Unconfirmed],
            State::StopRequested,
        )
    }

    pub fn owner_lost(&mut self) -> Result<(), &'static str> {
        self.transition(
            &[State::Starting, State::Running, State::StopRequested],
            State::Unconfirmed,
        )
    }

    /// The adapter must independently confirm the owned VM stopped; a guest
    /// exit, transport loss or stop request is insufficient. No wire API yet.
    pub fn confirm_stopped(&mut self, owned_attempt: Uuid) -> Result<(), &'static str> {
        if owned_attempt != self.id {
            return Err("termination belongs to another attempt");
        }
        self.transition(
            &[
                State::Starting,
                State::Running,
                State::StopRequested,
                State::Unconfirmed,
            ],
            State::Stopped,
        )
    }

    pub fn may_settle(&self) -> bool {
        self.state == State::Stopped
    }

    fn transition(&mut self, from: &[State], to: State) -> Result<(), &'static str> {
        if !from.contains(&self.state) {
            return Err("invalid attempt transition");
        }
        self.state = to;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
