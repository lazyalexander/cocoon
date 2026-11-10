# Rust 持久根磁盘实验

[English（权威版本）](PERSISTENCE.md) · 简体中文

这是固定工作负载验收，不是通用执行服务。Rust 负责类型化 VM/镜像/启动/操作身份、
SQLite 意图和结果、工作磁盘租约、有界 runner 传输及超时监督。Swift 仅增加 virtio
块设备和框架观测。串口 guest 声明与框架终止确认分别处理。

Rust Linux ARM64 PID 1 将 `/dev/vda` 挂为 ext4 根，切换根目录并执行盘上的
`/guest-init`。验证自身可执行文件及根挂载后，写入新 32 字节内容、设置 0600 权限、
同步文件及目录，等待宿主停止。新启动实例读回内容并验证权限。标记不证明终止。
fixture 无通用 shell、网络、包管理器或 agent 命令入口，不是 Debian 用户空间。

## 构建与运行

固定宿主工具链须具备 `aarch64-unknown-linux-musl` target。构建器显式选择 Homebrew
LLD 和 e2fsprogs，避开 PATH 中可能优先出现的 Android SDK mke2fs。
`COCOON_LLD`、`COCOON_MKE2FS` 可显式覆盖；校验工具身份并记录版本及可执行文件
摘要。保留 LLD 可执行文件名，因为文件名决定驱动模式。拒绝环境中的 target、编译器
及 flags 覆盖。命令不隐式安装依赖或下载内核。

```sh
cargo build -p cocoon --release
cargo test -p cocoon-platform --release eight_gib_sparse_fallback -- --ignored
python3 experiments/microvm/probe.py build
python3 experiments/microvm/root_modules.py
python3 experiments/microvm/build_root.py
python3 experiments/microvm/persistence.py --fixture FIXTURE_DIRECTORY --kernel KERNEL_IMAGE --rounds 10
```

使用构建器返回的私有 fixture 目录，其中包括 8 GiB 逻辑容量稀疏根磁盘、initramfs
和清单，记录工具版本、摘要及实际分配量。产物和运行数据留在已有忽略目录。
受管理 macOS 环境可能须在执行沙箱外运行；拒绝时绝不回退宿主原生执行。

Python 仅编排验收，实际启动归属由 Rust 的
`host STATE experiment persistence VM RUNNER write|read PAYLOAD` 管理。使用 release
宿主二进制，因为 debug 模式下完整 8 GiB 逻辑镜像 SHA-256 校验很慢。校验时间不是
guest 启动基准，仍需后续优化。

## 持久化与控制

platform crate 隔离原生 `clonefile` FFI。优先 APFS 克隆，仅不支持或跨设备时允许
稀疏复制；权限、I/O、空间不足和目标已存在均报错。稀疏复制跳过全零块并保留长度。
报告包含复制方式、逻辑容量和当前分配量；APFS 共享块统计不等于独占物理占用。
工作盘可写，基线只读，使用前校验摘要。

锁定独立租约文件而非磁盘本体，因为 Apple 框架附件自行锁定磁盘。spawn 前在数据库
记录不确定状态；未知或中断的启动禁止复用。runner 死亡不证明 VM 终止。runner
fixture 截止时间为十秒，Rust 看门狗二十秒后终止并回收超时 runner，但不据此认定
VM 已终止。guest 输出与可信控制结果分别限制。

schema v0 事务迁移到 v1，保留旧镜像和 prepared VM。历史复制计量缺失仍为未知。
未知的更高 schema 版本拒绝修改。状态及镜像位置是可信宿主数据，不隔离同用户任意
宿主代码。

实验尚无独立持久监督、未知结果自动核对、授权、回滚/分支执行或生产磁盘容量限制。
本轮验收是宿主确认停止后的 retain 持久化，不可手动清除未知状态后重试。

## 来源与验证

原创 fixture 代码为 MIT；链接的 Rust 标准库与 musl 保留上游许可。工具链和
 e2fsprogs 是外部构建工具，不被本仓库重新许可。操作员内核保留 Linux 及发行方
声明。不提交内核、磁盘、工具二进制或私有报告。

运行 `tools/check`、交叉 target guest Clippy、原生协议测试和真实十轮流程。
宿主测试覆盖稀疏内容/分配量、原生克隆独立性、拒绝覆盖/符号链接、schema 拒绝和
错误 fixture 请求。宿主测试不证明 guest 可运行。通过要求二十个独立启动身份、
基线摘要不变、ext4 根及执行文件检查、内容/权限持久化及框架终止确认。

显式 `root_modules.py` 步骤获取固定 SHA-256 的 Debian 安装器及 ext4/scsi 模块包，
对应内核 6.12.107+deb13-arm64。仅 virtio_blk、crc32c_generic、jbd2、mbcache、crc16 和 ext4 按依赖
顺序进入 initramfs。准备时检查模块版本，加载时由内核校验。模块保留上游 GPL 和
发行方声明；HTTPS 摘要固定不等于验证独立签名。

## 已观察到的验收

当前 Apple Silicon macOS 宿主上十轮写入/读回全部通过：二十个独立启动身份、ext4
根及磁盘可执行文件校验、0600 内容持久化、每次框架停止确认、基线 SHA-256 不变。
8 GiB 逻辑基线初始分配 207,306,752 字节，含 journal。APFS 克隆独立性及显式 8 GiB
稀疏复制回退测试也通过。分配量不代表 APFS 总独占空间。

此前尝试暴露了磁盘锁冲突与缺失运行时文件系统模块（包括动态 CRC32C 依赖）。
报告均已保留；附件失败保持未知隔离，guest 阶段失败已确认宿主停止。通过的测试
使用新环境，没有自动重放未知尝试。这不代表通用后端或 v0.1 性能门槛通过。
