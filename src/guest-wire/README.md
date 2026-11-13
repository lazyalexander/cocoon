# guest-wire

English (authoritative) · [简体中文](README.zh-CN.md)

Bounded guest message protocol. Defines typed requests, responses, correlation identities and length-prefixed framing. A matching identity is correlation, not authorization.

Workspace dependencies: serde, serde_json. See [architecture](../../docs/architecture.md) and [contribution rules](../../CONTRIBUTING.md).

## Boundary

This crate never accepts grant tokens or decides host permissions. Bound every frame and payload before allocation or forwarding.
