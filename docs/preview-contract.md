# Preview contract for an-agent integration

English (authoritative) · [简体中文](preview-contract.zh-CN.md)

This document describes the development preview control plane. It is not v0.1
qualification. Passing these operations does not prove isolation, capacity, or
the budgets in `SPECIFICATION.md`.

Cocoon remains independent of an-agent. A trusted adapter converts an-agent
authorization into a Cocoon work grant. Cocoon does not interpret spool rules,
silk/kit labels, or business acceptance. An exported file is a candidate, not
an accepted artifact. A stopped VM is not a successful business task.

## Credentials

The state-directory management credential authorizes operator actions: image
import and publication, VM creation, protection policy, branch, rollback,
candidate promotion, and grant issue or revocation.

Work methods require a work grant. The management credential is not a substitute.
A grant binds a subject, one VM, the VM generation, an action set, an expiry,
and a maximum export size. Actions are `inspect`, `guest`, `export`, and `stop`.
One execution attempt occupies one VM. A grant cannot register a VM, transfer
it, publish an image, change protection, or act on another VM.

Derived grants may only narrow the parent: fewer or equal actions, an earlier
or equal expiry, a smaller or equal export limit, and the same VM and
generation. Revoking a parent rejects its children. The token is returned once
and stored only as a SHA-256 digest. It is not written to the journal, guest
environment, or command arguments. A caller string, task id, or VM id is not
authorization.

## Operation identity

`HostRequest.request` is the consumer operation id for start, stop, guest, and
export. The same id and the same parameter digest return the saved result and
do not execute again. The same id with a different digest is rejected. The
digest covers the operation and the grant id, not the token or management
credential.

If the host records intent and exits before the result is committed, a later
query reports `unknown`. Repeating the id does not execute it. Unknown is not
success. Query with `operation_status` after a timeout instead of submitting a
new id. Disk reuse, rollback, and deletion remain forbidden while a result is
unknown.

## Control and work lanes

Stop, emergency stop, grant revocation, and operation query are not queued
behind guest execution, export, start, or resume. Memory protection runs on
its own timer. Connections, queued requests, read time, and frame size are
bounded. The frame limit remains 32 KiB.

Stop, cancellation, suspend, resume, and cold recovery are different
operations. A guest exit code, a runner state, and framework stop evidence
stay separate. vCPU count is not a CPU-time quota. Configured guest memory is
not the host's total footprint. Unsupported capabilities are rejected. There
is no native-execution fallback.

Consumers use the `cocoon-host` request types. They do not read SQLite, the
disk layout, or the Swift runner protocol.

## Primary agent context

A work grant with `inspect` may call `context` for its VM. The versioned result
contains current VM state, active attempt and generation, configured CPU/memory
and network state, permitted work actions, export limit and grant expiry. It
contains no host paths, management credential or grant token. The primary caller
owns the host request identity. Small agents may run inside its VM and share its
guest permissions; they do not receive independent host lifecycle authority.
A guest observation cannot enlarge the grant.
