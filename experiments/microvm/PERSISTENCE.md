# Rust persistent-root experiment

English (authoritative) · [简体中文](PERSISTENCE.zh-CN.md)

This is a fixed-workload acceptance path, not a general execution service. Rust
owns typed VM/image/boot/operation identities, SQLite intent and result records,
working-disk leases, bounded runner transport and timeout supervision. Swift only
attaches a virtio block device and reports framework observations. Serial guest
claims are kept separate from framework termination confirmation.

The Rust Linux ARM64 PID 1 mounts `/dev/vda` as an ext4 root, changes root and
executes `/guest-init` from that disk. It verifies its executable and root mount,
writes a fresh 32-byte payload with mode 0600, syncs file and directory, then waits
for host stop. A new boot reads the previous payload and checks permissions.
A marker alone never proves termination. The fixture has no general shell,
network, package manager or agent command endpoint. It is not Debian userland.

## Build and run

The pinned host toolchain needs the `aarch64-unknown-linux-musl` target. The builder
uses explicit Homebrew LLD and e2fsprogs paths, avoiding the Android SDK mke2fs
that may appear first in PATH. `COCOON_LLD` and `COCOON_MKE2FS` explicitly override
these paths; versions and executable digests are recorded, and tool identity is
checked. The LLD executable basename is preserved because it selects the driver.
Ambient target/compiler/flags overrides are rejected. No dependency installation
or kernel download is implicit in this command.

```sh
cargo build -p cocoon --release
cargo test -p cocoon-platform --release eight_gib_sparse_fallback -- --ignored
python3 experiments/microvm/probe.py build
python3 experiments/microvm/root_modules.py
python3 experiments/microvm/build_root.py
python3 experiments/microvm/persistence.py --fixture FIXTURE_DIRECTORY --kernel KERNEL_IMAGE --rounds 10
```

Use the private fixture directory returned by the builder. It contains a sparse
8 GiB logical root disk, initramfs and build manifest; metadata includes tool
versions, hashes and actual allocated bytes. Build and run outputs stay in the
existing ignored directories. On managed macOS, VM execution may require running
outside the execution sandbox; refusal never selects host-native execution.

The Python script only sequences the acceptance. Actual boot ownership resides
in the Rust `host STATE experiment persistence VM RUNNER write|read PAYLOAD`
command. A release host binary is used because full 8 GiB logical-image SHA-256
verification is expensive in debug builds. Verification time is not a guest boot
benchmark and remains an optimization item.

## Persistence and containment

The platform crate isolates native `clonefile` FFI. APFS clones are preferred;
only unsupported/cross-device errors permit sparse copying. Permissions, I/O,
space and existing-target errors fail. Sparse copying seeks across all-zero
chunks and preserves length. Copy reports record method, logical size and current
allocated size; APFS shared-block accounting is not exclusive physical usage.
Work copies are writable, baselines read-only, and digests are checked before use.

A separate lease file is locked, not the disk itself: Apple framework attachment
uses its own disk locking. The database records uncertainty before spawn. Unknown
or interrupted boots block reuse; runner death is not proof of VM termination.
The runner has a ten-second fixture deadline; the Rust watchdog kills/reaps an
overlong runner after twenty seconds without treating this as VM termination.
Guest output is bounded separately from the trusted control response.

Schema v0 metadata migrates transactionally to v1, preserving old image records
and prepared VMs. Missing historical copy accounting remains unknown. Newer
unknown schema versions are refused. Root state and image locations remain
trusted-host data; arbitrary same-user host code is outside this boundary.

The experiment does not yet implement an independent durable supervisor,
automated unknown-outcome reconciliation, grants, rollback/branch execution or
production disk capacity enforcement. Retain across explicit host-confirmed
stops is the acceptance target. Do not manually clear unknown state to retry.

## Provenance and verification

The original fixture code is MIT. Linked Rust standard-library and musl components
retain their upstream licenses; toolchain and e2fsprogs are external build tools,
not relicensed by this repository. Operator-supplied kernels retain their Linux
and distributor notices. No kernel, disk, tool binary or private report is committed.

Run `tools/check`, cross-target guest Clippy, native runner protocol checks and
the real ten-round sequence. Host tests cover sparse bytes/allocation, native
clone independence, overwrite/symlink rejection, schema refusal and invalid
fixture requests. Host-only tests do not qualify guest execution. A passing
sequence requires twenty distinct boot IDs, unchanged baseline digest, ext4 root
and executable checks, byte/permission persistence and framework-confirmed stops.

The explicit `root_modules.py` step fetches SHA-256-pinned Debian installer and
ext4/scsi module packages for kernel 6.12.107+deb13-arm64. Only virtio_blk, crc32c_generic, jbd2,
mbcache, crc16 and ext4 enter the fixture initramfs, in dependency order. Module
vermagic is checked during preparation and the kernel validates modules during
loading. They retain upstream GPL and distribution notices. HTTPS checksum pins
are not a claim of detached-signature verification.

## Observed acceptance

On the current Apple Silicon macOS host, ten write/read cycles passed: twenty
separate boot identities, ext4 root and disk executable verified, 0600 payload
retained, framework stop confirmed each time, baseline SHA-256 unchanged.
The 8 GiB logical baseline initially allocated 207,306,752 bytes, including its
journal. APFS clone independence and an explicit 8 GiB sparse-copy fallback test
also passed. Allocation figures do not measure aggregate exclusive APFS space.

Earlier attempts exposed a disk-lock conflict and missing runtime filesystem
modules (including dynamically requested CRC32C). Those reports were retained;
attachment failures remain quarantined unknown, while guest-stage failures have
confirmed host stops. The passing run used a new environment, never automatic
replay of unknown attempts. No general backend or v0.1 performance claim follows.
