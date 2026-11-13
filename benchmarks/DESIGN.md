# Cocoon benchmark design

English (authoritative) · [简体中文](DESIGN.zh-CN.md)

This is the execution design for the [benchmark protocol](README.md) and the
[v0.1 acceptance gates](../SPECIFICATION.md). It defines what a future collector
must measure. The current real-VM acceptance proves a small project workflow,
but it supplies no latency or capacity score. A missing feature or instrument is
`blocked`, never zero latency or a pass.

## 1. Runs and ownership

| Lane | Trigger | Outcome |
| --- | --- | --- |
| Contract | Each PR, Linux CI | Rust/Python/parser checks; no VM performance claim. |
| Native protocol | Each PR, macOS CI | Build and sign runner, check protocol without a boot. |
| Real VM correctness | Explicit operator run on pinned Mac | One full managed project/lifecycle acceptance before timing. |
| Reference performance | Release candidate on base M5 / 16 GB | Full matrix, private raw evidence, sanitized review summary. |

The operator owns image import, registration, grant issuance, VM stop, and eventual
reclaim. A primary agent owns project import, guest edits, commands and export
under a scoped grant. No benchmark task may grant itself lifecycle authority.
The collector is Python; timing probes and trusted lifecycle observations come
from the Rust host and signed macOS runner. Python is outside the measured VM
control plane. No native execution fallback is allowed.

## 2. Pinned inputs and environment

Use release binaries from one commit and `Cargo.lock`. Record hashes of host
binary, guest helper, signed runner, kernel, initrd, baseline disk and project
fixture. Record machine identifier, OS build, RAM reported by macOS, APFS volume,
power mode, thermal state, free disk, Rust/Xcode versions and whether the host is
otherwise idle. A private manifest contains exact paths and operator details;
published summaries contain only digests and nonsensitive machine labels.

The default VM is 1 vCPU, 512 MiB RAM, 8 GiB logical sparse disk, offline. The
project fixture has versioned bytes: one knowledge file, one failing script
which the agent replaces, one check script, and a deterministic build output.
Run a correctness warmup that imports, edits, checks, builds and byte-checks
that output. For the capacity fixture add bounded shell/Python work only after
the guest image actually includes the pinned interpreter. Pin cadence, input
size and expected digest before measuring. Use seeded incompressible bytes for
file transfer and retain the seed in the private manifest.

## 3. Timing and sample records

Use a host monotonic clock. Start each latency at API acceptance, including
queueing, and end only at the specified trusted observation plus durable metadata
commit. Boot readiness additionally requires the fresh host challenge to return
through an authenticated guest command. Stop ends at framework-confirmed runner
termination; socket loss, guest exit and PID absence do not qualify. Export ends
after host destination flush and digest verification. Record each phase separately
where a composite operation has clone, boot, challenge, guest execution or flush.

Give each operation an immutable request ID and each boot an attempt/generation.
Record `passed`, `failed`, `unknown` or `blocked` with reason; never resubmit an
unknown operation under a new ID. Store raw operation IDs, monotonic timestamps,
phase durations, counters and observations privately. The private JSONL accepted
by `analyze.py` contains one numeric measurement per unique run/scenario/metric;
the `host` value is a digest of the full configuration, not just the model name.
Do not write a numeric row for a blocked feature. Report it in the coverage
manifest instead. Grouping different configurations is forbidden.

## 4. Measurement matrix

| Group | Exact workload and end observation | Samples and gate |
| --- | --- | --- |
| Local control | Registration, grant, status; durable result. | 100 each; status 1,000. P95 budgets in specification. |
| Disk setup | APFS independent working-disk clone and durable publication. | 100; P95 <= 1 s. Full-copy fallback reported separately. |
| Boot | Existing disk, create + boot, restart, first after host restart; authenticated fresh challenge. Keep three other VMs active except first-after-restart. | 100 each; five host restarts for first boot. Specification budgets. |
| Guest command | No-op `/bin/true` in a running guest; separately run the versioned project edit/check/build/export path. | 100 each; no-op P95 <= 100 ms. Project phases are diagnostic until a gate is specified. |
| Stop/version/reclaim | Forced stop to framework confirmation; stopped-VM rollback, branch and reclaim after artifacts are retained. | 100 each; stop has P95 <= 2 s and every sample <= 5 s. Unimplemented reclaim is blocked. |
| File I/O | 4 KiB upload + download with digest; 100 MiB each direction, sequential and flushed. | 100 roundtrips; 30 transfers per direction, P5 >= 100 MiB/s. Current client chunk limits do not substitute for the 100 MiB gate. |
| Footprint | Warm no-VM baseline, then one and eight idle VMs. Include manager, runner and attributable helpers. | 1 Hz for 60 s after 10 s idle; manager <= 32 MiB, per-VM increment <= 768 MiB, one-VM idle CPU <= 1% core. |
| Capacity | Four active VMs, then eight resident with four active; primary agent per VM, optional guest subagents inside the same limit. Keep host editor/browser workload reproducible. | Two hours each; sample memory pressure/swap every 10 s and health challenges. All correctness and stop gates still apply. |
| Recovery/security | At least 10 injections at each applicable durable boundary; invalid, stale, revoked and cross-VM grants. | Zero duplicate effects or unauthorized host actions; unknowns quarantined. No latency percentile can hide a failure. |

The full 100 create/execute/stop/destroy cycle gate remains blocked until safe
host-owned destroy exists. The same is true of mandatory NAT, root/non-root,
hostile guest-path and full-disk cases until the corresponding feature and
fault-injection fixture exist. Keep these rows in the coverage manifest rather
than silently omitting them.

## 5. Collection order and verdict

1. Verify artifact digests and free space. Run protocol checks and one real VM
   correctness pass. Save a private run manifest before timing.
2. Warm the host and guest. Run short control, guest and file cases with four
   active VMs; rotate the slot used for lifecycle tests. Preserve every sample.
3. Run one-VM and eight-VM footprint profiles, then the two capacity profiles.
   Stop every VM through the host and confirm the framework observation.
4. Run fault/security cases on disposable private state and a bounded test
   volume. Never fill the developer's system volume.
5. Analyze with `benchmarks/analyze.py`. A passing percentile requires its full
   sample count and zero failed/unknown cases. Include a coverage table for every
   F01–F11 and specification performance gate, with `passed`, `failed`, `unknown`
   or `blocked`, evidence digest and source revision.

CI runs only deterministic checks. Real VM timing is operator initiated and
does not gate ordinary PRs. A release candidate passes only when every mandatory
gate has real evidence on the reference M5 configuration, failures are resolved,
and the sanitized report is reviewed. Do not commit raw samples, paths, grant
tokens, images or guest output.
