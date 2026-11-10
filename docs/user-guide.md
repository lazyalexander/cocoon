# Cocoon user guide

English (authoritative) · [简体中文](user-guide.zh-CN.md)

[Project overview](../README.md) · [Acceptance specification](../SPECIFICATION.md) · [Architecture](architecture.md)

## 1. What you can use today

Cocoon is an experimental, Rust-first, headless microVM project for Apple Silicon
macOS. It currently provides trusted-host image import, persistent VM registration,
task-schema validation and a fixed Rust guest persistence experiment.

The experiment boots an ext4 root disk, writes a payload, confirms host stop and
boots again to verify that payload and permissions. Ten write/read cycles have
passed on the development host. This is not a general-purpose Linux shell or an
agent execution service. `vm start` intentionally reports execution unavailable.
The M5 / 16 GB capacity and latency figures in the specification are future
acceptance targets, not current supported capacity.

There is no public socket API, grant command, general exec/file API, automatic
reconciliation, destroy command or automatic rollback/branch execution yet.
`retain`, `rollback` and `branch` are accepted stored policies; only the fixed
experiment has demonstrated retention. CLI access is trusted host authority,
not a credential-scoped interface for untrusted agents.


The [offline project experiment](../experiments/microvm/PROJECT.md) additionally
provides a minimal BusyBox shell and experimental Rust vsock read/write/exec
helper. Three real project boots passed. Follow its separate build/run instructions;
public agent admission and general project inputs remain unavailable.

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
| `vm start VM_UUID` | Explicit unavailable error; no fallback execution. |
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
| Unknown VM state / reconciliation required | Preserve the original VM/boot records, runner observations and disk. Automatic recovery is not implemented; do not clear state or reuse the disk to force a retry. |
| `vm start` unavailable | Expected for the current release; use only the documented fixed experiment. |

Stopping does not mean deleting. There is no supported automatic uninstall/VM
cleanup command yet. Before manually removing any private state or build data,
establish that no VM or process still owns it and retain any required evidence.
Never use deletion as reconciliation. Read-only queries do not grant cleanup
permission. Keep working disks and private reports out of commits and support
attachments; share sanitized summaries instead.
