# cocoon

[English（权威版本）](README.md) · 简体中文

可信命令行入口。解析 plan 与可信宿主命令，将状态和执行交给 cocoon-host；不实现 VM 设备或 guest 代码。

工作区依赖：runtime、host。参见[架构](../../docs/architecture.zh-CN.md)与[贡献规范](../../CONTRIBUTING.zh-CN.md)。

## 边界

只保留解析与输出，宿主权限决策交给 cocoon-host。CLI 是可信操作者入口，不是无需授权的 agent 端点。
