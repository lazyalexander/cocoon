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
python3 experiments/microvm/probe.py check-kernel --kernel /absolute/path/to/Image
python3 experiments/microvm/probe.py boot --kernel /absolute/path/to/Image --case roundtrip
python3 experiments/microvm/probe.py boot --kernel /absolute/path/to/Image --case deadline --seconds 2
python3 experiments/microvm/probe.py boot --kernel /absolute/path/to/Image --case cancel --seconds 10
python3 experiments/microvm/probe.py benchmark --kernel /absolute/path/to/Image --samples 30
```

An existing uncompressed Linux ARM64 Image must support initramfs, gzip, ELF and
built-in virtio-console. The kernel is operator-supplied, hashed, opened read-only
and never redistributed by this experiment. A generic desktop kernel is useful
for compatibility testing but is not a minimal production kernel.

`check-kernel` inspects embedded IKCONFIG when available. It requires built-in
`CONFIG_BLK_DEV_INITRD`, `CONFIG_RD_GZIP`, `CONFIG_BINFMT_ELF`, `CONFIG_VIRTIO`
and `CONFIG_VIRTIO_CONSOLE`. A candidate configuration is not proof of boot.
Absent embedded configuration is reported as unknown, not compatible; boot may
still be attempted with operator-supplied knowledge. A known incompatible or
malformed embedded configuration is rejected before creating a VM attempt.
Configuration decompression is limited to 2 MiB. This is a harness check; the
native runner itself is an experimental internal interface, not an admission API.

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

The kernel command line uses `panic=0` to prevent automatic reboot after panic.
A serial panic message requests host stop with reason `guest_panic`; it is an
untrusted diagnostic hint and never proves either a panic or VM termination.
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

## Hardware baseline

Ubuntu 6.8.0-146-generic ARM64 passed fixed roundtrip, deadline and cancellation on a 1-vCPU, 128 MiB guest. All 30 repeated roundtrips passed using the same kernel with host filesystem caches retained. Guest readiness had a median of 367.57 ms and P95 of 380.13 ms. The median framework running callback was 100.40 ms. Median runner peak RSS was 9.44 MiB, excluding framework helpers.

In one 10-second deadline trial the baseline runner (SHA-256 prefix 88107d61) requested stop about 663 ms late, then confirmed stop in 4.37 ms. This case checks stop reason and termination. The revised runner uses a strict dispatch timer and reports deadline_due_ms for measuring lateness on hardware.

The lifecycle case pauses one resident VM five times. After each confirmed pause it waits 50 ms, resumes and sends a serial command. PID 1 increments, saves, increments, restores and increments a counter. Acceptance requires the ordered values 1, 1, 2, 1, 2 and confirmed final stop. The checkpoint lives in guest memory and covers only the counter. Full machine snapshots and recovery after runner exit remain open.

```sh
python3 experiments/microvm/probe.py boot --kernel experiments/microvm/build/Image --case lifecycle
```

## Save/restore configuration check

Run `build/cocoon-vz REQUEST.json --check-save-restore` from this directory
with an existing experimental request to validate its actual configuration using
Virtualization.framework. The command creates no VM. Exit zero and `candidate`
mean configuration support only; `save_restore_tested` remains false. Preserve
framework errors and distinguish sandbox restrictions from host capability.
Full persistence still requires real save/restore with matching disk bytes and
termination of the original VM before a new owner resumes it.

## Cross-process persistence experiment

Run `python3 experiments/microvm/persistence.py --kernel experiments/microvm/build/Image`
from the repository root after building. The guest uses a memory-only random marker
and a fixed 512-byte raw-block record: `1P` is flushed before pausing, and `2D`
is flushed only after restoration reads the prior progress. This is a bounded
binary persistence fixture, not a filesystem or general artifact API.

The first launcher pauses, saves framework state and confirms VM termination.
Only after that process exits does the controller copy the disk and start a new
launcher. The generic machine identifier is persisted alongside the state file.
The new launcher restores paused, resumes, and verifies the same memory marker
and continued disk progress without a cold-boot checkpoint message. The original
disk snapshot must remain unchanged. Each invocation uses a fresh private run.

Machine state is host-bound. This experiment does not implement production
admission, an atomic complete snapshot bundle, external-effect rollback, arbitrary
spool execution, concurrent restore exclusion or durable owner-loss reconciliation.
Failures preserve evidence and do not automatically replay. A framework save alone
is not artifact acceptance. The initial missing-machine-identifier restore failure
remains retained privately alongside successful independent attempts.
