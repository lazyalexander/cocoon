# Cocoon

English (authoritative) · [简体中文](README.zh-CN.md)

[User guide](docs/user-guide.md) · [简体中文使用说明](docs/user-guide.zh-CN.md)

Cocoon is an independent execution runtime project designed for agent workloads.
The first target is headless Linux ARM64 execution on Apple Silicon macOS.
an-agent is the intended first consumer, not a runtime dependency.

**Current status: the managed path supports real start/stop, authenticated private
Unix-socket control, guest file/command operations, binary artifact export,
cross-process suspend/resume, approved cold recovery, candidate versions, branches,
rollback and policy-driven automatic suspension. The combined real-VM acceptance
passes. This is still not a completed v0.1 qualification or a claim of adversarial
isolation, capacity, or full Debian tooling.**

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

The remaining work is broader qualification: repeated lifecycle/version cycles,
crash and resource-failure injection, full workload compatibility, capacity and
performance budgets. The managed path uses Virtualization.framework directly;
OCI management is deferred. No native fallback is allowed when VM execution is selected.

Model calls, conversations, planning and multi-agent coordination belong to
consumers. GPU/GUI guests and other host platforms are outside the first delivery
scope. Runtime memory snapshots now have experimental real-VM support but are not
yet covered by all v0.1 qualification gates.

## License

Cocoon's original source and documentation are licensed under the
[MIT License](LICENSE). Operator-supplied Linux kernels and third-party
dependencies retain their own licenses; they are not relicensed by Cocoon.

## Native agent microVM direction

See the [implementation plan and current delivery status](docs/implementation-plan.md).
The trusted host CLI supports registration and real lifecycle operations; the
private socket service additionally exposes authenticated agent operations.

[Rust-first workspace architecture and contribution workflow](docs/architecture.md) · [Contributing](CONTRIBUTING.md)

The [v0.1 acceptance specification](SPECIFICATION.md) defines mandatory gates and
the development endpoint; its targets are not yet achieved guarantees.
