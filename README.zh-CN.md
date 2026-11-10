# Cocoon

[English（权威版本）](README.md) · 简体中文

[使用说明书](docs/user-guide.zh-CN.md) · [English user guide](docs/user-guide.md)

Cocoon 是面向智能体工作负载设计的独立执行运行时项目。首个目标是在
Apple Silicon macOS 上无头运行 Linux ARM64。an-agent 是预期的首个使用者，
不属于运行时依赖。

**当前状态：真实管理链路已支持启动/停止、经认证的私有 Unix socket 控制、guest
文件/命令操作、二进制产物导出、跨进程挂起/恢复、经批准的冷恢复、候选版本、分支、
回滚和策略驱动的主动挂起。组合真实 VM 验收已通过。这仍不等于完成 v0.1 资格、
对抗性隔离、容量验收或完整 Debian 工具链。**

## 模型

- 调用者可以是智能体、用户或服务；身份归属信息不等于身份认证。
- 任务描述固定版本的可执行内容、参数和所请求的资源限制。
- 执行尝试表示任务的一次运行；重试必须产生新身份。
- 授权由宿主准入机制负责，guest 指令不能扩大权限。
- 停止请求、执行所有者失联和确认终止是不同状态。

Rust 库校验一个范围有限的离线任务配置，并在内存中建模执行状态转换。
它不持久化任务、不认证调用者、不强制限制资源、不监督进程，也不接收产物。
生命周期方法供可信宿主调用，不是接收 guest 报告的端点。
执行身份匹配只能检查归属，不能证明执行已终止；后续后端必须独立确认 VM
的归属与终止，再调用这些方法。

## 本地开发

```sh
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
cargo fmt --all -- --check
cargo run --offline -- plan examples/task.json
```

`plan` 最多读取 64 KiB，拒绝未知字段和不支持的联网请求，并始终报告执行不可用。
示例使用刻意不存在的镜像身份，仅用于展示格式，不能启动 guest。
固定内容摘要不等于认证发布者；计划也不是授权令牌。

最小启动实验见
[`experiments/microvm/README.zh-CN.md`](experiments/microvm/README.zh-CN.md)。
它构建不依赖标准运行库的 ARM64 Linux init，以及经过优化的原生框架启动器，
不依赖 OCI 管理包。实验包含输入输出、期限停止验收与重复启动基准工具。
无磁盘与固定持久根磁盘验收已通过，生产资格验收仍待完成。

无须内核的 `doctor` 命令将能力信息读取错误与硬件支持情况分开记录。
原生启动器协议检查在 macOS CI 上运行，不启动 guest。取消已通过有界无磁盘实验；恶意输出和生产监督仍须单独验收。

较早的空上下文预检保留在 `experiments/macos-preflight/`。
两个实验均不依赖 an-agent，也不会启动现有 Docker 或 Lima 虚拟机。

[离线项目实验](experiments/microvm/PROJECT.zh-CN.md) 已在真实 VM 中验证 Rust vsock
文件操作、shell 代码修改/执行和有界产物接收，连续三次启动通过。它尚不是
外部 agent 服务或完整 Debian 开发镜像。

## 下一阶段验收

剩余工作是更广泛的资格验收：重复生命周期/版本循环、崩溃与资源故障注入、完整
工作负载兼容性、容量及性能预算。管理链路继续使用 Virtualization.framework；
OCI 管理延后。选择 VM 执行后，不允许自动退回宿主原生执行。

模型调用、对话、规划和多智能体协调由使用方负责。
GPU、GUI guest 及其他宿主平台不属于首阶段交付范围。运行中内存快照已有真实实验
支持，但尚未覆盖全部 v0.1 资格门槛。

## 协议

Cocoon 原创源码与文档采用 [MIT 协议](LICENSE)。用户提供的 Linux 内核和
第三方依赖保留各自协议，不会因使用 Cocoon 而变更协议。

## 原生 agent microVM 方向

参见[实施计划与当前交付状态](docs/implementation-plan.zh-CN.md)。可信宿主 CLI 支持
注册与真实生命周期操作，私有 socket 服务另提供经认证的 agent 操作。

[Rust 优先 workspace 架构与贡献流程](docs/architecture.zh-CN.md) · [Contributing](CONTRIBUTING.zh-CN.md)

[v0.1 验收规范](SPECIFICATION.zh-CN.md) 定义强制门槛和开发终点；目标尚不是已实现保证。
