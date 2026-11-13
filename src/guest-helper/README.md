# guest-helper

English (authoritative) · [简体中文](README.zh-CN.md)

Linux guest operations. Runs inside the Linux ARM64 VM. Serves vsock file and process operations under guest permissions; it has no host registration, grant or reclaim verbs.

Workspace dependencies: guest-wire, libc. See [architecture](../../docs/architecture.md) and [contribution rules](../../CONTRIBUTING.md).

## Boundary

The guest can modify its own userspace, including this helper. Host registration, ownership, termination and artifact publication must remain outside the guest.
