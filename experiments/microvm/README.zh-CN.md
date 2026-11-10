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

## 较早的不兼容内核观察

ARM64 guest init 和优化原生启动器均已构建成功。受管理执行环境拒绝框架配置，
且读取两项宿主能力 sysctl 均返回 EPERM。使用者在普通终端运行 `doctor` 时，
框架报告支持，两项能力值均为 1。

使用者运行的 roundtrip 实验约在 168 ms 收到框架运行回调，并最终由宿主确认停止。
guest 未发出就绪标记：所用 LinuxKit 内核关闭了 `CONFIG_BLK_DEV_INITRD`，
尝试挂载根文件系统时发生 panic。此前的 `panic=-1` 导致立即重启，反复输出 panic
直至触及串口上限。现在工具会拒绝这份已知不兼容的配置，并禁用 panic 自动重启。
私有原始报告与使用者提供的内核二进制不随项目发布。

这证明了该失败尝试中观察到的 VM 启动与停止，不代表输入输出、期限停止或取消验收
通过。168 ms 的框架回调不是 guest 就绪时间，也不是成功启动基准；仍需兼容内核和
真实验收。无须内核的 `doctor` 不创建 VM，其私有报告仅用于能力诊断。

原生启动器协议检查会在创建 VM 前拒绝未知字段、无效限制、畸形 JSON 和超过
16 KiB 的请求。macOS CI 构建、签名启动器并运行这些检查，不启动或认证 guest。
Python 工具的进程测试同样不能证明 VM 取消能力已有效。

此前较少的设备配置本身不证明 guest 可用，也不代表达到 Firecracker 的安全保障。macOS 宿主进程隔离和恶意 guest 输出防护仍须单独验收。

## 固定 Debian 内核与真实无磁盘验收

显式下载工具获取 Debian trixie 安装器 ARM64 内核：

```sh
python3 experiments/microvm/fetch_kernel.py
python3 experiments/microvm/probe.py boot --kernel experiments/microvm/build/debian/linux --case roundtrip
python3 experiments/microvm/probe.py boot --kernel experiments/microvm/build/debian/linux --case deadline --seconds 2
python3 experiments/microvm/probe.py boot --kernel experiments/microvm/build/debian/linux --case cancel
python3 experiments/microvm/probe.py benchmark --kernel experiments/microvm/build/debian/linux --samples 30
```

下载须显式执行，上限为 128 MiB，SHA-256 必须为
`9c37ace54ef7e1919411fd912e9f34d6982ddd7f4c8ef3c8aee56f4c4d70db67`。
官方来源 URL 记录在 `fetch_kernel.py`；上游 `current` 改变后工具拒绝下载结果，
不会自动改变摘要。已有操作员文件只校验、不覆盖。摘要已与 Debian HTTPS 提供的
SHA256SUMS 对照，但未验证独立签名。内核保留 Linux/Debian 许可，仅存入忽略的
`build/`，不随仓库分发。无需 Debian 安装器根文件系统，guest 仍运行 Cocoon 的
独立 C init。

在 Apple Silicon macOS 27.0.1 上、管理沙箱外，1 vCPU、128 MiB 配置通过了输入
输出、两秒期限停止和 SIGTERM 取消。另有一次 512 MiB 输入输出通过。各场景均通过
所需的 guest 就绪、预期输出（适用时）和框架终止检查。

128 MiB 的 30 次独立重复输入输出全部通过（30/30）。从 runner 初始化起计，guest
就绪中位数为 239.875 ms，最近秩 P95 为 245.762 ms。未清空缓存。这只是固定工作
负载的无磁盘测量，不是冷缓存、完整 Linux 开发环境、总内存核算或 Firecracker
对比。原始报告保留在忽略的私有 `runs/`。可写根磁盘持久化、guest helper 和生产
所有权恢复仍未实现或验收。

新增有界磁盘路径见 [Rust 持久根磁盘实验](PERSISTENCE.zh-CN.md)。

## 实验性跨进程状态保存与恢复

构建持久化 Rust fixture 后，运行三个独立顺序样本：

```sh
python3 experiments/microvm/root_modules.py
python3 experiments/microvm/probe.py build
python3 experiments/microvm/build_root.py
python3 experiments/microvm/probe.py snapshot --fixture /absolute/private/root-BUILD_ID --kernel /absolute/private/linux --samples 3
```

`snapshot-api` 仅编译检查 SDK 接口。`snapshot` 命令启动 512 MiB 固定 guest，
持久化并同步磁盘内容，暂停 VM、保存完整机器状态、确认旧 runner 停止，再由一个
新 runner 从该状态恢复。仅存在于 RAM 的随机 nonce 必须一致，且计数器必须在恢复后
恰好递增一次。恢复准入时再次核对工作磁盘 SHA-256；恢复后的文件系统变化（包括
ext4 日志重放）单独记录，不视为检查点损坏。机器标识保存在私有状态文件旁，并在
恢复时重用。不使用冷启动或本机执行回退，失败样本不重放。

在 macOS 27.0.1（26A434）上，三个真实跨进程样本全部通过。每份状态文件为
24,121,344 字节。保存进程总耗时 5,505.2～5,561.0 ms；恢复进程总耗时
1,206.9～1,219.9 ms，其中包含 guest 在恢复标记前固定等待的一秒；框架恢复本身耗时
175.3～187.0 ms。这些是固定
工作负载测量，不是已批准的生产预算或完整内存释放测量。

本实验不实现生产租约、检查点/WAL 发布、主动挂起、流水线策略、产物保管、对抗性
隔离或并行快照恢复。协议测试本身不证明 VM 支持；必须同时通过真实 guest 计数器、
磁盘摘要、进程边界和框架终止检查。

### 快照负面验收

`snapshot_faults.py` 使用真实 VM，而非合成 runner 证据：

```sh
python3 experiments/microvm/snapshot_faults.py --fixture /absolute/private/root-BUILD_ID --kernel /absolute/private/linux --case corrupt-state
python3 experiments/microvm/snapshot_faults.py --fixture /absolute/private/root-BUILD_ID --kernel /absolute/private/linux --case wrong-machine-id
python3 experiments/microvm/snapshot_faults.py --fixture /absolute/private/root-BUILD_ID --kernel /absolute/private/linux --case cold-boot
python3 experiments/microvm/snapshot_faults.py --fixture /absolute/private/root-BUILD_ID --kernel /absolute/private/linux --case double-restore
```

每个用例使用新的 VM、磁盘和状态文件。损坏状态和错误机器标识必须被拒绝；完整
冷启动序列即使 guest 自身正常退出，也不能通过恢复验证；第二次恢复必须在 runner
的独占恢复认领处失败。认领后恢复失败或崩溃会消耗认领，未知结果必须核对而非重放。
认领创建前的失败尚未尝试框架恢复。

四个负面对例在同一 macOS 版本上全部通过；随后三个正常跨进程样本也在启用独占恢复
认领后全部通过。这些测试不证明能够防御可修改或删除私有状态的可信 macOS 用户，
也不能替代生产检查点认证、WAL 恢复或故障注入资格验收。
