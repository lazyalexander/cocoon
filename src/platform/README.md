# platform

English (authoritative) · [简体中文](README.zh-CN.md)

Narrow macOS filesystem adapter. Contains the native clonefile boundary and sparse disk-copy fallback. It does not own authorization or VM lifecycle.

Workspace dependencies: none. See [architecture](../../docs/architecture.md) and [contribution rules](../../CONTRIBUTING.md).

## Boundary

All unsafe native FFI stays inside this crate. Callers request a copy and receive the observed copy method; they must not infer isolation from a successful copy.
