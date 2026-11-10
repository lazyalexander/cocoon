# Rust 优先架构

[English（权威版本）](architecture.md) · 简体中文

## 参考与适配

参考本地 Firecracker 提交
[`7a43eb7a20b34c4dbcc31574834810855413a58e`](https://github.com/firecracker-microvm/firecracker/tree/7a43eb7a20b34c4dbcc31574834810855413a58e)
的 workspace、`src/firecracker`、`src/vmm`、设计文档、贡献指南、工具链和依赖策略。
本次没有复制上游源码。Firecracker 为 Apache-2.0；未来复制代码必须保留原许可和
适用声明，不能改标 MIT。

Firecracker 把宿主 API 与 VMM 事件循环分开，每个进程拥有一个 microVM。Cocoon
借鉴职责边界，不照搬 Linux 的 KVM、epoll、seccomp、jailer、TAP 或 musl 假设。
Virtualization.framework 下由 Apple 执行 vCPU/设备，不能声称 Cocoon 实现了
Firecracker 的 VMM 内部结构或等价隔离。原生 runner 仍是实验。

## 已实现 workspace

| Crate | 职责 | Cocoon 内部依赖 |
| --- | --- | --- |
| `cocoon` | CLI 解析与宿主输出 | runtime、host |
| `cocoon-runtime` | 平台无关任务校验与尝试状态 | 无 |
| `cocoon-host` | SQLite、镜像保管、注册及实验监督 | platform、guest-wire |
| `cocoon-platform` | 最小原生文件系统 FFI 与稀疏复制 | 无 |
| `cocoon-guest-fixture` | 固定 Linux ARM64 根磁盘验收 | 无 |
| `cocoon-guest-wire` | 类型化有界实验 guest 协议 | 无 |
| `cocoon-guest-helper` | Linux vsock 和 guest 文件/命令操作 | guest-wire |

SQLite 和文件保管不属于契约 crate。guest helper 不得依赖宿主管理 crate。
所有成员均为默认成员，根目录检查覆盖全部实现。CLI 命令保持不变；Rust 调用方从
`cocoon_runtime::manager` 改为 `cocoon_host`，这是发布前有意进行的源码 API 调整。

## 语言及后续后端边界

生产 CLI、宿主服务、授权、生命周期、核对、镜像管理、runner 协议和未来 guest
helper 默认使用 Rust。Swift 仅用于现有 Apple 框架实验及仍有必要的最小平台适配层。
Python 用于开发、镜像获取与验收；C 当前仅提供最小启动测试程序。Python 和 Swift
均不继续扩展生产授权或注册逻辑。

持久根磁盘实验现已将 Rust 进程监督与 Apple 框架调用分离。原生文件系统 FFI 和
Linux fixture 系统调用隔离在各自 crate，控制面继续禁止 unsafe。固定 fixture
不是未来通用 vsock helper。限制见[持久化实验](../experiments/microvm/PERSISTENCE.zh-CN.md)。

宿主策略独立于单 VM 执行路径。guest 传输即使 VM ID 匹配也不可信。传输丢失或
runner 死亡不证明终止；未知操作必须核对。

## 工具和治理

采用 Cargo workspace、集中依赖版本、继承 lint、提交锁文件、固定工具链、显式
release profile 和统一本地检查入口。CI 使用固定版本检查整个 workspace，macOS
协议测试与真实 VM 验收分开。继续排除私有基准文件。审阅、依赖、来源和发布规则见
[贡献指南](../CONTRIBUTING.zh-CN.md)。

## 实验性项目链路

`cocoon-guest-wire` 包含严格 guest 消息类型和有界帧；`cocoon-guest-helper`
只依赖该 crate 与 libc，不依赖宿主管理。宿主新增 guest-wire 依赖以生成和核验
固定工作流。Swift ProjectChannel 仅传输字节，Rust 负责核验以及确认停止后的
产物接收。见[项目实验](../experiments/microvm/PROJECT.zh-CN.md)。
