# 贡献指南

感谢你改进 MimicWX-Linux。项目欢迎可验证的缺陷修复、微信版本兼容性改进、接口可靠性增强、测试和文档更新。

> English summary: open an issue before large or compatibility-sensitive changes, keep each pull request focused, include reproducible validation, and never upload WeChat data or credentials.

## 开始之前

- 缺陷报告应先搜索现有 Issue，避免重复提交。
- 涉及接口不兼容、数据库格式、密钥提取或整体架构的修改，建议先通过 Issue 说明需求和兼容策略。
- 安全问题不要提交公开 Issue，请按照 [SECURITY.md](SECURITY.md) 使用私密渠道。
- Issue、日志和测试数据中不得包含数据库、密钥、Token、联系人、聊天内容或真实附件。

## 开发环境

推荐环境：

- x86-64 Linux
- Rust stable
- Docker Engine 24+
- Docker Compose v2

克隆项目：

```bash
git clone https://github.com/qianshulab/MimicWX-Linux.git
cd MimicWX-Linux
```

Rust 核心的基础检查：

```bash
cargo fmt --check
cargo test --all-targets
```

容器配置检查：

```bash
cp .env.example .env
cp config.toml config.local.toml
mkdir -p secrets data/xwechat data/xwechat_files
printf '%s\n' 'temporary-local-password' > secrets/vnc_password.txt
docker compose config --quiet
```

不要把上述本地运行文件提交到 Git；它们已由 `.gitignore` 排除。

## 修改原则

### 保持兼容

- 新接口优先新增，不随意改变现有字段语义。
- 需要废弃接口时，先保留兼容路径并在更新日志中说明迁移方式。
- 数据库字段、XML 内容和 UI 节点都应视为可能变化的外部输入。
- 文件路径、文件名、Base64、Range 和分页参数必须进行边界校验。

### 保持安全边界

- 不把密钥、口令、数据库或个人数据写入日志。
- 不执行收到的附件，也不根据扩展名信任文件内容。
- 新增外部下载依赖时，应固定版本或校验摘要，并保留许可证。
- 不扩大容器 capability、端口暴露或宿主机挂载范围，除非有明确理由和风险说明。

### 保持可观测性

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

一个提交只处理一个清晰主题。不要混入无关格式化、个人配置、构建产物或运行数据。

## Pull Request 要求

Pull Request 应包含：

1. 修改目的和用户可见影响。
2. 兼容性、安全性和回滚影响。
3. 已执行的验证命令及结果。
4. 涉及接口时提供请求与响应示例。
5. 涉及微信版本兼容时提供版本号和脱敏日志。
6. 新增或变更行为对应的文档与更新日志。

提交前确认：

- [ ] `cargo fmt --check` 通过。
- [ ] `cargo test --all-targets` 通过。
- [ ] `docker compose config --quiet` 通过。
- [ ] 没有提交本地凭据、数据库、附件或个人信息。
- [ ] 新增依赖的许可证和固定版本已核对。
- [ ] 文档、示例和 `CHANGELOG.md` 已同步。

## 缺陷报告信息

为便于复现，请提供：

- MimicWX-Linux 版本或 Git 提交号
- 微信 Linux 客户端版本
- Linux 发行版、内核、Docker 与 Compose 版本
- 脱敏后的 `/status` 输出
- 最小复现步骤、预期行为和实际行为
- 相关脱敏日志

敏感信息应使用占位符替换，而不是简单截图遮挡后上传原文件。
