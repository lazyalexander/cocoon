# Cocoon

English (authoritative) · [简体中文](README.zh-CN.md)

Cocoon is an independent execution runtime project designed for agent workloads.
The first target is headless Linux ARM64 execution on Apple Silicon macOS.
an-agent is the intended first consumer, not a runtime dependency.

**The Apple Silicon experiment passed roundtrip, deadline and cancellation checks. All 30 repeated boots passed with median guest readiness at 367.6 ms. The production backend remains in development.**

## Model

- A caller is an attributed agent, human or service. Attribution is not authentication.
- A task describes pinned executable content, arguments and requested limits.
- An attempt represents one execution of a task. Retries create new identities.
- Host admission owns authorization; guest instructions never enlarge it.
- A stop request, owner loss and confirmed termination are separate states.

The Rust library validates a narrow offline task profile and models attempt
transitions in memory. It does not persist tasks, authenticate callers, enforce
limits, supervise processes or accept artifacts. The lifecycle methods are
trusted-host APIs, not endpoints for guest reports. Matching an attempt ID only
checks attribution; it is not proof of termination. The future backend must
independently establish VM ownership and termination before calling them.

## Local development

```sh
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
cargo fmt --all -- --check
cargo run --offline -- plan examples/task.json
```

`plan` accepts at most 64 KiB, rejects unknown fields and unsupported networking,
and always reports execution unavailable. The example uses a deliberately
nonexistent image identity; it is a schema example, not a runnable guest.
Pinning bytes does not authenticate their publisher. Plans are not approval tokens.

The minimal boot experiment is documented in
[`experiments/microvm/README.md`](experiments/microvm/README.md). It builds a
freestanding ARM64 Linux init and an optimized native framework runner with no
OCI package dependency. It includes input/output, deadline-stop acceptance and
a repeated-boot benchmark harness.

The kernel-free `doctor` command records capability-read errors separately from
hardware support. macOS CI separately checks the runner protocol.

The earlier empty-context probe remains under `experiments/macos-preflight/`.
Neither experiment depends on an-agent or starts existing Docker/Lima machines.

## Next acceptance gate

Compare guest readiness across generic and minimal kernels with the same workload. Measure host overhead and deadline lateness before integrating durable supervision and reconciliation.
No native fallback is allowed when VM execution is selected.

Model calls, conversations, planning and multi-agent coordination belong to
consumers. GPU/GUI guests, live memory snapshots and other host platforms are
outside the first delivery scope.

## License

Cocoon's original source and documentation are licensed under the
[MIT License](LICENSE). Operator-supplied Linux kernels and third-party
dependencies retain their own licenses; they are not relicensed by Cocoon.

## Spool backend integration

`cocoon plan-spool TASK.json` validates the separate `cocoon-spool-task-v1`
profile: caller, release_sha256, kernel_sha256, initramfs_sha256, platform
(`linux-arm64`), argv, limits and network (`false`). Digests are 64 hexadecimal
characters. The executable is an absolute guest path. Limits retain the original
profile bounds. This does not load or verify the referenced bytes, grant execution,
or start a VM. The OCI proposal remains compatible through `plan`.

The intended caller is the host ExecutionBackend adapter. The host owns admission,
disclosure and artifact acceptance; Cocoon owns the execution environment and
trusted termination observations. Native macOS binaries cannot be passed through
as Linux workloads. The next acceptance requires a real offline spool, verified
input bytes, bounded output transfer, cancellation and owner-loss reconciliation.
The existing fixed microVM probe does not qualify this profile.
