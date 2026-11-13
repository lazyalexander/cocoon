# cocoon

English (authoritative) · [简体中文](README.zh-CN.md)

Trusted command-line entry point. Parses plan and trusted host commands. It delegates state and execution to cocoon-host. It does not implement VM devices or guest code.

Workspace dependencies: runtime, host. See [architecture](../../docs/architecture.md) and [contribution rules](../../CONTRIBUTING.md).

## Boundary

Keep parsing and presentation here; forward host authority decisions to cocoon-host. The CLI is a trusted operator entry point, not a grant-free agent endpoint.
