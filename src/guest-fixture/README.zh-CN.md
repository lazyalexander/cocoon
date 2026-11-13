# guest-fixture

[English（权威版本）](README.md) · 简体中文

固定 Linux 验收 init。实验启动与持久化验收使用的最小 PID 1；不是通用生产 guest agent，也不是完整 Debian 用户态。

工作区依赖：无。参见[架构](../../docs/architecture.zh-CN.md)与[贡献规范](../../CONTRIBUTING.zh-CN.md)。

## 边界

固定验收模式必须明确。fixture 通过仅证明该工作负载，不代表完整 Debian 或生产服务通过资格。
