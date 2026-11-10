# Cocoon

[English（权威版本）](README.md) · 简体中文

Cocoon 是面向智能体工作负载设计的独立执行运行时项目。首个目标是在
Apple Silicon macOS 上无头运行 Linux ARM64。an-agent 是预期的首个使用者，
不属于运行时依赖。

**当前状态：任务契约原型，以及已经编译的最小 macOS VM 实验。
真实 Linux guest 启动仍被框架配置校验阻塞；尚无通过资格验收的生产执行后端。**

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
所有 guest 验收仍未通过。

较早的空上下文预检保留在 `experiments/macos-preflight/`。
两个实验均不依赖 an-agent，也不会启动现有 Docker 或 Lima 虚拟机。

## 下一阶段验收

查明 Virtualization.framework 不可用的原因，选择最小 Linux ARM64 内核，
证明一个归属明确的 VM 能启动、离线运行、返回固定输入的转换结果并在期限到达后停止。
当前实验直接使用 Virtualization.framework；OCI 管理延后。
通过这些验收后，再接入真实准入、持久监督、故障对账与产物保管机制。
选择 VM 执行后，不允许自动退回宿主原生执行。

模型调用、对话、规划和多智能体协调由使用方负责。
GPU、GUI guest、实时内存快照及其他宿主平台不属于首阶段交付范围。

## 协议

Cocoon 原创源码与文档采用 [MIT 协议](LICENSE)。用户提供的 Linux 内核和
第三方依赖保留各自协议，不会因使用 Cocoon 而变更协议。
