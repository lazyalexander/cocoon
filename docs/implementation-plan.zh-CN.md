# 原生 agent microVM 实施计划

[English（权威版本）](implementation-plan.md) · 简体中文

## 产品边界

Cocoon 基于 Apple Virtualization.framework，在 Apple Silicon 上运行无头
Linux ARM64 guest。guest root 可以修改或破坏整个 guest；注册、归属、授权、
租约、恢复和销毁始终由可信宿主掌握，guest 报告不构成授权。

agent 通过受控工具接入；以管理器同一 macOS 用户任意执行宿主代码不在隔离保证内。
不配置 GPU、显示、音频、USB、物理设备直通和宿主共享目录。默认离线；显式 NAT
授权可能允许访问本地服务。撤销必须确认停止后才完成，禁止原生执行回退。

## 架构与交付门槛

1. **M0：契约。** 分离 VM、启动实例、磁盘、授权、任务与尝试身份，同步双语文档。
2. **M1：真实 VM。** 验证兼容内核启动、输入输出、超时和取消，再加入精简 Debian
   ARM64 glibc 根文件系统与可写磁盘。
3. **M2：宿主生命周期。** Rust 管理器、SQLite、私有 Unix socket、CLI 和每次启动
   独立 runner；Rust 负责监督与协议，Swift 仅限必要的 Apple 框架适配；副作用前持久化意图，未知结果核对而非自动重跑。
4. **M3：agent 接入。** 编译型 vsock helper 提供有界命令输出和文件传输；绑定 VM
   的可撤销凭据不进入 guest。杀死 helper 不得阻止宿主终止。
5. **M4：资格验证。** 测试崩溃重启、磁盘满、输出洪泛、所有权丢失、撤销和镜像
   发布失败。至少 30 次独立启动，报告所有失败，区分冷启动和重复启动。

不可变镜像关联固定内核、initramfs、根磁盘及来源；工作磁盘优先 APFS 克隆，
否则显式完整复制。默认保留；回滚从基线重建；分支在确认停止后产生不可信派生镜像。
任务完成由宿主确认，不采信 guest 标记；不做运行中快照。默认 1 vCPU、512 MiB、
8 GiB 稀疏磁盘，需真实验收。统计内存包括框架辅助进程，不把 vCPU 数量当作 CPU
时间配额，也不宣称等价 Firecracker。

## 当前实现状态

宿主基础层已实现 SQLite 记录、带 SHA-256 校验的只读镜像副本、来源记录、独立
工作磁盘、按调用方指定 UUID 幂等注册，以及待核对操作查询。目前优先 APFS 克隆，
否则保留稀疏分配，并记录逻辑与分配容量。已注册 VM 默认 1 vCPU、512 MiB、离线。

候选保存、批准、回滚和分支已是可执行宿主操作。输入不会自动获得可启动 Debian
资格；来源由调用方提供，不代表发布者认证。

可信宿主 CLI：

```text
cocoon host ABSOLUTE_PRIVATE_STATE_DIR image import KERNEL INITRD ROOT_RAW PROVENANCE
cocoon host ABSOLUTE_PRIVATE_STATE_DIR image show IMAGE_UUID
cocoon host ABSOLUTE_PRIVATE_STATE_DIR vm create VM_UUID IMAGE_UUID OWNER retain
cocoon host ABSOLUTE_PRIVATE_STATE_DIR vm list
cocoon host ABSOLUTE_PRIVATE_STATE_DIR vm show VM_UUID
cocoon host ABSOLUTE_PRIVATE_STATE_DIR operations
```

状态目录须为私有目录（0700）。CLI 面向可信操作员，不是经过授权的 agent 接口。
同一 macOS 用户可以绕过文件权限，所以这些副本不防该用户篡改。失败操作保留为
pending 供检查；`vm reconcile` 已支持显式核对。

当前管理链路支持真实 `vm start/stop/reconcile`、私有认证 socket、guest 文件与命令、
二进制产物、挂起/恢复、批准的冷恢复、候选/分支/回滚及主动保护。正式 runner 仍有
期限上限；销毁、完整容量和全量故障资格未完成。固定实验继续独立保留。

提交前运行 Rust 测试、Clippy 严格检查、格式检查、Python 测试和原生协议检查。
真实 macOS VM 验收通过前不宣称后端可用。不公开凭据、私有报告和操作员路径；发布
需要明确授权。

M1 无磁盘进展：固定 Debian 内核在 128 MiB 下通过输入输出、期限停止和取消，
30 次重复输入输出全部通过。测量边界见实验说明；M1 完整 Debian 用户空间验收仍待完成。

里程碑完成以 [v0.1 验收规范](../SPECIFICATION.zh-CN.md) 为准。持久根磁盘发布门槛
取代探索阶段至少 30 次启动的要求；无磁盘实验不等于发布验收。

参考交付为 M5 / 16 GB，八台默认 VM 常驻、四台轻量 agent 工作活跃。先验证四台
活跃，再验证八台常驻。规范区分强制延迟预算与可选优化目标、容量估算与实测支持。
重负载数量仅供规划；磁盘克隆和本地传输指标不代表完整镜像发布或互联网操作耗时。

固定 Rust ext4 根磁盘 fixture 已通过十轮持久化（二十次启动），包含权限、根与
执行文件检查和确认停止。Debian 用户空间、生产 helper 及完整 v0.1 验收仍待完成。

下一步离线闭环已实现：Rust guest-wire crate 与 Linux helper、固定来源的最小 shell
用户态、vsock 通道和受控产物接收。三次真实固定项目启动通过，见
[项目实验](../experiments/microvm/PROJECT.zh-CN.md)。这不代表生产 helper、外部授权、
完整 Debian、恢复或回滚/分支验收已完成。

## 实验性快照平台证据

在 macOS 27.0.1（26A434）上，固定 512 MiB Rust ext4 guest 的三个真实
Virtualization.framework 跨进程保存/恢复样本全部通过。每个样本均保存状态、确认
原 runner 停止、由一个新进程恢复、匹配仅存在于 RAM 的随机 nonce 并将计数器恰好
递增一次、在恢复准入时匹配工作磁盘 SHA-256，并确认最终停止。状态文件为
24,121,344 字节。保存进程总耗时 5,505.2～5,561.0 ms；恢复进程总耗时
1,206.9～1,219.9 ms，其中包含固定的一秒 guest 等待；框架恢复本身耗时
175.3～187.0 ms。这些不是生产预算或完整内存统计。

实验将通用机器标识与状态文件一起持久化并在恢复时重用。它不提供生产检查点/WAL
发布、租约、恢复、主动挂起或并行恢复。下一项独立实施门槛是将宿主自有的检查点和
WAL 保管接入生产生命周期与租约模型。在显式加入强制验收标准前，运行中内存快照仍
不属于 v0.1 资格范围。
