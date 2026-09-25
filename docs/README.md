# MimicWX-Linux 文档中心

这里汇总 MimicWX-Linux 的部署、接口和项目维护文档。首次使用建议依次阅读“快速开始 → 部署指南 → API 参考”。

## 中文

| 文档 | 适用对象 | 内容 |
| --- | --- | --- |
| [项目首页](../README.md) | 所有人 | 项目定位、核心能力、快速开始和安全边界 |
| [Docker 部署指南](DEPLOYMENT.zh-CN.md) | 运维与自托管用户 | 安装、网络、代理、登录、升级、备份和故障排查 |
| [API 参考](API.zh-CN.md) | 接口调用方 | 消息身份、实时消费、历史补拉、附件、发送与回复 |
| [更新日志](../CHANGELOG.md) | 维护者与升级用户 | 版本变更和兼容性说明 |
| [贡献指南](../CONTRIBUTING.md) | 贡献者 | 开发环境、验证要求、提交和 Pull Request 规范 |
| [安全策略](../SECURITY.md) | 所有人 | 支持版本、安全问题报告和敏感数据要求 |

## 建议阅读路径

### 首次部署

1. 阅读项目首页的环境要求与安全提示。
2. 按照 Docker 部署指南准备配置、持久化目录和凭据。
3. 登录微信并确认 `/status` 中 `db_available=true`。
4. 使用 API 参考完成鉴权、实时接收和历史补拉。

### 接入业务系统

1. 先理解 `message_id`、`conversation_id` 和 `sender_id` 的区别。
2. 建立 WebSocket 后补拉历史消息，并按 `message_id` 持久化去重。
3. 把附件作为不可信输入处理，再交给下游系统。
4. 回复时优先使用稳定会话 ID；按消息回复只适用于仍在实时缓存中的消息。

### 升级现有部署

1. 查看更新日志和兼容性说明。
2. 备份微信数据、配置和凭据，并保留旧镜像标签。
3. 更新后验证登录、数据库、接收、附件下载、文本回复和文件发送。

## 文档约定

- `HOST` 表示运行 MimicWX-Linux 的主机地址。
- 示例中的 Token、消息 ID、会话 ID 和文件名均为占位值。
- 除 `/status` 外，API 示例默认需要 Bearer Token。
- 文档以当前 `main` 分支的 v0.6 接口为准。

---

## English

| Document | Audience | Contents |
| --- | --- | --- |
| [Project overview](../README.en.md) | Everyone | Positioning, capabilities, quick start, and security boundaries |
| [Docker deployment](DEPLOYMENT.md) | Operators | Installation, networking, proxy, sign-in, upgrade, backup, and troubleshooting |
| [API reference](API.md) | Integrators | Identity, real-time delivery, history catch-up, attachments, sending, and replies |
| [Changelog](../CHANGELOG.md) | Maintainers and operators | Version changes and compatibility notes |
| [Contributing](../CONTRIBUTING.md) | Contributors | Development setup, validation, commits, and pull requests |
| [Security policy](../SECURITY.md) | Everyone | Supported versions, private reporting, and sensitive-data requirements |

Recommended onboarding order:

1. Review the requirements and security boundaries in the project overview.
2. Follow the Docker deployment guide to prepare configuration and persistent state.
3. Sign in to WeChat and confirm that `/status` reports `db_available=true`.
4. Follow the API reference for authentication, real-time events, and history catch-up.
