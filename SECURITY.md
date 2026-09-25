# 安全策略 / Security Policy

MimicWX-Linux 涉及微信账号数据、数据库密钥、消息内容、附件和具备较高权限的容器 capability。安全问题必须通过私密渠道报告。

## 支持版本

| 版本 | 安全更新 |
| --- | --- |
| `0.6.x` | 支持 |
| `< 0.6` | 不再主动维护 |

建议始终使用默认分支的最新稳定提交，并在升级前保留可回滚镜像和持久化数据备份。

## 私密报告

请使用仓库页面的 **Security → Report a vulnerability** 提交私密报告：

<https://github.com/qianshulab/MimicWX-Linux/security/advisories/new>

不要在公开 Issue、Discussion、Pull Request 或日志附件中披露漏洞细节。

报告建议包含：

- 受影响版本或 Git 提交号
- 漏洞类型和潜在影响
- 最小复现步骤或概念验证
- 所需前置条件与权限
- 建议修复方式（如有）
- 已采取的披露或缓解措施

请先对所有样本和日志进行脱敏。不要上传真实微信数据库、密钥映射、口令、API Token、联系人信息、聊天内容或私人附件。

## 安全范围

优先处理的问题包括：

- API 认证绕过或 Token 泄露
- 附件路径穿越、软链接逃逸或任意文件读取
- 未授权消息发送或会话路由错误
- 恶意消息或附件导致的代码执行、拒绝服务或资源耗尽
- 数据库口令、派生密钥或个人数据写入日志或镜像
- noVNC、WebSocket 或反向代理配置导致的访问控制绕过
- 容器 capability、挂载或启动脚本造成的宿主机影响

以下情况通常不视为项目漏洞：

- 将 noVNC 或 API 无保护地直接暴露到公网
- 主动关闭 API Token 后产生的未授权访问
- 宿主机、Docker Engine 或反向代理本身未修复的漏洞
- 微信官方客户端升级导致的兼容性失效
- 已在文档中明确说明的至少一次投递和消息重放行为

## 响应与披露

维护者会尽力确认报告、评估影响并协调修复。修复可用前，请避免公开技术细节。完成修复后，可在双方确认的时间发布安全公告与贡献者致谢。

---

## English summary

Report security issues privately through [GitHub Security Advisories](https://github.com/qianshulab/MimicWX-Linux/security/advisories/new). Do not open a public issue. Include the affected version, impact, prerequisites, reproduction steps, and a proposed mitigation when available. Remove all WeChat databases, keys, tokens, contacts, messages, and private attachments from the report.
