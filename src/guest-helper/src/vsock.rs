use cocoon_guest_wire::{PORT, Request, Response, receive, send};
use std::{
    fs::File,
    io,
    os::fd::{AsRawFd, FromRawFd},
};

pub fn serve() -> io::Result<()> {
    // SAFETY: socket has no pointer arguments; successful fresh fd is owned below.
    let fd = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: this is a newly created, uniquely owned descriptor.
    let listener = unsafe { File::from_raw_fd(fd) };
    // SAFETY: zero is valid for all fields and padding of sockaddr_vm.
    let mut address: libc::sockaddr_vm = unsafe { std::mem::zeroed() };
    address.svm_family = libc::AF_VSOCK as _;
    address.svm_cid = libc::VMADDR_CID_ANY;
    address.svm_port = PORT;
    // SAFETY: address has the indicated size and remains alive for bind.
    if unsafe {
        libc::bind(
            listener.as_raw_fd(),
            (&address as *const libc::sockaddr_vm).cast(),
            std::mem::size_of_val(&address) as _,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: descriptor is a valid socket and backlog is bounded.
    if unsafe { libc::listen(listener.as_raw_fd(), 1) } != 0 {
        return Err(io::Error::last_os_error());
    }
    println!("COCOON_HELPER_READY");
    // One connection per boot; disconnect never replays commands.
    // SAFETY: null address arguments request no peer-address output.
    let fd = unsafe {
        libc::accept4(
            listener.as_raw_fd(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            libc::SOCK_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: accept returned a new uniquely owned fd.
    let mut stream = unsafe { File::from_raw_fd(fd) };
    let mut previous: Option<(String, String, String)> = None;
    for sequence in 0..16 {
        let request: Request = receive(&mut stream)?;
        request.validate()?;
        if request.identity.request != sequence {
            return Err(io::Error::other("out-of-order request; no replay"));
        }
        let identity = (
            request.identity.caller.clone(),
            request.identity.task.clone(),
            request.identity.attempt.clone(),
        );
        if let Some(ref expected) = previous
            && expected != &identity
        {
            return Err(io::Error::other("identity changed"));
        }
        let result = crate::execution::execute(&request.operation).map_err(|e| e.to_string());
        let failed = result.is_err();
        send(
            &Response {
                identity: request.identity.clone(),
                result,
            },
            &mut stream,
        )?;
        // Own identity values rather than retaining request references.
        previous = Some(identity);
        if failed {
            break;
        }
    }
    Ok(())
}
