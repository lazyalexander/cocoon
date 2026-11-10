# 离线 agent 项目实验

[English（权威版本）](PROJECT.md) · 简体中文

[使用说明](../../docs/user-guide.zh-CN.md) · [持久化实验](PERSISTENCE.zh-CN.md)

这是明确由可信操作者启动的实验，使用真实 macOS VM、独立 ext4 磁盘、Rust Linux
helper 和 virtio-vsock。没有配置 SSH、网卡、宿主共享目录、GPU 或本机执行降级。
它是固定验收流程，不是公开 agent API，也不是完整 Debian 开发镜像。

## 运行

前置工具和固定内核获取方法见[使用说明](../../docs/user-guide.zh-CN.md)。
project 选项额外下载固定 Debian 内核包以获得 vsock 模块；已有文件摘要不符会拒绝。

```sh
python3 experiments/microvm/root_modules.py --project
python3 experiments/microvm/probe.py build
cargo build --release --locked -p cocoon
python3 experiments/microvm/build_root.py --project
python3 experiments/microvm/project.py --fixture /absolute/private/root-BUILD_ID --kernel /absolute/private/linux --rounds 3
```

使用 `build_root.py` 输出的私有目录。真实 VM 需要 macOS 虚拟化 entitlement 以及
允许虚拟化的执行环境。脚本新建私有状态目录、导入镜像、注册一个 VM，每次启动均
经过 Rust supervisor。已有 project 镜像 VM 的等效固定命令为：

```text
cocoon host ABSOLUTE_PRIVATE_STATE_DIR experiment project VM_UUID ABSOLUTE_SIGNED_RUNNER
```

`vm start` 仍不可用。不要给项目实验传入仅支持持久化的 fixture。
下载内容、生成镜像和报告都留在本地忽略目录。完整内核包仅用于构建，镜像只含选定模块。

## 验证内容

每次启动按序经 vsock 发出八个请求：

1. 导入知识文件 `/workspace/knowledge.txt`。
2. 导入一个故意未完成的 shell 脚本。
3. 读回脚本并核对原始字节。
4. 将其改为读取知识文件并生成成品的代码。
5. 在 `/workspace` 执行 `/bin/sh build.sh`，要求退出码 0 和 `TEST_PASS`。
6. 读回成品字节。
7. 另行验证命令的 stderr 和退出码 7。
8. 验证 helper 的两秒命令超时。

宿主核验每条响应身份和预期值。只有框架确认 VM 停止后，才将已验证内容写入本次
启动私有目录下新建的 `artifact.txt`，权限 0600，并记录大小与 SHA-256。
guest 文件名不能选择宿主目的路径，不覆盖现有文件，也不在宿主执行返回内容。
脚本检查产物字节与摘要、持久化的 stopped 状态以及基础磁盘未变。
这证明了无头 shell 代码流程，不代表模型推理或第三方 agent 集成已经验收。

## 限制和权限边界

`cocoon-guest-wire` 管理严格类型协议和四字节大端长度帧；`cocoon-guest-helper`
管理 Linux 执行和 vsock。宿主控制 crate 继续禁止 unsafe。helper 没有注册、镜像、
授权、回收或宿主路径操作。caller 归属、task UUID、boot/attempt 身份和请求序号
分别记录；匹配这些字段不等于身份认证。当前只有可信宿主生成的固定工作负载，
不接收不可信 agent 提交的任务。生产限权授权服务仍待实现。

每次启动仅一条连接，最多 16 个请求；每帧 32 KiB，UTF-8 文件内容、stdout 和
stderr 各 8 KiB，响应累计 128 KiB，宿主工作流输入 16 KiB。命令清空环境，显式
指定 PATH、argv/cwd，stdin 为空，并使用独立进程组。guest 命令限时两秒；
宿主另设十秒 VM 截止时间，并保留停止与 runner watchdog。通信 I/O 超时三秒。
断连或畸形响应不重放；停止未确认则 VM 保持 unknown，禁止复用磁盘。
通信失败后的确认停止，也不证明命令是否已经完成。

helper 以 guest root 运行，可以按 Linux 文件系统规则写 guest 文件，包括自身
可执行文件。这不是 VM 内部的路径沙箱。命令可能破坏 helper 或逃离进程组；
只有独立的 VM 停止才能确认外层执行边界。不据此宣称对抗性隔离或生产资源限制。
知识进入 VM 后，仍会被获准读取者以及其发送内容的外部模型看到。

## 验收和后续工作

开发机连续三次真实项目启动通过，包括成品接收、退出码/stderr、超时、确认停止和
基础镜像完整性。这是功能证据，不是 M5 容量或延迟资格。最终源码还执行 workspace
测试/lint、Linux ARM64 helper 交叉 lint 和原生协议测试；这些检查单独不能证明 VM 执行。

仍未实现：完整 Debian 用户态、Git/语言工具链、任意项目准入与导入清单、外部 agent
socket/授权、二进制或大文件传输、流式输出、按请求 ID 取消、通用产物选择、自动核对、
回滚/分支执行以及并发资格。已验证保留；rollback/branch 仍仅存储策略。
下一里程碑是宿主授权的项目输入与面向外部 agent 的会话，同时保持生命周期边界。

## 第三方来源

安装器摘要固定在 `root_modules.py`，仅选用其中 BusyBox 和 glibc loader/libc；
提取时不沿归档成员路径写文件。BusyBox 为 GPL-2.0，glibc 为 LGPL-2.1-or-later，
保留各自许可，不改为 Cocoon 的 MIT。源码与许可信息见：
[Debian BusyBox](https://sources.debian.org/src/busybox/)、
[Debian glibc](https://sources.debian.org/src/glibc/) 和
[Debian Linux](https://sources.debian.org/src/linux/)。
内核包固定 SHA-256 来自
[Debian 下载元数据](https://packages.debian.org/trixie/arm64/linux-image-6.12.107%2Bdeb13-arm64/download)。
构建清单记录输入、文件与 helper 摘要。这里不提交或分发第三方二进制；
发布镜像还需要完整许可、对应源码和来源打包。
