# runtime

[English（权威版本）](README.md) · 简体中文

平台无关任务契约。定义任务、调用者、执行尝试、限制和状态转移；这里不依赖文件系统、SQLite、macOS 框架或 guest 通信。

工作区依赖：无。参见[架构](../../docs/architecture.zh-CN.md)与[贡献规范](../../CONTRIBUTING.zh-CN.md)。

## 边界

契约变更必须区分 caller、task、attempt 与 grant。内存状态转移不能证明 VM 实际运行或停止。
