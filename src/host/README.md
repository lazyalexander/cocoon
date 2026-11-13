# host

English (authoritative) · [简体中文](README.zh-CN.md)

Trusted host control plane. Owns SQLite metadata, image and disk custody, authorization, VM lifecycle, guest request admission, export and recovery. The database is private host state: SQLite WAL with full synchronous commits, not a guest or agent API. One primary caller receives a scoped grant for one VM generation. Guest subagents share that VM and its guest permissions; they do not gain host lifecycle authority.

Workspace dependencies: platform, guest-wire, SQLite and serialization. See [architecture](../../docs/architecture.md) and [contribution rules](../../CONTRIBUTING.md).

## Boundary

Keep store/schema in store.rs, work grants in grants.rs, API routing in api.rs, lifecycle supervision in lifecycle.rs, host-owned reclaim intents and artifact retention in reclaim.rs, and image/version custody in versions.rs. Unknown execution outcomes require reconciliation; guest data never proves termination. Reclaim requires confirmed stop evidence and cannot be authorized by a work grant.
