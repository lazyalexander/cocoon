# Cocoon

English (authoritative) · [简体中文](README.zh-CN.md)

[User guide](docs/user-guide.md) · [简体中文使用说明](docs/user-guide.zh-CN.md)

Cocoon is an independent execution runtime project designed for agent workloads.
The first target is headless Linux ARM64 execution on Apple Silicon macOS.
an-agent is the intended first consumer, not a runtime dependency.

**Current status: persistent host registration, real diskless and ext4-root VM
acceptance, and an offline Rust/vsock shell-project experiment. Three project
boots passed code editing, execution and artifact receipt. Production agent
admission, recovery and the complete Debian backend remain unqualified.**

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
a repeated-boot benchmark harness. Diskless and fixed persistent-root acceptance have passed; production qualification remains pending.

The kernel-free `doctor` command records capability-read errors separately from
hardware support. Native runner protocol checks run on macOS CI without booting
a guest. Cancellation passed the bounded diskless guest experiment; adversarial output
and production supervision remain separate qualification work.

The earlier empty-context probe remains under `experiments/macos-preflight/`.
Neither experiment depends on an-agent or starts existing Docker/Lima machines.

The [offline project experiment](experiments/microvm/PROJECT.md) now exercises
Rust vsock file operations, shell-code editing/execution and bounded artifact
receipt in a real VM. Three consecutive project boots passed. It is not yet an
external agent service or complete Debian development image.

## Next acceptance gate

The fixed Rust ext4-root fixture now passes ten persistence cycles. Next add
the Debian userland and production guest-helper/control path; fixed-workload
acceptance does not qualify those capabilities. It uses Virtualization.framework directly; OCI management is deferred.
Only then connect a real backend to admission, durable ownership, reconciliation
and artifact custody. No native fallback is allowed when VM execution is selected.

Model calls, conversations, planning and multi-agent coordination belong to
consumers. GPU/GUI guests, live memory snapshots and other host platforms are
outside the first delivery scope.

## License

Cocoon's original source and documentation are licensed under the
[MIT License](LICENSE). Operator-supplied Linux kernels and third-party
dependencies retain their own licenses; they are not relicensed by Cocoon.

## Native agent microVM direction

See the [implementation plan and current delivery status](docs/implementation-plan.md).
The trusted host CLI now supports persistent image and VM registration; VM execution
remains unavailable through this CLI.

[Rust-first workspace architecture and contribution workflow](docs/architecture.md) · [Contributing](CONTRIBUTING.md)

The [v0.1 acceptance specification](SPECIFICATION.md) defines mandatory gates and
the development endpoint; its targets are not yet achieved guarantees.
