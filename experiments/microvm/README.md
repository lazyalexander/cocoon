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

## Earlier incompatible-kernel observation

The ARM64 guest init and optimized native runner build successfully. The managed
execution context refuses framework configuration and cannot read the two host
capability sysctls (EPERM). An operator-run `doctor` in a normal terminal reports
framework support and both capability values as 1.

An operator-run roundtrip reached framework running state at approximately
168 ms and confirmed host stop. The guest never emitted readiness: its LinuxKit
kernel has `CONFIG_BLK_DEV_INITRD` disabled and panicked while trying to mount a
root filesystem. The previous `panic=-1` command line caused immediate reboot,
repeating panic output until the serial cap. The harness now rejects that known
incompatible configuration, and panic reboot is disabled. No raw private reports
or operator kernel binaries are redistributed here.

This establishes observed VM startup and stop for that failed attempt, not guest
roundtrip, deadline or cancellation acceptance. The 168 ms framework callback is
not guest readiness or a successful boot benchmark. A compatible kernel and real
acceptance runs are still required. The kernel-free `doctor` never creates a VM;
its private reports are capability diagnostics, not boot benchmarks.

Native runner protocol checks reject unknown fields, invalid limits, malformed
JSON and requests over 16 KiB before creating a VM. The macOS CI job builds and
signs the runner and runs those checks; it does not boot or qualify a guest.
Python harness process tests likewise do not establish VM cancellation support.

The implementation configures fewer devices than the earlier 1 GiB preflight,
but device count alone does not establish guest acceptance or Firecracker-equivalent
security. macOS process containment and protection against hostile guest output
remain separate qualification work.

## Pinned Debian kernel and real diskless acceptance

The explicit download helper retrieves a Debian trixie installer ARM64 kernel:

```sh
python3 experiments/microvm/fetch_kernel.py
python3 experiments/microvm/probe.py boot --kernel experiments/microvm/build/debian/linux --case roundtrip
python3 experiments/microvm/probe.py boot --kernel experiments/microvm/build/debian/linux --case deadline --seconds 2
python3 experiments/microvm/probe.py boot --kernel experiments/microvm/build/debian/linux --case cancel
python3 experiments/microvm/probe.py benchmark --kernel experiments/microvm/build/debian/linux --samples 30
```

The download is opt-in, capped at 128 MiB, and requires SHA-256
`9c37ace54ef7e1919411fd912e9f34d6982ddd7f4c8ef3c8aee56f4c4d70db67`.
The official source URL is recorded in `fetch_kernel.py`; upstream `current` can
move, in which case the helper fails rather than repinning. Existing operator
files are verified, never replaced. The pin was compared with Debian's HTTPS
SHA256SUMS; no detached signature verification is claimed. The kernel remains
under its own Linux/Debian licenses, is stored only in ignored `build/`, and is
not redistributed by this repository. Debian's installer root filesystem is
not needed: the experiment still runs Cocoon's freestanding C init.

Real runs on Apple Silicon macOS 27.0.1, outside the managed sandbox, passed
roundtrip, a two-second deadline and SIGTERM cancellation with 1 vCPU and
128 MiB. A separate 512 MiB roundtrip also passed. All required guest readiness,
expected output (where applicable) and framework termination checks passed.

Thirty independent repeated roundtrips at 128 MiB passed (30/30). Guest-ready
median was 239.875 ms and nearest-rank P95 was 245.762 ms, measured from runner
initialization. Caches were not flushed. These are fixed-workload diskless
measurements, not cold-cache boots, full Linux development environments, total
memory accounting or Firecracker comparisons. Raw reports remain private under
ignored `runs/`. Writable-root persistence, guest helper access and production
ownership recovery remain unimplemented and unqualified.

See the [Rust persistent-root experiment](PERSISTENCE.md) for the new bounded disk path.

## Experimental cross-process state save and restore

Build the persistent Rust fixture, then run three independent sequential samples:

```sh
python3 experiments/microvm/root_modules.py
python3 experiments/microvm/probe.py build
python3 experiments/microvm/build_root.py
python3 experiments/microvm/probe.py snapshot --fixture /absolute/private/root-BUILD_ID --kernel /absolute/private/linux --samples 3
```

`snapshot-api` performs a compile-only SDK check. The `snapshot` command boots a
512 MiB fixed guest, persists and synchronizes a disk payload, pauses the VM,
saves complete machine state, confirms the old runner stopped, and starts one new
runner from that state. A RAM-only counter must advance exactly once across the
restore and the working-disk SHA-256 must remain unchanged. The machine identifier
is stored beside the private state file and reused on restore. No cold-boot or
native fallback is used, and failed samples are not replayed.

On macOS 27.0.1 (26A434), three of three real cross-process samples passed. Each
state file was 24,121,344 bytes. Save-process wall times were 472.7–502.7 ms;
restore-process wall times were 1,206.2–1,214.2 ms, including the guest's fixed
one-second wait before its resumed marker. These are fixed-workload measurements,
not approved production budgets or total memory-release measurements.

This spike does not implement production leases, checkpoint/WAL publication,
automatic suspension, pipeline policy, artifact custody, adversarial isolation, or
parallel snapshot restore. Protocol tests alone do not establish VM support; the
real guest counter, disk digest, process boundary, and framework termination
checks are required.
