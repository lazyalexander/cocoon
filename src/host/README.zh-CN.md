# host

[English（权威版本）](README.md) · 简体中文

可信宿主控制面。负责 SQLite 元数据、镜像和磁盘保管、授权、VM 生命周期、guest 请求准入、导出和恢复。数据库是私有宿主状态：SQLite WAL 与完整同步提交，不是 guest 或 agent API。一个主调用者获得绑定单个 VM 代际的限权授权；VM 内小 agent 共享该 VM 的 guest 权限，不获得宿主生命周期权限。

工作区依赖：platform、guest-wire、SQLite 与序列化。参见[架构](../../docs/architecture.zh-CN.md)与[贡献规范](../../CONTRIBUTING.zh-CN.md)。

## 边界

store.rs 管理数据库/schema，grants.rs 管理工作授权，api.rs 负责路由，lifecycle.rs 负责生命周期监督，versions.rs 负责镜像版本保管。未知执行结果须核对；guest 数据不能证明终止。
