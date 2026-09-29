# 安全策略 / Security Policy

MimicWX-Linux 处理微信账号数据、数据库密钥、消息和附件，并使用 Linux capability 访问客户端进程与文件事件。安全漏洞请通过私密渠道报告。

## 版本信息

报告中请注明版本号或 Git 提交号，并说明是否能在当前 `main` 分支复现。版本变更见 [CHANGELOG.md](CHANGELOG.md)。项目未设定长期支持周期或历史版本回补承诺。

## 私密报告

请通过 [GitHub 私密漏洞报告](https://github.com/qianshulab/MimicWX-Linux/security/advisories/new) 提交。

不要在公开 Issue、Discussion、Pull Request 或日志附件中披露漏洞细节。

报告请包含：

- 受影响版本或 Git 提交号
- 漏洞类型和潜在影响
- 最小复现步骤或概念验证
- 所需前置条件与权限
- 建议修复方式（如有）
- 已采取的披露或缓解措施

请先对所有样本和日志进行脱敏。不要上传真实微信数据库、密钥映射、口令、API Token、联系人信息、聊天内容或私人附件。

## 报告范围

与本项目相关的安全问题包括：

- API 认证绕过或 Token 泄露
- 附件路径穿越、软链接逃逸或任意文件读取
- 未授权消息发送或会话路由错误
- 恶意消息或附件导致的代码执行、拒绝服务或资源耗尽
- 数据库口令、派生密钥或个人数据写入日志或镜像
- noVNC、WebSocket 或反向代理配置导致的访问控制绕过
- 容器 capability、挂载或启动脚本造成的宿主机影响

微信客户端兼容性问题、消息重放和一般部署故障可通过 [Issues](https://github.com/qianshulab/MimicWX-Linux/issues) 报告。如果问题涉及未授权访问、数据泄露或其他安全影响，请使用私密渠道。

## 响应与披露

请在私密报告中讨论复现条件、修复方案和披露时间。修复或缓解措施可用前，请勿公开可用于利用漏洞的细节。项目不承诺固定的响应或修复时限。

---

## English summary

Report vulnerabilities through [GitHub private vulnerability reporting](https://github.com/qianshulab/MimicWX-Linux/security/advisories/new), not public issues. Include the affected version or commit, impact, prerequisites, reproduction steps, and any proposed mitigation. Use synthetic data and remove credentials and personal information.

Discuss remediation and disclosure in the private report. No fixed response time, support period, or backport schedule is guaranteed.
