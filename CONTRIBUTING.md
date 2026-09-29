# 贡献指南

MimicWX-Linux 接受缺陷修复、微信客户端兼容性改进、接口改进、测试和文档贡献。

> For English-speaking contributors: discuss breaking or architectural changes in an issue first. Keep pull requests focused, include validation results, and exclude credentials and personal data.

## 问题反馈与变更讨论

- 缺陷报告应先搜索现有 Issue，避免重复提交。
- 涉及接口不兼容、数据库格式、密钥提取或整体架构的修改，建议先通过 Issue 说明需求和兼容策略。
- 安全问题不要提交公开 Issue，请按照 [SECURITY.md](SECURITY.md) 使用私密渠道。
- Issue、日志和测试数据中不得包含真实微信数据库、密钥、Token、联系人、聊天内容或私人附件。复现数据应使用虚构值。

## 开发环境

项目运行于 x86-64 Linux。开发环境需要：

- Rust stable
- Docker Engine
- Docker Compose v2
- 与 [Dockerfile](Dockerfile) 一致的 Linux 构建依赖

克隆项目：

```bash
git clone https://github.com/qianshulab/MimicWX-Linux.git
cd MimicWX-Linux
```

Rust 代码检查：

```bash
cargo fmt --check
cargo test --locked --all-targets
```

容器配置检查使用本地配置和测试凭据：

```bash
cp .env.example .env
cp config.toml config.local.toml
mkdir -p secrets data/xwechat data/xwechat_files
printf '%s\n' 'temporary-local-password' > secrets/vnc_password.txt
docker compose config --quiet
```

上述本地配置、凭据和数据目录已由 `.gitignore` 排除。运行容器前，请按照 [部署指南](docs/DEPLOYMENT.zh-CN.md) 配置独立的 VNC 密码和 API Token。

## 修改原则

### 接口与兼容性

- 保持现有接口和字段语义兼容。
- 需要废弃接口时，先保留兼容路径并在更新日志中说明迁移方式。
- 数据库字段、XML 内容和 UI 节点都应视为可能变化的外部输入。
- 文件路径、文件名、Base64、Range 和分页参数必须进行边界校验。

### 安全

- 不把密钥、口令、数据库或个人数据写入日志。
- 不执行收到的附件，也不根据扩展名信任文件内容。
- 新增外部下载依赖时，应固定版本或校验摘要，并保留许可证。
- 不扩大容器 capability、端口暴露或宿主机挂载范围，除非有明确理由和风险说明。

### 日志与后台任务

- 错误信息应能定位问题，但不得泄露敏感路径或凭据。
- 新的后台任务必须有清晰的启动、恢复和停止行为。
- 重试应设置边界或退避，避免异常情况下形成高频循环。

## 提交规范

建议使用简洁的 Conventional Commits 风格：

```text
feat: add conversation filter
fix: reject escaped attachment path
docs: clarify history checkpoint semantics
test: cover message identity mapping
```

每个提交聚焦一个主题，避免混入无关格式化、个人配置、构建产物或运行数据。

## Pull Request 要求

Pull Request 描述应说明解决的问题、用户可见行为和验证结果。涉及不兼容变更时，注明迁移方式和回滚限制；涉及接口或微信版本兼容时，附上请求与响应示例、客户端版本和脱敏日志。

按变更范围执行检查：

| 变更 | 验证要求 |
| --- | --- |
| Rust 代码 | `cargo fmt --check`、`cargo test --locked --all-targets` |
| Docker 或启动配置 | `docker compose config --quiet`，以及受影响的启动、登录或恢复流程 |
| API 行为 | 请求与响应示例、兼容性说明，以及受影响的接收、发送或附件流程 |
| 文档 | 链接、命令、字段名与当前实现一致 |

用户可见的行为变更需同步更新文档和 `CHANGELOG.md`。新增依赖需说明来源、版本和许可证。无法完成的验证应注明原因，不要将未执行的检查标记为通过。

## 缺陷报告信息

为便于复现，请提供：

- MimicWX-Linux 版本或 Git 提交号
- 微信 Linux 客户端版本
- Linux 发行版、内核、Docker 与 Compose 版本
- 脱敏后的 `/status` 输出
- 最小复现步骤、预期行为和实际行为
- 相关脱敏日志

敏感信息应使用占位符替换，而不是简单截图遮挡后上传原文件。
