# host

[English（权威版本）](README.md) · 简体中文

可信宿主控制面。负责 SQLite 元数据、镜像和磁盘保管、授权、VM 生命周期、guest 请求准入、导出和恢复。数据库是私有宿主状态：SQLite WAL 与完整同步提交，不是 guest 或 agent API。一个主调用者获得绑定单个 VM 代际的限权授权；VM 内小 agent 共享该 VM 的 guest 权限，不获得宿主生命周期权限。

工作区依赖：platform、guest-wire、SQLite 与序列化。参见[架构](../../docs/architecture.zh-CN.md)与[贡献规范](../../CONTRIBUTING.zh-CN.md)。

## 边界

store.rs 管理数据库/schema，grants.rs 管理工作授权，api.rs 负责路由，lifecycle.rs 负责生命周期监督及 runner 身份证据，reclaim.rs 负责宿主回收意图与产物保留，versions.rs 负责镜像版本保管。未知执行结果须核对；guest 数据不能证明终止。回收要求确认停止证据，工作 grant 无法授权回收。

手动维护通过 `image-maintenance-info` 和 `image-inventory` 提供本机操作员查询。
`image-request` 支持 `retire`（镜像及摘要）和 `reclaim_vm`（VM、代次及版本），
沿用镜像写入认证。存活引用、未知操作及未核实的终止均阻止清理。镜像清理保留
元数据和回执；删除中断只允许原操作 `image-reconcile`。持久删除意图之后的取消
无法恢复已删除字节。VM 回收保留导出产物；分配量包含共享块估算，不保证等量
空间释放。不启用自动 GC。
