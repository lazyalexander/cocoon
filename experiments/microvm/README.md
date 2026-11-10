# Minimal macOS microVM experiment

English (authoritative) · [简体中文](README.zh-CN.md)

This experiment tests a smaller path than the initial OCI-based proposal:
one native runner, one ARM64 Linux Image, one tiny initramfs and one serial
device. There is no Containerization package dependency, image daemon, network,
block device, shared directory, graphics device or agent framework in this path.
It uses Apple's Virtualization.framework instead of implementing a new VMM.

The runner belongs to Cocoon and has no an-agent or spool dependency. The C guest
is a fixed test workload, not an arbitrary task executor or a security-qualified
guest agent. The existing Rust task model is a separate contract prototype;
neither surface yet admits or supervises production workloads.

## Build and run

Requires Apple Silicon, Xcode tools, Python 3 and LLVM's `ld.lld`.
No package installation or download is performed by these commands.

```sh
python3 experiments/microvm/probe.py build
python3 experiments/microvm/probe.py doctor
python3 experiments/microvm/runner_checks.py experiments/microvm/build/cocoon-vz
python3 -m unittest discover -s experiments/microvm -p 'test_*.py' -v
python3 experiments/microvm/probe.py boot --kernel /absolute/path/to/Image --case roundtrip
python3 experiments/microvm/probe.py boot --kernel /absolute/path/to/Image --case deadline --seconds 2
python3 experiments/microvm/probe.py boot --kernel /absolute/path/to/Image --case cancel --seconds 10
python3 experiments/microvm/probe.py benchmark --kernel /absolute/path/to/Image --samples 30
```

An existing uncompressed Linux ARM64 Image must support initramfs, gzip, ELF and
built-in virtio-console. The kernel is operator-supplied, hashed, opened read-only
and never redistributed by this experiment. A generic desktop kernel is useful
for compatibility testing but is not a minimal production kernel.

Build outputs go under ignored `build/`. Private requests, input image, logs and
evidence go under ignored `runs/`, with a new private directory for each attempt.
The native runner is optimized and ad-hoc signed; this is not a notarized release.

## Acceptance

The `roundtrip` guest emits readiness, reads the input bundled in its initramfs,
uppercases it, emits a token-bound result and stays alive. The host checks the
returned bytes and stops that VM. This demonstrates bounded bootstrap input and
serial output only, not general file transfer, arbitrary code execution or custody.

The `deadline` guest emits readiness and remains alive without a completion
marker. The host must stop it after the selected deadline. Both cases require a
successful framework stop callback and stopped state (or an observed guest stop).
A readiness marker is workload evidence, not termination evidence or a grant.

The `cancel` case uses the waiting guest and sends SIGTERM to the owned runner
after two seconds (or half the deadline if shorter). The runner handles SIGTERM
and SIGINT on its VM queue, requests a framework stop and must still confirm
termination. A signal arriving before guest readiness does not pass acceptance.
Serial reads are nonblocking, at most 4 KiB per event, and processed on the VM
queue; guest output does not accumulate in a separate callback queue. Further
output is drained without retention once stopping begins.

Serial capture is capped at 64 KiB. The runner has a 5-second stop watchdog;
the Python harness adds a final process timeout. Watchdog expiry is recorded as
unconfirmed termination, never a pass or an automatic retry. There is no durable
supervisor, crash reconciliation, caller authentication or production resource
accounting in this experiment.

The harness kills and reaps its local runner after an outer timeout or an
interruption, while preserving unknown VM termination. This is local cleanup,
not a substitute for a durable independent supervisor or VM reconciliation.

The requested initial VM size is 1 vCPU and 128 MiB. Validation may refuse it;
only a real successful boot establishes support. No environment checks are
disabled and no other engine or native process is selected on refusal.

## Measurement

Reports separate identity preparation, whole runner process time, framework
start completion, guest readiness, guest completion and VM stop confirmation.
Runner timestamps use a monotonic clock from runner initialization. Stop duration
can be derived from `stopped_ms - stop_requested_ms`. Runner peak RSS excludes
framework helper processes and must not be reported as total VM overhead.

Benchmark samples use separate runner processes and new VM attempts, reusing
already built artifacts. They do not flush host filesystem caches. The summary
reports median and nearest-rank P95 only for passing samples and stops at the
first failure. Build and configuration refusal are never reported as guest boot
performance. Compare with Firecracker only using matching readiness definitions,
guest kernels, hardware context and cache conditions; no parity is claimed.

## Current observation

The ARM64 guest init and optimized native runner build successfully. Both real
boot cases currently stop at configuration validation with `VZErrorDomain` code 2
in the managed execution context. Linux startup, roundtrip and deadline-stop
acceptance remain unqualified. The earlier Hypervisor context probe succeeded,
so the framework refusal needs diagnosis in the actual host environment.

`doctor` requires no kernel and never creates a VM. It records framework
availability and whether `kern.hv_support` and `hw.optional.arm64` can be read,
including errno on failure. Both reads currently return EPERM in the managed
execution context. This supports investigating restricted capability detection;
it does not prove the sole cause of the framework refusal or a hardware defect.
Diagnostic reports are private files under `runs/`, not boot benchmarks.

Native runner protocol checks reject unknown fields, invalid limits, malformed
JSON and requests over 16 KiB before creating a VM. The macOS CI job builds and
signs the runner and runs those checks; it does not boot or qualify a guest.
Python harness process tests likewise do not establish VM cancellation support.

The implementation configures fewer devices than the earlier 1 GiB preflight,
but this does not establish working 128 MiB guests or Firecracker-equivalent
security. macOS process containment and protection against hostile guest output
remain separate qualification work.
