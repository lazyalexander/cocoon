# guest-wire

[English（权威版本）](README.md) · 简体中文

有界 guest 消息协议。定义类型化请求、响应、关联身份与长度前缀帧；身份匹配仅用于关联，不等于授权。

工作区依赖：serde、serde_json。参见[架构](../../docs/architecture.zh-CN.md)与[贡献规范](../../CONTRIBUTING.zh-CN.md)。

## 边界

本 crate 不接收授权 token，也不判断宿主权限。分配内存或转发前必须限制每帧和载荷大小。
