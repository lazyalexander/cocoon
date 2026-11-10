# Rust-first architecture

English (authoritative) · [简体中文](architecture.zh-CN.md)

## Reference and adaptation

This organization was informed by the local Firecracker checkout at commit
[`7a43eb7a20b34c4dbcc31574834810855413a58e`](https://github.com/firecracker-microvm/firecracker/tree/7a43eb7a20b34c4dbcc31574834810855413a58e),
particularly its workspace manifest, `src/firecracker`, `src/vmm`, design document,
contribution guide, toolchain pin and dependency policy. No upstream source code
was copied. Firecracker is Apache-2.0; any later copied code must retain that
license and applicable notices rather than being relabeled MIT.

Firecracker separates its host API from the VMM event loop and owns one microVM
per process. Cocoon adopts the responsibility separation, not Linux-specific KVM,
epoll, seccomp, jailer, TAP or musl assumptions. With Virtualization.framework,
Apple owns vCPU/device execution; Cocoon must not claim to implement Firecracker's
VMM internals or equivalent containment. The native runner remains experimental.

## Implemented workspace

| Crate | Responsibility | Dependencies within Cocoon |
| --- | --- | --- |
| `cocoon` | CLI parsing and host-facing output | runtime, host |
| `cocoon-runtime` | Platform-neutral task validation and attempt state | none |
| `cocoon-host` | SQLite, image custody, VM registration and experimental supervision | platform, guest-wire |
| `cocoon-platform` | Narrow native filesystem FFI and sparse copying | none |
| `cocoon-guest-fixture` | Fixed Linux ARM64 root-disk acceptance | none |
| `cocoon-guest-wire` | Typed bounded experimental guest protocol | none |
| `cocoon-guest-helper` | Linux vsock and guest file/command operations | guest-wire |

SQLite and filesystem custody do not belong in the contract crate. Guest helpers
must never depend on the host administration crate. Every workspace member is a
default member, so root checks cover the entire implementation. CLI command names
remain unchanged. Rust callers of the old `cocoon_runtime::manager` module must
use `cocoon_host` instead; this is an intentional pre-release source API change.

## Language and next backend boundary

Rust is the default for production CLI, host service, authorization, lifecycle,
reconciliation, image management, runner protocol and future guest helper.
Swift is restricted to the existing Apple-framework experiment and any minimal
platform adapter that remains necessary. Python is for development, image
acquisition and acceptance harnesses; C currently supplies the tiny boot fixture.
Neither Python nor Swift should acquire production authorization or registry logic.

The persistent-root experiment now separates Rust process supervision from
Apple framework calls. Native filesystem FFI and Linux fixture syscalls are
isolated in their own crates; control-plane crates continue forbidding unsafe.
The fixed fixture is not the future general vsock guest helper. See the
[persistence experiment](../experiments/microvm/PERSISTENCE.md) for limitations.

Host policy remains outside the per-VM execution path. Guest transport stays
untrusted even with a matching VM ID. Lost transport or runner death is not
termination evidence, and unknown operations require reconciliation.

## Tooling and governance

Use a Cargo workspace, centralized dependency versions, inherited lint policy,
a committed lockfile, pinned toolchain, explicit release profile and one local
check entry point. CI enforces workspace tests and lints on the pinned version;
macOS protocol checks remain distinct from real VM acceptance. Keep existing
private benchmark artifacts excluded. See [contribution policy](../CONTRIBUTING.md)
for review, dependencies, provenance and publication rules.

## Experimental project path

`cocoon-guest-wire` contains strict guest message types and bounded framing;
`cocoon-guest-helper` depends only on that crate and libc, never host administration.
The host now also depends on guest-wire to generate and verify the fixed workflow.
Swift ProjectChannel only transports bytes; Rust owns verification and artifact
receipt after confirmed stop. See the [project experiment](../experiments/microvm/PROJECT.md).
