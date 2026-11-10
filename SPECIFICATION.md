# Cocoon v0.1 acceptance specification

English (authoritative) · [简体中文](SPECIFICATION.zh-CN.md)

Status: development requirements, **not achieved guarantees**. This document
sets the development endpoint for v0.1. Every mandatory gate below must pass on
the release candidate; missing evidence is pending, never a pass. Changing a
target requires an explicit specification review, not changing it to fit results.

## 1. Supported product and trust boundary

v0.1 is a single-host, developer-facing, headless Linux ARM64 microVM system on
Apple Silicon macOS. It is not a hostile multi-tenant cloud qualification.
Production control, policy, persistence and guest-helper logic are Rust; any
Swift code is limited to the Apple framework adapter. Python/C experiments are
not the production control plane.

The reference target is a base Apple M5 Mac with 16 GB unified memory and an APFS
state volume, one exact macOS build, and one pinned Debian ARM64 glibc image.
Record the actual OS-reported RAM capacity rather than assuming GB equals GiB.
Record machine model, CPU, RAM, filesystem, OS, Rust/Xcode versions and all binary
and image digests. Other configurations are unsupported until tested. This is a
narrow initial support matrix, not support for all Apple Silicon Macs.

Default VM: 1 vCPU, 512 MiB RAM, 8 GiB logical sparse writable disk, no network.
No GPU, GUI, audio, USB, passthrough or host directory sharing. Local CLI and Unix
socket API only. Explicit NAT grants permit general connectivity, including
potential host/LAN access; they are not destination-filtered networking.

The host manager and macOS account are trusted. Agents receive scoped tools,
not arbitrary host execution under that account. Guest root may modify or crash
the guest. Host registration, ownership, grant issuance, lease expiry and
reclamation must remain outside guest authority. Apple virtualization, macOS and
hardware are dependencies, not components proven safe by Cocoon tests.

### Capacity target on M5 / 16 GB

The release target is **eight resident default VMs, with four concurrently active
on light agent work**. Residency is not simultaneous CPU saturation. Budget
roughly 6–8 GiB for macOS, editor/browser and safety margin; eight VMs at the
768 MiB per-VM budget consume up to 6 GiB, with the manager counted separately.
Local model inference consumes the same memory pool and reduces VM capacity.
These are planning estimates, not measured support claims.

| Workload | Guest RAM per VM | Planning concurrency |
| --- | --- | --- |
| Idle, shell, small scripts, waiting for model responses | 512 MiB | 8 resident / 4 active is mandatory; 10–12 resident is exploratory only. |
| Python/Node tooling and small tests | 1 GiB | 4–6; workload-dependent, not a v0.1 guarantee. |
| Compilation and medium test suites | 2 GiB | 2–4; workload-dependent. |
| Large builds, databases or browser workloads | 4 GiB | 1–2; workload-dependent. |

Qualify four active VMs first, then eight resident VMs with four active, for two
hours each. Light active work means a repeated, versioned shell/Python fixture
that edits files, hashes data, transfers bounded output and runs small tests; it
excludes package downloads, large builds and local model inference. Pin fixture
bytes, input sizes and cadence before collecting results; record those with the
release evidence. Idle guests still answer a periodic health challenge.

Keep a documented, repeatable host editor/browser workload during the capacity
run. Sample host memory pressure and swap usage every 10 seconds. No VM/helper
OOM, no warning/critical memory pressure lasting 60 seconds, and no swap growth
across six consecutive one-minute samples after a 10-minute warm-up. A nonzero
pre-existing swap allocation alone is not failure. Control latency and stop
budgets must still pass. This qualifies practical capacity, not the maximum
number that can merely reach a boot callback.

## 2. Mandatory behavior gates

| ID | Requirement and pass condition |
| --- | --- |
| F01 | Create, boot, query, execute, stop and destroy via CLI and API. Every operation has a stable identity and structured result. No native execution fallback under any failure. |
| F02 | A root-authorized agent installs a package under an explicit NAT grant, edits system configuration and runs a compiler/interpreter workload. A non-root guest session cannot escalate through Cocoon's helper. |
| F03 | One VM executes 100 sequential commands, with separate task/attempt and boot identities, correct exit codes, bounded stdout/stderr and a byte-exact binary file roundtrip. A duplicate execution request never starts a second attempt. |
| F04 | Retain preserves files, permissions and system configuration across confirmed stop/restart. Rollback removes these changes and restores the pinned baseline. Branch publishes a bootable independent derived image; its parent digest remains unchanged. Each policy passes 10 cycles. |
| F05 | Guest helper termination, broken startup configuration and guest panic cannot prevent host stop. Guest completion markers and socket loss cannot mark a VM stopped. A prepared replacement kernel is activated only by host authorization; failed boot leaves the original baseline recoverable. |
| F06 | Expired, revoked, malformed, wrong-VM and insufficient-scope credentials cannot execute or mutate management state. Guest/work credentials cannot register, reassign, grant, reset, publish or destroy. Host credentials never enter the guest image or command environment. |
| F07 | Offline guests have no network device and cannot reach host, LAN or Internet. NAT requires a host grant. Network revocation stops the VM; completion is reported only after confirmed stop. Resume remains offline unless a new grant is issued. |
| F08 | Output flooding, oversized/malformed messages, path traversal, guest symlinks and a full guest disk cannot write outside the configured export store, bypass limits or crash the manager. Exported guest bytes are never automatically executed. |
| F09 | A running VM refuses rollback, branch or disk deletion until stop is confirmed. Failed image publication preserves the working disk and never exposes an incomplete image as usable. Every command and image has an explicit provenance/identity association; content hashes do not imply trusted publishers. |
| F10 | Lease expiry and host cancellation terminate the owned VM independently of the helper. If confirmation fails, state is unknown and disk reuse/deletion, branching and automatic task replay remain blocked. No success is inferred from PID disappearance. |
| F11 | Four simultaneous VMs keep files, credentials, output and lifecycle observations separate. Stopping or corrupting one cannot stop another or alter its disk. Requests from a previous boot cannot control a new boot. |

## 3. Failure, security and reliability gates

Inject each of the following at least 10 times at the applicable durable-intent,
side-effect and completion boundaries: manager SIGKILL/restart, runner SIGKILL,
control-channel loss, guest-helper death, image-copy failure and host-state-volume
write failure. Run host disk-full tests on a bounded test volume, never by filling
the developer's real system disk.

After manager restart, reconciliation must finish within 30 seconds as either
confirmed state or explicit unknown. Known owned runners must remain stoppable.
Unknown resources remain quarantined with an actionable recovery procedure;
manual deletion of records is not a recovery procedure. Run a manager-restart
case with a real VM and prove no duplicate VM or task is created. Power-loss and
physical storage failure durability are outside v0.1 qualification.

Complete 100 create/execute/stop/destroy cycles without failures or leaked owned
runners, registered live VMs, open workspace files or unreferenced working disks.
Run both capacity profiles from section 1, including command churn and the
stop/restart of one active VM. Any unexplained failure fails the candidate; preserve it before rerunning.

Parser/authorization/state-machine negative tests run in CI. Before release,
fuzz each external protocol parser with a fixed recorded seed corpus for at
least one hour on a documented runner; no crashes, hangs beyond parser deadlines
or configured allocation-limit violations. Keep reproducers and sanitized
summaries. Fuzzing and tests are not a proof of absence of VM escapes. Kani proofs
may later cover selected invariants but are not claimed or required for v0.1.

## 4. Performance and measurement gates

These are Cocoon engineering budgets, not Firecracker parity claims. Use release
binaries and the default persistent-root VM with production helper. A ready event
alone is insufficient: readiness ends at completion of an authenticated command
returning a fresh host challenge. Exclude image downloading and compilation;
report disk cloning separately from boot. Do not hide failures from percentiles.

Except first-after-restart and idle-footprint/CPU tests, measure latency with
four running guest VMs in the pinned light-work profile. Boot/create tests replace
one of those four slots while the other three continue working. Use 100 samples
per operation unless stated otherwise. All times use the host monotonic clock,
including queueing from API acceptance; report failures, never success-only P95.

| Metric | Mandatory v0.1 budget | Later optimization objective |
| --- | --- | --- |
| Short local API (register, status, grant) | P95 <= 100 ms; 1,000 status samples | <= 20 ms |
| APFS work-disk clone | P95 <= 1 second | <= 200 ms |
| Existing-disk boot to authenticated challenge result | P95 <= 2 seconds | <= 500 ms |
| Create + boot to authenticated challenge result | P95 <= 3 seconds | <= 1 second |
| First boot after host restart | Every sample <= 5 seconds; 5 host restarts | No additional target |
| No-op command in an already running guest | P95 <= 100 ms, through exit-result receipt | <= 30 ms |
| Forced stop to framework confirmation | P95 <= 2 seconds; every sample <= 5 seconds | <= 500 ms |
| Restart to authenticated challenge result | P95 <= 4 seconds | <= 1 second |
| Rollback disk, VM already stopped | P95 <= 1 second | <= 300 ms |
| Branch working disk, VM already stopped | P95 <= 1 second | <= 300 ms |
| Reclaim records/work disk, VM already stopped | P95 <= 2 seconds | <= 500 ms |
| Manager-restart reconciliation | Every case <= 30 seconds to confirmed or explicit unknown | <= 5 seconds |
| Small file roundtrip | 4 KiB upload + download P95 <= 100 ms | No additional target |
| Large file transfer | 100 MiB each direction, sequentially; P5 throughput >= 100 MiB/s over 30 transfers each way | No additional target |
| Idle physical footprint | Manager <= 32 MiB; per-VM increment <= 768 MiB including guest RAM | Reduce measured overhead, no invented 5 MiB parity |
| Idle CPU | Mean <= 1% of one host core for manager + one VM and its helpers | No additional target |

Optimization objectives are not extra release blockers. Register/grant timing
includes durable metadata commit. Clone/reset/branch timings end after durable
local metadata and disk publication, but exclude full-image hashing and boot
validation. Full image publication must report clone, digest and validation
phases separately; it cannot be advertised as a one-second operation. Pin and
report baseline allocated bytes, logical disk size and changed data volume.
Full-copy fallback has a separate size/throughput report, not the clone budget.

File transfer uses byte-checked, incompressible fixture data over the local helper
channel. Throughput includes destination write/flush completion; it excludes
Internet transfers. Use nearest-rank percentiles for latency and throughput.
Graceful shutdown may wait at most 5 seconds before entering forced stop; record
both phases, never hide the guest wait inside the forced-stop metric. A watchdog
expiry fails stop acceptance and retains an unknown result.

For footprint/CPU, sample once per second for 60 seconds after challenge
completion and 10 seconds idle. Include attributable framework helpers and
subtract a matched no-VM baseline for each VM increment; publish both totals and
attribution. Missing/shared attribution that cannot be resolved cannot pass.
Measure single-VM and eight-idle-VM profiles separately; the 1% CPU gate is for
manager plus one idle VM, not an unsupported eight-VM aggregate promise.
First-after-restart measurements do not claim empty storage caches.

Record every sample, host load and thermal/power conditions. After 100 lifecycle
cycles, idle host footprint must return within 16 MiB of the warmed no-VM baseline
within 30 seconds, with no surviving per-VM resources. These limits do not claim
hard host CPU quotas or aggregate VM memory enforcement. Disk size bounds and
protocol byte limits require separate functional enforcement tests.

For subsequent releases, compare baseline and candidate on the same machine,
image and harness. A median or P95 regression above 10% requires investigation
and explicit approval even if the absolute budget passes; alternate A/B order
across three runs to reduce cache and thermal bias.

## 5. Development endpoint and release evidence

v0.1 is finished only when **all** behavior, failure, security, reliability and
performance gates pass, and the following delivery requirements are satisfied:

- A clean checkout builds using the pinned toolchain/lockfile; documented local
  installation and uninstall work without unpublished developer files. No
  mandatory Python runtime for production operation. Notarized distribution and
  automatic updates are deferred.
- Documented CLI and versioned local API, reproducible image recipe with pinned
  inputs and third-party notices, recovery procedure, threat boundary and support
  matrix are available in English and Simplified Chinese.
- Workspace tests, Clippy, formatting and native adapter protocol checks pass.
  Automated dependency advisory/license/source checks are added and pass; any
  exception is documented with owner, justification and expiry. No unresolved
  demonstrated authorization bypass, data-loss bug or false termination report.
- Each gate has a test identifier, exact source revision and artifact digests,
  result and sanitized evidence summary. Raw operator paths, tokens, images and
  reports stay private. CI results and real-VM evidence are labeled separately.
- A fresh reviewer/operator can run the documented end-to-end demo: grant root,
  modify guest, restart and retain, kill helper, host-stop, rollback, branch,
  revoke access and destroy. Review records explicitly approve the candidate.
  Publishing remains a separate authorized operation.

The endpoint is **a qualified developer microVM v0.1 on the M5 / 16 GB reference
profile, with eight resident VMs and four light-work VMs active**, not perpetual expansion
until Firecracker feature parity. GUI/GPU, other hosts/architectures, remote API,
OCI compatibility, live memory snapshots, migration, hostile host-user isolation
and cloud multi-tenancy require later specifications. A failed mandatory budget
keeps v0.1 unfinished; it does not justify adding unrelated features.

### Current evidence

The diskless C guest passed real roundtrip, deadline, cancellation and 30 repeated
boots at 128 MiB. Separately, the fixed Rust root-disk fixture passed ten write /
confirmed-stop / reboot-read cycles (20 distinct boots) with an unchanged baseline.
This is not the Debian production helper, agent access or full release acceptance.
All v0.1 gates remain pending until corresponding full-path evidence exists.

### Reference

Inspired by Firecracker's versioned
[specification](https://github.com/firecracker-microvm/firecracker/blob/7a43eb7a20b34c4dbcc31574834810855413a58e/SPECIFICATION.md),
[test methodology](https://github.com/firecracker-microvm/firecracker/blob/7a43eb7a20b34c4dbcc31574834810855413a58e/tests/README.md)
and scoped verification practice. Thresholds and product scope here are Cocoon's
own proposals, not upstream measurements or certifications.

### Experimental snapshot evidence

A fixed 512 MiB guest passed three sequential cross-process state save/restore
samples on macOS 27.0.1 (26A434), with random RAM-nonce and counter continuity,
matching disk digests at restore admission, and confirmed termination. This is platform feasibility evidence,
not v0.1 lifecycle, recovery, isolation, capacity, or performance qualification.
The current exclusion of runtime memory snapshots remains in force.

### Managed-path evidence

The managed real-VM path has passed a combined scenario covering authenticated
socket start, binary guest writes and execution, 12,000-byte artifact export,
confirmed suspend and process exit, resume with a new generation, candidate
approval, booting an independent branch, rollback, and policy-triggered automatic
suspension. A second combined run additionally corrupted the real snapshot,
required recovery approval, cold-recovered the persistent working disk, and then
passed automatic suspension and subsequent operations. These results establish
the exercised functions only; they do not close the remaining v0.1 capacity,
performance, cleanup, broad workload, or full fault-injection gates.
