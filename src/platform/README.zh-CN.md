# platform

[English（权威版本）](README.md) · 简体中文

最小 macOS 文件系统适配。包含原生 clonefile 边界与稀疏磁盘复制回退，不负责授权或 VM 生命周期。

工作区依赖：无。参见[架构](../../docs/architecture.zh-CN.md)与[贡献规范](../../CONTRIBUTING.zh-CN.md)。

## 边界

原生 unsafe FFI 仅留在本 crate。调用方请求复制并取得实际复制方式；复制成功不代表隔离已成立。
