# Native agent microVM implementation plan

English (authoritative) · [简体中文](implementation-plan.zh-CN.md)

## Product boundary

Cocoon targets headless Linux ARM64 guests on Apple Silicon using Apple's
Virtualization.framework. Guest root may modify or break the entire guest.
Registration, ownership, grants, leases, recovery and destruction remain trusted
host responsibilities. Guest reports never authorize these actions.

Agents receive controlled tool access; arbitrary host code under the manager's
macOS user is outside the isolation boundary. No GPU, display, audio, USB,
physical passthrough or shared host folders are configured. Networking is off by
default; explicit NAT authorization permits potentially reaching local services.
Revocation must stop the VM before reporting completion. No native fallback.

## Architecture and delivery gates

1. **M0 — contracts:** separate VM identity, boot identity, disk identity, grants,
   tasks and attempts. Keep English and Chinese documentation synchronized.
2. **M1 — real VM:** qualify compatible Linux boot, roundtrip, deadline and cancel;
   then add a minimal Debian ARM64 glibc root filesystem and writable disk.
3. **M2 — host lifecycle:** a Rust manager with SQLite metadata, a private Unix
   socket, CLI and one Swift runner per boot. Persist operation intent before
   side effects. Reconcile uncertainty; never replay unknown execution.
4. **M3 — agent access:** a compiled guest helper over vsock for bounded command
   output and file transfer. VM-scoped revocable credentials stay outside guests.
   Killing the helper must not prevent host termination.
5. **M4 — qualification:** crash/restart, disk-full, flooding, lost ownership,
   revocation and image publication failure tests. At least 30 independent boots;
   report all failures and distinguish cold from repeated startup measurements.

Immutable images contain pinned kernel, initramfs and root disk with provenance.
Independent writable disks use APFS cloning where available, otherwise an
explicit full-copy path. Default to retain; configured rollback recreates from
baseline; branch publishes a new untrusted derived image only after confirmed
stop. Task completion is a host action, not a guest marker. No live snapshots.
Defaults: 1 vCPU, 512 MiB RAM, 8 GiB sparse disk, subject to real acceptance.
Count framework helper memory when measuring total overhead. Do not equate vCPU
count with CPU time quotas or benchmark results with Firecracker equivalence.

## Current implementation status

The host foundation implements SQLite records, read-only image copies with
SHA-256 verification, provenance recording, independent full-copy work disks,
idempotent registration by caller-selected VM UUID, and pending operation
inspection. The full-copy implementation does not yet preserve sparse allocation
or use APFS clones. Prepared VM defaults are 1 vCPU, 512 MiB and no networking.

`retain`, `rollback` and `branch` are stored policies only; their execution is not
implemented. Inputs are not validated as bootable Debian images. Image import
requires caller-supplied provenance; it does not authenticate the publisher.

The trusted host CLI is available as:

```text
cocoon host ABSOLUTE_PRIVATE_STATE_DIR image import KERNEL INITRD ROOT_RAW PROVENANCE
cocoon host ABSOLUTE_PRIVATE_STATE_DIR image show IMAGE_UUID
cocoon host ABSOLUTE_PRIVATE_STATE_DIR vm create VM_UUID IMAGE_UUID OWNER retain
cocoon host ABSOLUTE_PRIVATE_STATE_DIR vm list
cocoon host ABSOLUTE_PRIVATE_STATE_DIR vm show VM_UUID
cocoon host ABSOLUTE_PRIVATE_STATE_DIR operations
```

The state directory must be private (0700). This CLI is for trusted operators,
not an authorized agent endpoint. The same macOS user can bypass file permissions;
these copies are not tamper-proof against that user. Failed operations remain
pending for inspection; automated reconciliation is not implemented.

`vm start VM_UUID` deliberately returns execution unavailable without launching a
host process or VM. Local socket service, grants, leases, disk recovery, guest
helper, production runner integration, image builder and real VM acceptance are
still outstanding. Existing diskless experiments remain separate.

Before committing: Rust tests, Clippy with warnings denied, formatting checks,
Python harness tests and native protocol checks. Real macOS execution is required
before qualifying any backend. Do not publish credentials, private reports or
operator paths; publishing requires explicit authorization.
