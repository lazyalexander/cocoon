# 构建与检查入口

[English（权威版本）](README.md) · 简体中文

`tools/build check` 快速检查固定 Rust workspace；`tools/build release` 构建
release；`tools/build guest` 用显式目标与 linker 交叉构建 Linux ARM64 fixture；
`tools/build runner` 在被忽略的 `target/` 下编译并签名最小 macOS 适配层，
需要 macOS。
`tools/check` 执行文档链接、Rust 格式/测试/Clippy 与 Python 验收脚本测试。
生产 VM 压测不进入 CI，须在注明型号的真实 Mac 上用私有输入运行。
CI 的 Linux 源码检查与 macOS runner 协议检查独立并行，取消过时运行，
不自动部署。发布或 CD 需另外明确决定。
