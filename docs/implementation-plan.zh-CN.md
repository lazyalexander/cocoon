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
   独立的 Swift runner；副作用前持久化意图，未知结果核对而非自动重跑。
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

宿主基础层已实现 SQLite 记录、带 SHA-256 校验的只读镜像副本、来源记录、独立完整
复制的工作磁盘、按调用方指定 UUID 幂等注册，以及待核对操作查询。目前完整复制
不保证保留稀疏分配，也尚未使用 APFS 克隆。已注册 VM 默认 1 vCPU、512 MiB、离线。

`retain`、`rollback`、`branch` 目前仅存储，尚未执行。输入未验证为可启动 Debian
镜像；来源由调用方提供，不代表发布者认证。

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
pending 供检查；自动核对尚未实现。

`vm start VM_UUID` 明确返回执行不可用，不启动宿主进程或 VM。socket 服务、授权、
租约、磁盘恢复、guest helper、生产 runner 集成、镜像构建器和真实 VM 验收仍未完成。
现有无磁盘实验独立保留。

提交前运行 Rust 测试、Clippy 严格检查、格式检查、Python 测试和原生协议检查。
真实 macOS VM 验收通过前不宣称后端可用。不公开凭据、私有报告和操作员路径；发布
需要明确授权。

M1 无磁盘进展：固定 Debian 内核在 128 MiB 下通过输入输出、期限停止和取消，
30 次重复输入输出全部通过。测量边界见实验说明；M1 持久根磁盘验收仍待完成。
