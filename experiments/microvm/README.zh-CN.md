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
python3 experiments/microvm/probe.py doctor
python3 experiments/microvm/runner_checks.py experiments/microvm/build/cocoon-vz
python3 -m unittest discover -s experiments/microvm -p 'test_*.py' -v
python3 experiments/microvm/probe.py check-kernel --kernel /absolute/path/to/Image
python3 experiments/microvm/probe.py boot --kernel /absolute/path/to/Image --case roundtrip
python3 experiments/microvm/probe.py boot --kernel /absolute/path/to/Image --case deadline --seconds 2
python3 experiments/microvm/probe.py boot --kernel /absolute/path/to/Image --case cancel --seconds 10
python3 experiments/microvm/probe.py benchmark --kernel /absolute/path/to/Image --samples 30
```

用户提供未压缩的 Linux ARM64 Image，须支持 initramfs、gzip、ELF 和内建
virtio-console。实验计算内核摘要并只读使用，不重新分发内核。
通用桌面内核可用于兼容性验证，但不是最小生产内核。

构建产物写入已忽略的 `build/`。私有请求、输入镜像、日志和证据写入已忽略的
`runs/`，每次执行创建新的私有目录。原生启动器经过优化和临时签名，尚不是经过
公证的发行版本。

`check-kernel` 在存在内嵌 IKCONFIG 时检查配置，要求
`CONFIG_BLK_DEV_INITRD`、`CONFIG_RD_GZIP`、`CONFIG_BINFMT_ELF`、`CONFIG_VIRTIO`
和 `CONFIG_VIRTIO_CONSOLE` 均内建（`=y`）。候选配置不代表启动成功。
缺少内嵌配置时报告“未知”，使用者可根据已知配置继续实验；已知不兼容或格式损坏
的内嵌配置会在创建 VM 尝试之前被拒绝。配置解压上限为 2 MiB。
这属于 Python 工具的检查；原生启动器仍是实验内部接口，不是准入 API。

内核命令行采用 `panic=0`，防止 panic 后自动重启。串口出现 panic 文本时，
宿主以 `guest_panic` 为由请求停止；该文本仅是不可信诊断线索，不能证明 panic
确实发生，也不能证明 VM 已终止。

## 验收

`roundtrip` guest 发出就绪标记，读取 initramfs 中的输入，转换为大写，
输出与本次令牌绑定的结果，并继续保持运行。宿主核对返回字节，再停止该 VM。
这只验证有界启动输入和串口输出，不代表通用文件传输、任意代码执行或产物保管。

`deadline` guest 发出就绪标记后保持运行，不输出完成标记。宿主必须在期限到达后
停止它。两种场景均要求框架停止回调成功并观察到 stopped 状态，或观察到 guest
停止。就绪标记只是工作负载证据，既不能证明终止，也不能授予权限。

`cancel` 场景使用持续等待的 guest，在两秒后向所属启动器发送 SIGTERM；
若期限较短，则在期限一半时发送。启动器在 VM 队列中处理 SIGTERM、SIGINT，
请求框架停止，并仍须确认终止。在 guest 就绪之前收到信号不算验收通过。
串口采用非阻塞读取，每个事件最多读取 4 KiB，在 VM 队列中处理，不在单独的
回调队列积累 guest 输出。进入停止流程后，继续排空输出但不再保留字节。

串口采集上限为 64 KiB。启动器设置 5 秒停止看门狗，Python 工具另有最终进程超时。
看门狗到期记为终止未确认，不算通过，也不自动重试。本实验尚无持久监督、崩溃
对账、调用者认证或生产级资源核算。

外层超时或执行中断时，工具会终止并回收所属启动器进程，同时保留 VM 终止未知
的结论。这只是本地进程清理，不能替代独立持久监督或 VM 对账。

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

## 实机基线

Ubuntu 6.8.0-146-generic ARM64 内核在 1 vCPU、128 MiB 配置下通过固定输入输出、期限停止和取消三项验收。复用同一内核、保留宿主文件缓存的 30 次 roundtrip 全部通过。guest 就绪时间中位数 367.57 ms，P95 为 380.13 ms。框架运行回调中位数 100.40 ms。启动器峰值 RSS 中位数为 9.44 MiB，框架辅助进程另计。

原始启动器（SHA-256 前缀 88107d61）在一次 10 秒期限测试中晚约 663 ms 请求停止，随后 4.37 ms 确认停止。该用例只验收停止原因与终止状态。新版使用严格计时器并记录 deadline_due_ms，实机偏差仍需复测。

lifecycle 用例在同一台驻留内存的 VM 上执行五轮暂停和恢复。每次框架确认暂停后等待 50 ms，恢复后通过串口发出命令。PID 1 依次递增、保存、递增、恢复、再递增计数器。验收要求依次返回 1、1、2、1、2，并由宿主确认最终停止。检查点位于 guest 内存中，仅覆盖计数器；整机快照和启动器退出后的恢复仍待实现。

```sh
python3 experiments/microvm/probe.py boot --kernel experiments/microvm/build/Image --case lifecycle
```
