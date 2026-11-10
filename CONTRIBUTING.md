# Contributing

English (authoritative) · [简体中文](CONTRIBUTING.zh-CN.md)

Keep changes small and independently reviewable. Describe the problem, resulting
behavior, validation and remaining uncertainty in each pull request. Architecture
or authority-boundary changes require a design discussion before expanding the
implementation. Repository owners review and authorize merges and publication;
no automated approval or maintainer quorum is implied.

Run `tools/check` before proposing a change. Native runner changes additionally
require compilation, signing and protocol tests on macOS. Execution, isolation,
resource enforcement and termination claims require real VM acceptance, separately
from unit tests. Never commit operator images, credentials or private reports.
English documentation is authoritative; update its Simplified Chinese companion.
Code, comments and CLI messages remain English.

## Rust and dependencies

The checked-in Rust toolchain and workspace Cargo.lock are authoritative. CI uses
that same toolchain and locked dependencies. Update the toolchain, CI selection
and lockfile deliberately together when needed, and record validation.

Declare shared dependencies once in the root workspace; members inherit them.
Use path dependencies for internal crates. Prefer crates.io releases over Git
sources; any Git source requires an immutable revision and documented reason.
A dependency change must explain its purpose, enabled features, license, source,
binary/runtime cost and relevant security advisories. Do not add a new crate just
to mirror an upstream directory. Publish remains disabled for workspace packages.

Unsafe code is forbidden in current control-plane crates. A future platform FFI
adapter must isolate unsafe code in its own crate with documented safety contracts
and explicit review; do not relax the workspace policy globally. Release builds
use abort-on-panic and LTO while retaining symbols for diagnosis.

Firecracker's dependency-audit practices are a reference, but Cocoon does not yet
have an automated cargo-deny/advisory gate. Do not describe manual dependency
review as an automated security audit. Third-party source additions must retain
original licenses and provenance; Cocoon's MIT license does not replace them.
