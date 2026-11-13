# guest-helper

[English（权威版本）](README.md) · 简体中文

Linux guest 操作。在 Linux ARM64 VM 内运行，通过 vsock 按 guest 权限提供文件和进程操作；不提供宿主注册、授权或回收操作。

工作区依赖：guest-wire、libc。参见[架构](../../docs/architecture.zh-CN.md)与[贡献规范](../../CONTRIBUTING.zh-CN.md)。

## 边界

guest 可以修改自身用户态，包括此 helper。宿主注册、所有权、终止判断及产物发布必须留在 guest 外。
