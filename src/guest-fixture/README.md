# guest-fixture

English (authoritative) · [简体中文](README.zh-CN.md)

Fixed Linux acceptance init. A minimal PID 1 used by experimental boot and persistence acceptance. It is not the general production guest agent or a full Debian userland.

Workspace dependencies: none. See [architecture](../../docs/architecture.md) and [contribution rules](../../CONTRIBUTING.md).

## Boundary

Keep fixed acceptance modes explicit. A passing fixture is evidence for that workload only, not full Debian or production service qualification.
