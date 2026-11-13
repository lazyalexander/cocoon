# runtime

English (authoritative) · [简体中文](README.zh-CN.md)

Platform-neutral task contracts. Defines task, caller, attempt, limits and transition rules. No filesystem, SQLite, macOS framework or guest transport dependency belongs here.

Workspace dependencies: none. See [architecture](../../docs/architecture.md) and [contribution rules](../../CONTRIBUTING.md).

## Boundary

Contract changes must preserve separate caller, task, attempt and grant concepts. Unit transitions never prove a VM actually ran or stopped.
