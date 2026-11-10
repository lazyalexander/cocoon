# 最小 macOS microVM 实验

[English（权威版本）](README.md) · 简体中文

本实验验证比最初 OCI 提案更小的执行路径：一个原生启动器、一个 ARM64 Linux
Image、一个极小 initramfs 和一个串口设备。不引入 Containerization 包、镜像守护
服务、网络、块设备、共享目录、图形设备或智能体框架，直接使用 Apple
Virtualization.framework，不自行实现 VMM。

启动器属于 Cocoon，不依赖 an-agent 或 spool。C guest 只执行固定验收工作负载，
不是通用任务执行器，也不是通过安全资格认证的 guest agent。现有 Rust 任务模型
仍是单独的契约原型；两个入口均未具备生产工作负载准入和监督能力。

## 构建与运行

需要 Apple Silicon、Xcode 工具、Python 3 和 LLVM 的 `ld.lld`。
以下命令不安装软件包或下载内容。从仓库根目录运行：

```sh
python3 experiments/microvm/probe.py build
python3 -m unittest discover -s experiments/microvm -p 'test_*.py' -v
python3 experiments/microvm/probe.py boot --kernel /absolute/path/to/Image --case roundtrip
python3 experiments/microvm/probe.py boot --kernel /absolute/path/to/Image --case deadline --seconds 2
python3 experiments/microvm/probe.py benchmark --kernel /absolute/path/to/Image --samples 30
```

用户提供未压缩的 Linux ARM64 Image，须支持 initramfs、gzip、ELF 和内建
virtio-console。实验计算内核摘要并只读使用，不重新分发内核。
通用桌面内核可用于兼容性验证，但不是最小生产内核。

构建产物写入已忽略的 `build/`。私有请求、输入镜像、日志和证据写入已忽略的
`runs/`，每次执行创建新的私有目录。原生启动器经过优化和临时签名，尚不是经过
公证的发行版本。

## 验收

`roundtrip` guest 发出就绪标记，读取 initramfs 中的输入，转换为大写，
输出与本次令牌绑定的结果，并继续保持运行。宿主核对返回字节，再停止该 VM。
这只验证有界启动输入和串口输出，不代表通用文件传输、任意代码执行或产物保管。

`deadline` guest 发出就绪标记后保持运行，不输出完成标记。宿主必须在期限到达后
停止它。两种场景均要求框架停止回调成功并观察到 stopped 状态，或观察到 guest
停止。就绪标记只是工作负载证据，既不能证明终止，也不能授予权限。

串口采集上限为 64 KiB。启动器设置 5 秒停止看门狗，Python 工具另有最终进程超时。
看门狗到期记为终止未确认，不算通过，也不自动重试。本实验尚无持久监督、崩溃
对账、调用者认证或生产级资源核算。

初始请求配置为 1 vCPU、128 MiB 内存。配置校验可能拒绝；只有真实成功启动才能
证明支持。拒绝时不禁用环境检查，不选择其他引擎，也不改为宿主原生执行。

## 测量

报告分别记录身份准备、启动器进程总耗时、框架启动完成、guest 就绪、guest 完成
和 VM 停止确认时间。启动器从初始化开始使用单调时钟计时。
停止耗时可由 `stopped_ms - stop_requested_ms` 计算。
启动器峰值 RSS 不包含框架辅助进程，不能作为整个 VM 的内存开销。

基准每个样本使用独立启动器进程和新的 VM 执行尝试，复用已构建产物，不清空宿主
文件系统缓存。汇总仅对通过验收的样本计算中位数和最近秩 P95，遇到首次失败即停止。
构建或配置拒绝耗时不作为 guest 启动性能。与 Firecracker 比较时，须对齐就绪定义、
guest 内核、硬件环境和缓存条件；当前不宣称性能相当。

## 当前观察

ARM64 guest init 和优化原生启动器均已构建成功。两种真实启动测试在受管理执行
环境中均停在配置校验，返回 `VZErrorDomain` 错误码 2。Linux 启动、输入输出和
期限停止仍未通过验收。此前 Hypervisor 空上下文测试成功，因此仍需诊断实际宿主
环境中的框架拒绝原因。

当前配置的设备比早期 1 GiB 预检更少，但这不证明 128 MiB guest 可用，也不代表
达到 Firecracker 的安全保障。macOS 宿主进程隔离和恶意 guest 输出防护仍须单独验收。
