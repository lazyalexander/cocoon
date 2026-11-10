# Cocoon 使用说明书

[English（权威版本）](user-guide.md) · 简体中文

[项目概览](../README.zh-CN.md) · [验收规范](../SPECIFICATION.zh-CN.md) · [架构](architecture.zh-CN.md)

## 1. 当前可以使用什么

Cocoon 是面向 Apple Silicon macOS、Rust 优先的实验性无头 microVM 项目。当前提供
可信宿主镜像导入、持久化 VM 注册、真实启动/停止/核对、经认证的私有 Unix socket
API、guest 执行/文件操作、二进制产物导出、挂起/恢复、经批准的冷恢复、候选批准、
分支、回滚和主动宿主保护挂起。

组合真实 VM 验收已在由 Debian 输入构建的最小 BusyBox/glibc 用户空间上通过这些操作。
它不是完整 Debian 开发发行版、公开网络服务或防御不可信宿主用户的凭据系统；当前仍
信任同一 macOS 账户。容量、延迟、销毁/清理、广泛工作负载兼容性和完整 v0.1 故障
资格仍待完成。

[离线项目实验](../experiments/microvm/PROJECT.zh-CN.md) 和固定持久化 fixture 仍是
独立验收路径。当前 agent 工作流使用下面的管理链路验收。

## 2. 环境与构建

宿主记录和格式检查使用仓库固定的 Rust 工具链。真实 VM 实验需要 Apple Silicon
macOS、Xcode 命令行工具、Python 3、LLD 和 e2fsprogs。优先 APFS 以支持克隆，
为独立工作磁盘和私有失败证据预留空间。

以下命令在仓库根目录执行。需要时显式安装工具链和 target：

```sh
rustup toolchain install 1.98.0 --profile minimal --component rustfmt --component clippy
rustup target add aarch64-unknown-linux-musl --toolchain 1.98.0
cargo build --locked -p cocoon --release
./tools/check
```

CLI 位于 `target/release/cocoon`，无需安装到系统目录。生产功能不通过 Python
执行，但当前构建/验收脚本需要 Python。脚本不会安装 LLVM 或 e2fsprogs；默认使用
Homebrew `lld@22` 与 `e2fsprogs`，可通过 `COCOON_LLD`、`COCOON_MKE2FS` 显式指定
可执行文件路径。

## 3. 只校验任务，不执行

```sh
target/release/cocoon plan examples/task.json
```

命令检查有界离线任务并输出 JSON。示例镜像不存在。返回
`execution_available: false`、`execution_authorized: false` 是正常结果；格式
合法不等于授权，也不会启动 VM。

## 4. 运行真实持久化实验

以下命令会在本地缺少输入时，显式下载固定版本的第三方材料：

```sh
python3 experiments/microvm/fetch_kernel.py
python3 experiments/microvm/root_modules.py
```

下载校验摘要，上游变化会被拒绝，不自动更换摘要。内核和模块保留原许可。
然后构建签名后的原生适配器与固定根镜像：

```sh
python3 experiments/microvm/probe.py build
python3 experiments/microvm/probe.py doctor
python3 experiments/microvm/build_root.py
```

`doctor` 只检查能力，不启动 VM。根镜像构建器输出 `private_output`，把该确切
目录填入下面的 `COCOON_FIXTURE`。不要自动取最新目录，旧产物可能使用不同模块。

```sh
COCOON_FIXTURE="/absolute/path/from/private_output"
python3 experiments/microvm/persistence.py \
  --fixture "$COCOON_FIXTURE" \
  --kernel experiments/microvm/build/debian/linux \
  --rounds 10
```

工具为本次运行创建独立私有状态目录，然后对一个工作磁盘执行十轮写入和读回，
共二十次启动。guest 内存为 512 MiB，逻辑磁盘容量为 8 GiB、稀疏分配，无网络或
宿主共享目录。只执行固定程序，不支持任意命令。

成功要求 `passed: true`、`boots: 20`、`unique_boots: true` 和
`baseline_unchanged: true`；每次启动还必须有框架停止确认。
`production_backend_qualified` 仍为 false。首个失败就停止，不得自动重试未知结果。

报告和 SQLite 留在被忽略的 `experiments/microvm/runs/`，镜像、清单和二进制留在
被忽略的 `experiments/microvm/build/`，不要公开这些目录。测量边界、工具校验及
限制见[持久化实验说明](../experiments/microvm/PERSISTENCE.zh-CN.md)。

## 5. 直接管理宿主记录

可以不运行完整验收工具，单独操作底层管理命令。选择版本控制之外的私有绝对路径：

```sh
COCOON_STATE="$HOME/Library/Application Support/Cocoon/manual-state"
mkdir -p "$COCOON_STATE"
chmod 700 "$COCOON_STATE"
COCOON_KERNEL="$(pwd -P)/experiments/microvm/build/debian/linux"
target/release/cocoon host "$COCOON_STATE" image import \
  "$COCOON_KERNEL" "$COCOON_FIXTURE/initrd.cpio.gz" "$COCOON_FIXTURE/root.raw" \
  "Debian kernel/modules: upstream licenses; original Cocoon fixture: MIT"
```

使用返回的镜像 UUID，再生成一个 VM UUID。同一 VM UUID 配相同注册参数是幂等的，
更改参数会被拒绝。owner 字符串仅表示归属，不代表认证。

```sh
COCOON_IMAGE="UUID_FROM_IMAGE_IMPORT"
COCOON_VM="$(uuidgen)"
target/release/cocoon host "$COCOON_STATE" image show "$COCOON_IMAGE"
target/release/cocoon host "$COCOON_STATE" vm create "$COCOON_VM" "$COCOON_IMAGE" local-operator retain
target/release/cocoon host "$COCOON_STATE" vm list
target/release/cocoon host "$COCOON_STATE" vm show "$COCOON_VM"
target/release/cocoon host "$COCOON_STATE" operations
```

注册通常返回 `prepared`，不表示 VM 已运行。镜像导入记录摘要与所提供的来源，
不代表已验证可启动或信任发布者。完整命令参考：

| `host STATE` 后的参数 | 作用 |
| --- | --- |
| `image import KERNEL INITRD ROOT_RAW PROVENANCE` | 导入私有基线副本，返回镜像 UUID。 |
| `image show IMAGE_UUID` | 读取镜像清单。 |
| `vm create VM_UUID IMAGE_UUID OWNER POLICY` | 创建独立工作磁盘与注册记录。 |
| `vm list` / `vm show VM_UUID` | 查询记录，不是实时 VM 核对。 |
| `operations` | 查询导入/创建意图及完成状态；启动记录另行保存。 |
| `vm start VM_UUID RUNNER` | 用显式签名 runner 启动真实管理 VM，返回 attempt/generation/lease。 |
| `vm stop VM_UUID` / `vm reconcile VM_UUID` | 请求确认停止或核对保留观测。 |
| `serve RUNNER` | 用固定 runner 启动经认证的私有宿主 socket 服务。 |
| `experiment persistence VM_UUID RUNNER write\|read PAYLOAD` | 一次有界 fixture 启动；RUNNER 须为 ARM64 Mach-O 绝对路径，PAYLOAD 为 32 个十六进制字符。 |

最后一条命令须使用构建好的 fixture、签名后的 `cocoon-vz`，写入/读回使用相同
payload。优先用完整验收脚本，它会检查二十次结果及基线完整性。`vm list` 不能代替
独立的终止证据。

## 6. 错误、恢复与数据处理

CLI 成功退出码为 0，通常输出 JSON。错误退出码为 2，stderr 输出英文说明；失败的
fixture 也可能在 stdout 输出 JSON 报告，应同时保留并检查退出码。

| 现象 | 处理方法 |
| --- | --- |
| 状态目录权限错误 | 使用当前账户拥有的私有目录，设置 0700，不放宽为其他用户可访问。 |
| 缺 target/工具、工具身份错误 | 安装指定 target/工具或显式选择可执行文件，不通过 PATH 偷换为 Android SDK 工具。 |
| 内核/模块摘要不符 | 保留原文件，审阅来源和版本变化，不关闭校验或覆盖操作员文件。 |
| `doctor` 返回 EPERM 或框架不支持 | 区分沙箱拒绝与硬件不支持；在获得授权的 macOS 终端运行真实测试，不改为原生执行。 |
| VM 未知状态、要求 reconciliation | 保留原 VM/启动记录、runner 观测和磁盘，执行核对；不清记录或复用磁盘强行重试。 |
| 快照要求 recovery_required | 审阅失败快照，显式批准从持久工作磁盘冷恢复；不自动冷恢复。 |

停止不等于删除。目前没有受支持的自动卸载/VM 清理命令。手动移除私有状态或构建
产物前，必须确认没有 VM/进程仍占有它们，并保留必要证据。不能用删除代替核对；
只读查询不授予清理权限。工作磁盘和私有报告不进入提交或支持附件，只分享脱敏摘要。

## 7. 管理链路 agent 验收

构建包含 helper 的 fixture 并运行组合真实 VM 链路：

```sh
python3 experiments/microvm/root_modules.py --project
python3 experiments/microvm/build_root.py --project
python3 experiments/microvm/probe.py build
cargo build --offline --release -p cocoon
python3 experiments/microvm/production_acceptance.py \
  --fixture /absolute/private/root-BUILD_ID \
  --kernel /absolute/private/linux --automatic-suspend
```

加入 `--corrupt-before-resume` 可实际损坏快照，验证显式冷恢复批准及之后的主动挂起。
工具创建私有状态目录、启动 socket 服务、写入并执行 guest 内容、导出并逐字节检查
12,000 字节产物、以新身份挂起/恢复、保存并批准候选、启动分支、停止分支并回滚原 VM。
不会发布凭据或原始报告。
