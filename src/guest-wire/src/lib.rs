//! Bounded experimental guest messages. Correlation is not authorization.
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

pub const MAX_FRAME: usize = 32768;
pub const MAX_CONTENT: usize = 8192;
pub const PORT: u32 = 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    #[serde(default)]
    pub vm: String,
    pub caller: String,
    pub task: String,
    pub attempt: String,
    #[serde(default)]
    pub generation: u64,
    pub request: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub identity: Identity,
    pub operation: Operation,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Write {
        path: String,
        content: String,
    },
    Read {
        path: String,
    },
    WriteBytes {
        path: String,
        content_hex: String,
        offset: u64,
        sync: bool,
    },
    ReadChunk {
        path: String,
        offset: u64,
        max_bytes: u32,
    },
    Exec {
        argv: Vec<String>,
        cwd: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub identity: Identity,
    pub result: std::result::Result<Output, String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Output {
    Written,
    Content {
        content: String,
    },
    Chunk {
        content_hex: String,
        next_offset: u64,
        eof: bool,
    },
    Exited {
        code: Option<i32>,
        stdout: String,
        stderr: String,
        timed_out: bool,
    },
}

pub fn encode_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn decode_bytes(value: &str) -> io::Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(io::Error::other("invalid hexadecimal content"));
    }
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let text = std::str::from_utf8(pair).map_err(io::Error::other)?;
            u8::from_str_radix(text, 16).map_err(io::Error::other)
        })
        .collect()
}

impl Request {
    pub fn validate(&self) -> io::Result<()> {
        for value in [
            &self.identity.caller,
            &self.identity.task,
            &self.identity.attempt,
        ] {
            if value.is_empty() || value.len() > 128 || value.contains('\0') {
                return Err(io::Error::other("invalid correlation identity"));
            }
        }
        let path = match &self.operation {
            Operation::Write { path, content } => {
                if content.len() > MAX_CONTENT {
                    return Err(io::Error::other("content limit"));
                }
                path
            }
            Operation::Read { path } => path,
            Operation::WriteBytes {
                path, content_hex, ..
            } => {
                if decode_bytes(content_hex)?.len() > MAX_CONTENT {
                    return Err(io::Error::other("binary content limit"));
                }
                path
            }
            Operation::ReadChunk {
                path, max_bytes, ..
            } => {
                if !(1..=MAX_CONTENT as u32).contains(max_bytes) {
                    return Err(io::Error::other("invalid chunk size"));
                }
                path
            }
            Operation::Exec { argv, cwd } => {
                if argv.is_empty()
                    || argv.len() > 32
                    || !argv[0].starts_with('/')
                    || argv
                        .iter()
                        .any(|s| s.len() > MAX_CONTENT || s.contains('\0'))
                {
                    return Err(io::Error::other("invalid argv"));
                }
                cwd
            }
        };
        if !path.starts_with('/') || path.len() > 4096 || path.contains('\0') {
            return Err(io::Error::other("guest path must be absolute"));
        }
        Ok(())
    }
}

/// Length-prefixed JSON avoids delimiter ambiguity and allocates only after checking size.
pub fn receive<T: serde::de::DeserializeOwned>(reader: &mut impl Read) -> io::Result<T> {
    let mut header = [0; 4];
    reader.read_exact(&mut header)?;
    let size = u32::from_be_bytes(header) as usize;
    if size == 0 || size > MAX_FRAME {
        return Err(io::Error::other("frame limit"));
    }
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

pub fn send(value: &impl Serialize, writer: &mut impl Write) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    if bytes.len() > MAX_FRAME {
        return Err(io::Error::other("frame limit"));
    }
    writer.write_all(&(bytes.len() as u32).to_be_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oversized_and_truncated_frames_fail() {
        assert!(receive::<Request>(&mut &(u32::MAX.to_be_bytes())[..]).is_err());
        assert!(receive::<Request>(&mut &[0, 0, 0, 4, b'{'][..]).is_err());
    }
    #[test]
    fn host_lifecycle_operations_and_relative_commands_are_rejected() {
        let identity = Identity {
            vm: "vm".into(),
            caller: "agent".into(),
            task: "task".into(),
            attempt: "attempt".into(),
            generation: 0,
            request: 0,
        };
        let value = serde_json::json!({"identity": identity, "operation": {"kind":"destroy", "vm":"victim"}});
        assert!(serde_json::from_value::<Request>(value).is_err());
        let request = Request {
            identity,
            operation: Operation::Exec {
                argv: vec!["sh".into()],
                cwd: "/workspace".into(),
            },
        };
        assert!(request.validate().is_err());
    }

    #[test]
    fn correlation_roundtrip_and_strict_schema() {
        let request = Request {
            identity: Identity {
                vm: "vm".into(),
                caller: "agent".into(),
                task: "task".into(),
                attempt: "attempt".into(),
                generation: 0,
                request: 1,
            },
            operation: Operation::Read {
                path: "/workspace/a".into(),
            },
        };
        request.validate().unwrap();
        let mut bytes = Vec::new();
        send(&request, &mut bytes).unwrap();
        assert_eq!(
            receive::<Request>(&mut bytes.as_slice()).unwrap().identity,
            request.identity
        );
        let mut value = serde_json::to_value(request).unwrap();
        value["grant"] = true.into();
        assert!(serde_json::from_value::<Request>(value).is_err());
    }

    #[test]
    fn binary_chunks_are_exact_and_strict() {
        let bytes = [0, 255, 1, 128];
        assert_eq!(encode_bytes(&bytes), "00ff0180");
        assert_eq!(decode_bytes("00ff0180").unwrap(), bytes);
        for invalid in ["0", "gg", "00\n"] {
            assert!(decode_bytes(invalid).is_err());
        }
    }
}
