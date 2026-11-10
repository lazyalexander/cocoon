# macOS VM 第一阶段预检

[English（权威版本）](README.md) · 简体中文

本目录包含真实 Apple 框架预检。它不修改产品代码，不启动现有 Lima 或 Docker
虚拟机，不安装服务，也不授予 spool 执行权限。所选内核仅用于验证 VM 配置；
不会启动 guest，也不会打开 guest 磁盘。

在配有 Xcode 工具的 Apple Silicon Mac 上，从本目录运行：

```sh
python3 run_preflight.py --kernel /absolute/path/to/linux-arm64-Image
```

脚本编译 Swift，为临时程序添加 Apple hypervisor 与 virtualization 权利并进行
临时签名，校验无磁盘、无网络的 Linux VM 配置，并创建、销毁空 Hypervisor 上下文。
结果写入 `preflight-result.json`。退出码 2 表示前置条件失败，不是 VM 验收成功；
即使退出码为 0，也只代表预检通过，不代表 guest 执行通过。

## 已观察到的边界

当前受管理的执行环境完成了编译和签名，空 Hypervisor 上下文创建、销毁均成功。
但 Virtualization.framework 返回 `isSupported == false`，配置校验返回
`VZErrorDomain` 错误码 2。这不能证明物理 Mac 不支持虚拟化；需要在普通 Terminal
会话中检查差异。实验未禁用任何环境限制。

PATH 中没有 Apple `container`。一次有时限的 GitHub HTTPS 请求因 DNS 解析失败，
无法取得 Containerization 依赖。实验只读使用已有 Docker ARM64 Linux 内核进行
配置校验，没有启动或修改 Docker、Lima。

## 尚待完成的第一阶段验收

1. 查明实际执行环境中的框架可用性。
2. 选择并固定 Linux ARM64 内核与 guest 镜像。较新的
   [最小 microVM 实验](../microvm/README.zh-CN.md)直接调用 Virtualization.framework，
   无须 Apple Containerization。
3. 启动独立 VM，不配置 guest 网络或宽泛的宿主目录挂载。
4. 传入固定输入、执行有界转换并比较输出。
5. 让任务超过期限，停止所属 VM 并确认终止。
6. 记录版本、身份、耗时和准确错误，再选择生产引擎。
   guest 或 CLI 退出均不足以单独证明终止。

本实验尚未通过上述 guest 验收，不能晋升为产品执行后端。
