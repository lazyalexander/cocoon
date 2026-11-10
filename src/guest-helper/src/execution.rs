use cocoon_guest_wire::{MAX_CONTENT, Operation, Output};
use std::{
    fs,
    io::{self, Read, Seek, SeekFrom, Write},
    os::unix::{fs::OpenOptionsExt, process::CommandExt},
    process::{Command, Stdio},
    sync::OnceLock,
    thread,
    time::{Duration, Instant},
};

fn bounded(reader: impl Read) -> io::Result<String> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_CONTENT + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_CONTENT {
        return Err(io::Error::other("output limit"));
    }
    String::from_utf8(bytes).map_err(io::Error::other)
}

static NONCE: OnceLock<String> = OnceLock::new();

pub fn initialize_nonce() -> io::Result<()> {
    let mut bytes = [0_u8; 16];
    // SAFETY: Linux getrandom receives a valid writable buffer and no flags.
    let result = unsafe { libc::getrandom(bytes.as_mut_ptr().cast(), bytes.len(), 0) };
    if result != bytes.len() as isize {
        return Err(io::Error::last_os_error());
    }
    NONCE
        .set(cocoon_guest_wire::encode_bytes(&bytes))
        .map_err(|_| io::Error::other("guest nonce already initialized"))
}

fn nonce() -> io::Result<String> {
    NONCE
        .get()
        .cloned()
        .ok_or_else(|| io::Error::other("guest nonce is unavailable"))
}

pub fn execute(operation: &Operation) -> io::Result<Output> {
    match operation {
        Operation::Status => Ok(Output::Status { nonce: nonce()? }),
        Operation::Write { path, content } => {
            let mut file = fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .mode(0o600)
                .open(path)?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
            Ok(Output::Written)
        }
        Operation::Read { path } => Ok(Output::Content {
            content: bounded(fs::File::open(path)?)?,
        }),
        Operation::WriteBytes {
            path,
            content_hex,
            offset,
            sync,
        } => {
            let bytes = cocoon_guest_wire::decode_bytes(content_hex)?;
            let mut options = fs::OpenOptions::new();
            options.write(true).mode(0o600);
            if *offset == 0 {
                options.create(true).truncate(true);
            }
            let mut file = options.open(path)?;
            file.seek(SeekFrom::Start(*offset))?;
            file.write_all(&bytes)?;
            if *sync {
                file.sync_all()?;
            }
            Ok(Output::Written)
        }
        Operation::ReadChunk {
            path,
            offset,
            max_bytes,
        } => {
            let mut file = fs::File::open(path)?;
            file.seek(SeekFrom::Start(*offset))?;
            let mut bytes = Vec::new();
            file.take((*max_bytes as u64) + 1).read_to_end(&mut bytes)?;
            let eof = bytes.len() <= *max_bytes as usize;
            bytes.truncate(*max_bytes as usize);
            let next_offset = offset
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| io::Error::other("file offset overflow"))?;
            Ok(Output::Chunk {
                content_hex: cocoon_guest_wire::encode_bytes(&bytes),
                next_offset,
                eof,
            })
        }
        Operation::Exec { argv, cwd } => {
            let mut child = Command::new(&argv[0])
                .args(&argv[1..])
                .current_dir(cwd)
                .env_clear()
                .env("PATH", "/bin:/usr/bin")
                .env("LANG", "C")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .process_group(0)
                .spawn()?;
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| io::Error::other("missing stdout"))?;
            let stderr = child
                .stderr
                .take()
                .ok_or_else(|| io::Error::other("missing stderr"))?;
            let out = thread::spawn(move || bounded(stdout));
            let err = thread::spawn(move || bounded(stderr));
            let deadline = Instant::now() + Duration::from_secs(2);
            let (status, timed_out) = loop {
                if let Some(status) = child.try_wait()? {
                    break (status, false);
                }
                if Instant::now() >= deadline {
                    // SAFETY: child was placed in its own process group, positive pid
                    // is negated to target only that guest group. Host deadline is separate.
                    unsafe {
                        libc::kill(-(child.id() as i32), libc::SIGKILL);
                    }
                    break (child.wait()?, true);
                }
                thread::sleep(Duration::from_millis(5));
            };
            // Reap lingering descendants' group even if the direct child exited.
            // SAFETY: group id is the spawned child's pid, never the helper's group.
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let stdout = out
                .join()
                .map_err(|_| io::Error::other("stdout reader"))??;
            let stderr = err
                .join()
                .map_err(|_| io::Error::other("stderr reader"))??;
            Ok(Output::Exited {
                code: status.code(),
                stdout,
                stderr,
                timed_out,
            })
        }
    }
}
