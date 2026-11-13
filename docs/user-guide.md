# Cocoon user guide

English (authoritative) · [简体中文](user-guide.zh-CN.md)

[Project overview](../README.md) · [Acceptance specification](../SPECIFICATION.md) · [Architecture](architecture.md)

## 1. What you can use today

Cocoon is an experimental, Rust-first, headless microVM project for Apple Silicon
macOS. It provides trusted-host image import, persistent VM registration, real
start/stop/reconcile, an authenticated private Unix-socket API, guest exec/file
operations, binary artifact export, suspend/resume, approved cold recovery,
candidate approval, branch and rollback operations, and automatic host-protection
suspension.

The combined real-VM acceptance passes these operations on a pinned minimal
BusyBox/glibc userland built from Debian inputs. It is not a complete Debian
development distribution, a public network service, or a credential system for
untrusted host users. The same macOS account remains trusted. Capacity, latency,
destroy/cleanup, broad workload compatibility and full v0.1 fault qualification
remain open.

The [offline project experiment](../experiments/microvm/PROJECT.md) and fixed
persistence fixture remain separate acceptance paths. Use the managed acceptance
below for the current agent workflow.

## 2. Prerequisites and build

For host records and schema checks, use the repository's pinned Rust toolchain.
For real VM experiments, use Apple Silicon macOS with Xcode command-line tools,
Python 3, LLD and e2fsprogs. Prefer APFS for clone support. Keep adequate free disk
space for independent working images and private failure evidence.

Run commands from the repository root. Explicit setup commands, if needed:

```sh
rustup toolchain install 1.98.0 --profile minimal --component rustfmt --component clippy
rustup target add aarch64-unknown-linux-musl --toolchain 1.98.0
cargo build --locked -p cocoon --release
./tools/check
```

The CLI is `target/release/cocoon`; no installation into a system directory is
required. Production functionality does not run through Python, but current
build/acceptance scripts do require it. LLVM and e2fsprogs installation is not
performed by these scripts. The defaults are Homebrew `lld@22` and `e2fsprogs`;
explicit `COCOON_LLD` and `COCOON_MKE2FS` executable paths override them.

## 3. Validate a task without executing it

```sh
target/release/cocoon plan examples/task.json
```

This validates a bounded offline task and prints JSON. The example image does not
exist. `execution_available: false` and `execution_authorized: false` are expected;
a valid plan grants no authority and starts no VM.

## 4. Run the real persistence experiment

These commands explicitly download pinned third-party inputs when absent:

```sh
python3 experiments/microvm/fetch_kernel.py
python3 experiments/microvm/root_modules.py
```

Downloads are digest-checked; upstream changes cause refusal, not silent repinning.
Kernels and modules retain their original licenses. Build the signed native
adapter and fixed root image:

```sh
python3 experiments/microvm/probe.py build
python3 experiments/microvm/probe.py doctor
python3 experiments/microvm/build_root.py
```

`doctor` only reads capabilities. It does not boot a VM. The root builder prints
`private_output`: use that exact directory as `COCOON_FIXTURE` below. Do not
silently select the newest directory; old artifacts may use different modules.

```sh
COCOON_FIXTURE="/absolute/path/from/private_output"
python3 experiments/microvm/persistence.py \
  --fixture "$COCOON_FIXTURE" \
  --kernel experiments/microvm/build/debian/linux \
  --rounds 10
```

This creates a new private state directory for this run, then performs twenty
boots of one working disk: write and read for each of ten rounds. The fixture has
512 MiB guest RAM, a sparse 8 GiB logical disk, no network and no shared host
folders. It executes only its fixed program, not arbitrary commands.

Success requires `passed: true`, `boots: 20`, `unique_boots: true` and
`baseline_unchanged: true`. Every boot also requires framework stop confirmation.
`production_backend_qualified` remains false. The harness stops on the first
failure; do not automatically retry an unknown outcome.

Reports and SQLite records remain under ignored `experiments/microvm/runs/`;
images, manifests and binaries remain under ignored `experiments/microvm/build/`.
Do not publish these directories. See [experiment details](../experiments/microvm/PERSISTENCE.md)
for measurement boundaries, tool identity checks and known limitations.

## 5. Manage host records directly

This path lets you inspect the primitives without running the complete harness.
Choose a private absolute directory, outside version control:

```sh
COCOON_STATE="$HOME/Library/Application Support/Cocoon/manual-state"
mkdir -p "$COCOON_STATE"
chmod 700 "$COCOON_STATE"
COCOON_KERNEL="$(pwd -P)/experiments/microvm/build/debian/linux"
target/release/cocoon host "$COCOON_STATE" image import \
  "$COCOON_KERNEL" "$COCOON_FIXTURE/initrd.cpio.gz" "$COCOON_FIXTURE/root.raw" \
  "Debian kernel/modules: upstream licenses; original Cocoon fixture: MIT"
```

Use the returned image UUID, and choose one VM UUID. Reusing the same VM UUID with
identical registration parameters is idempotent; changing parameters is rejected.
The owner string is attribution, not authentication.

```sh
COCOON_IMAGE="UUID_FROM_IMAGE_IMPORT"
COCOON_VM="$(uuidgen)"
target/release/cocoon host "$COCOON_STATE" image show "$COCOON_IMAGE"
target/release/cocoon host "$COCOON_STATE" vm create "$COCOON_VM" "$COCOON_IMAGE" local-operator retain
target/release/cocoon host "$COCOON_STATE" vm list
target/release/cocoon host "$COCOON_STATE" vm show "$COCOON_VM"
target/release/cocoon host "$COCOON_STATE" operations
```

Registration normally returns `prepared`; that is not a running VM. Image import
records digests and supplied provenance, not boot qualification or publisher trust.
The complete command reference is:

| Arguments after `host STATE` | Effect |
| --- | --- |
| `image import KERNEL INITRD ROOT_RAW PROVENANCE` | Import private baseline copies; return image UUID. |
| `image show IMAGE_UUID` | Read the stored manifest. |
| `vm create VM_UUID IMAGE_UUID OWNER POLICY` | Create an independent work disk and registration. |
| `vm list` / `vm show VM_UUID` | Read records, not live VM reconciliation. |
| `operations` | Inspect import/create intents and completion; per-boot records are separate. |
| `vm start VM_UUID RUNNER` | Start a real managed VM with the explicit signed runner; return attempt/generation/lease. |
| `vm stop VM_UUID` / `vm reconcile VM_UUID` | Request confirmed stop or reconcile retained observations. |
| `vm emergency-stop VM_UUID` | Operator-only stop attempt for a running or uncertain execution. |
| `vm reclaim VM_UUID` / `vm reconcile-reclaim VM_UUID` | Operator-only reclaim after confirmed stop; explicitly finish an interrupted reclaim. |
| `vm reclaim-status VM_UUID` | Inspect `pending` or `complete` reclaim intent; a missing intent returns `null`. |
| `serve RUNNER` | Serve the authenticated private host socket with a fixed runner. |
| `experiment persistence VM_UUID RUNNER write\|read PAYLOAD` | One bounded fixture boot; RUNNER is an absolute ARM64 Mach-O path, PAYLOAD is 32 hexadecimal characters. |

For the last command, use the generated fixture, the signed `cocoon-vz` runner and
the same payload for the write/read pair. Prefer the full harness, which checks
all twenty results and baseline integrity. `vm list` is not a substitute for
independent termination evidence.

## 6. Errors, recovery and data handling

Successful CLI operations exit 0 and usually print JSON. Errors exit 2 with an
English message on stderr; a failed fixture can also print a JSON report on
stdout. Capture both and check the exit status.

| Symptom | Action |
| --- | --- |
| State directory permission error | Choose a private directory owned by your account and set mode 0700; do not relax it for other users. |
| Missing compiler target/tool, wrong tool identity | Install the declared target/tools or explicitly select the executable; do not substitute Android SDK tools through PATH. |
| Kernel/module digest mismatch | Keep the original file; review source/version changes. Do not disable checks or overwrite operator files. |
| `doctor` reports EPERM or framework unsupported | Distinguish sandbox denial from hardware support. Use an appropriately authorized macOS terminal for real tests; do not use native fallback. |
| Unknown VM state / reconciliation required | Preserve original records, observations and disk; run reconcile. Never clear state or reuse a disk to force a retry. |
| Snapshot recovery required | Review the failed snapshot and explicitly approve cold recovery from the persistent working disk; automatic cold recovery is not attempted. |

Stopping does not mean deleting. The operator can reclaim a confirmed-stopped VM
with `vm reclaim`; exported files move to `STATE/artifacts/VM_UUID/ATTEMPT_UUID/`,
and the VM's working directory is removed. A failed reclaim retains a durable
intent and blocks new boots; inspect it with `vm reclaim-status` and use
`vm reconcile-reclaim` explicitly.
The VM UUID remains reserved. Do not manually delete state as reconciliation.
Keep working disks and private reports out of commits and support attachments;
share sanitized summaries instead.

## 7. Managed agent acceptance

Build the helper-enabled fixture and run the combined real-VM path:

```sh
python3 experiments/microvm/root_modules.py --project
python3 experiments/microvm/build_root.py --project
./tools/build runner
cargo build --locked --release -p cocoon
python3 experiments/microvm/production_acceptance.py \
  --fixture /absolute/private/root-BUILD_ID \
  --kernel /absolute/private/linux --automatic-suspend
```

Add `--corrupt-before-resume` to exercise real snapshot corruption, explicit cold
recovery approval, and subsequent automatic suspension. The harness creates a
private state directory and starts the socket service. The primary agent imports
a small directory of knowledge and code through its scoped API, edits a guest
script, checks and builds it, and byte-checks the exported result. The harness
also exports a 12,000-byte binary artifact, suspends/resumes with new identity,
saves/promotes a candidate, boots a branch, stops it, rolls back the original
VM, then reclaims both stopped VMs while retaining the exported project result.
It does not publish credentials or raw reports.

## 8. Run a project as the primary agent

After the operator starts a managed VM and issues a work grant with `inspect`,
`guest`, and `export` actions, the primary agent can use
[`examples/agent_project.py`](../examples/agent_project.py). The operator supplies
the private state path, a mode-0600 grant token file, and a mode-0600 JSON file
containing the successful `start` result. The caller must match the grant subject.

```sh
python3 examples/agent_project.py \
  --state /absolute/private/state --caller agent \
  --grant-file /absolute/private/grant \
  --authority-file /absolute/private/start.json \
  --project /absolute/private/project \
  --command-json '["/bin/sh","src/check.sh"]' \
  --command-json '["/bin/sh","src/build.sh"]' \
  --artifact result.txt --output-name result.txt
```

`--project` imports regular files recursively into `/workspace`; it rejects
symlinks and limits one import to 64 files, 32 MiB total, and 16 MiB per file.
Empty directories are not imported. `--input` can add individual files at the
workspace root when their names do not collide with project root entries.
Commands run in order and stop on the first failed guest exit.
The fixture provides a minimal shell, not an arbitrary language toolchain. The
export remains in host-owned private state; the operator handles VM stop and
reclaim. After an uncertain response, use `--status-id` with
the original operation UUID to reconcile it before taking further action.
