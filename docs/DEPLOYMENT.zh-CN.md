# Docker 部署指南

> **适用版本：** MimicWX-Linux v0.6.x · **运行平台：** x86-64 Linux、Docker Engine 24+、Docker Compose v2

本文档说明如何在通用 x86-64 Linux 主机上部署和维护 MimicWX-Linux。容器内包含微信 Linux 客户端、轻量桌面、noVNC、MimicWX 服务和数据库密钥生命周期组件。

## 目录

- [运行要求](#1-运行要求)
- [目录规划](#2-目录规划)
- [首次配置](#3-首次配置)
- [网络配置](#4-网络配置)
- [镜像源与代理](#5-镜像源与代理)
- [启动与登录](#6-启动与登录)
- [就绪检查](#7-就绪检查)
- [接口验证](#8-接口验证)
- [持久化与备份](#9-持久化与备份)
- [升级与回滚](#10-升级与回滚)
- [安全加固](#11-安全加固)
- [故障排查](#12-故障排查)

## 1. 运行要求

- x86-64 Linux 主机
- Docker Engine 24 或更高版本
- Docker Compose v2
- 至少 2 GB 可用内存
- 至少 5 GB 可用磁盘空间，以及微信数据和附件所需容量
- 可访问微信软件包、Ubuntu 软件源、Rust 工具链和 crate 注册表

容器需要以下 Linux capability：

- `SYS_PTRACE`：自动捕获数据库登录口令
- `SYS_ADMIN`：现有数据库变更通知机制

这两项 capability 权限较高，只应在可信、隔离的主机上运行本服务。

## 2. 目录规划

建议把程序、持久化数据和私密配置放在独立目录中：

```text
/opt/mimicwx/
├── docker-compose.yml
├── config.local.toml
├── .env
├── secrets/
│   └── vnc_password.txt
└── data/
    ├── xwechat/
    └── xwechat_files/
```

仓库目录可以直接作为部署目录。`data/`、`secrets/`、`.env` 和 `config.local.toml` 已从 Git 排除，不应进入镜像层或提交到版本库。

## 3. 首次配置

```bash
git clone https://github.com/qianshulab/MimicWX-Linux.git /opt/mimicwx
cd /opt/mimicwx

cp .env.example .env
cp config.toml config.local.toml
install -d -m 700 secrets data/xwechat data/xwechat_files
openssl rand -hex 4 > secrets/vnc_password.txt
chmod 600 .env config.local.toml secrets/vnc_password.txt
```

生成 API Token：

```bash
openssl rand -hex 32
```

将生成结果写入 `config.local.toml`：

```toml
[api]
token = "YOUR_RANDOM_API_TOKEN"
```

VNC 协议只使用密码的前八个字符，因此示例生成八位随机十六进制密码。即使已设置密码，也必须把 noVNC 限制在可信网络中。

## 4. 网络配置

`.env` 默认把 API 和 noVNC 绑定到回环地址：

```dotenv
MIMICWX_BIND_IP=127.0.0.1
```

根据访问方式选择配置：

1. 通过带认证的 TLS 反向代理访问：保持 `127.0.0.1`。
2. 通过 SSH 隧道访问：保持 `127.0.0.1`。
3. 从可信局域网直接访问：填写主机的具体局域网地址。

除非主机防火墙已经严格限制来源，否则不要使用 `0.0.0.0`。原始 VNC 端口 5901 不会发布到宿主机。

| 端口 | 服务 | 认证方式 |
| ---: | --- | --- |
| 6080 | noVNC 浏览器桌面 | VNC 密码 |
| 8899 | REST 与 WebSocket API | Bearer Token，`/status` 除外 |

## 5. 镜像源与代理

默认构建使用适合中国大陆网络的依赖镜像源：

```dotenv
MIMICWX_USE_MIRROR=1
```

使用 Ubuntu 和 Rust 上游源时改为：

```dotenv
MIMICWX_USE_MIRROR=0
```

主机需要 HTTP 代理时，建议在 Docker 服务层配置，不要把代理地址、账号或密码固化在 Compose 文件或镜像中。使用 systemd 的 Docker 可以创建：

```ini
# /etc/systemd/system/docker.service.d/http-proxy.conf
[Service]
Environment="HTTP_PROXY=http://PROXY_HOST:PORT"
Environment="HTTPS_PROXY=http://PROXY_HOST:PORT"
Environment="NO_PROXY=localhost,127.0.0.1"
```

应用服务配置：

```bash
sudo systemctl daemon-reload
sudo systemctl restart docker
```

代理凭据属于敏感信息，应由宿主机的受保护服务配置保存，不得提交到仓库。

## 6. 启动与登录

构建并启动服务：

```bash
docker compose up -d --build
docker compose ps
```

打开浏览器桌面：

```text
http://HOST:6080/vnc.html?autoconnect=true&resize=scale
```

使用 `secrets/vnc_password.txt` 中的密码连接，然后在虚拟桌面内登录微信。

微信 4.0 使用兼容的进程内存扫描路径。微信 4.1 在登录期间捕获 32 字节数据库口令，按每个数据库的 salt 派生密钥，并在通过 page-1 HMAC 验证后启用。监听器会继续处理后续口令轮换和新增数据库，不需要替换容器。

## 7. 就绪检查

调用公开状态接口：

```bash
curl --fail http://HOST:8899/status
```

只有在账号已登录且 `db_available=true` 时，数据库消息接口才进入可用状态。

检查容器健康状态和近期日志：

```bash
docker compose ps
docker compose logs --tail=200 mimicwx
```

Compose 健康检查只确认 API 进程可以响应 `/status`。消息消费方仍需判断响应中的登录状态和 `db_available`。

## 8. 接口验证

设置连接参数时，应避免把真实 Token 长期保留在 shell 历史中：

```bash
export MIMICWX_BASE="http://HOST:8899"
export MIMICWX_TOKEN="YOUR_API_TOKEN"
```

验证受认证接口：

```bash
curl --fail \
  -H "Authorization: Bearer $MIMICWX_TOKEN" \
  "$MIMICWX_BASE/sessions"
```

消息接收、历史补拉、附件下载和发送流程见 [API 参考](API.zh-CN.md)。

## 9. 持久化与备份

| 路径 | 内容 |
| --- | --- |
| `data/xwechat/` | 微信账号、加密数据库和派生密钥映射 |
| `data/xwechat_files/` | 已下载或缓存的微信文件 |
| `config.local.toml` | API Token 和监听配置 |
| `.env` | 主机绑定、镜像标签与构建源配置 |
| `secrets/vnc_password.txt` | VNC 密码来源 |

文件级备份前应停止服务，或确保微信处于空闲状态：

```bash
docker compose stop mimicwx
# 在此执行宿主机备份或快照。
docker compose start mimicwx
```

备份中可能包含账号状态、凭据、联系人元数据和消息附件，必须按敏感数据保护。

## 10. 升级与回滚

升级前：

1. 备份持久化数据和配置。
2. 记录当前 Git 提交与镜像标签。
3. 为新版本使用不同的不可变镜像标签。

```bash
git pull --ff-only
```

修改 `.env` 中的 `MIMICWX_IMAGE` 后重新构建并替换服务：

```bash
docker compose build mimicwx
docker compose up -d --force-recreate mimicwx
```

清理旧镜像前必须验证：

- 微信仍保持登录，或可以重新登录。
- `/status` 返回 `db_available=true`。
- 新消息可以通过 WebSocket 或 `/messages` 接收。
- 最近附件可以下载。
- 文本回复和文件发送能够完成。

回滚时恢复原应用版本和 `MIMICWX_IMAGE`，然后重新创建服务。应用回滚不得删除 `data/`。

## 11. 安全加固

- 为 API 设置长随机 Token，疑似泄露后立即轮换。
- 使用主机防火墙限制 6080 和 8899 的来源地址。
- 远程访问应放在 TLS 与额外认证层之后。
- 不要把 noVNC 直接暴露到公网。
- 所有进入的文件均按不可信输入处理，不依赖扩展名判断类型。
- 下游系统应执行大小限制、恶意文件扫描、内容识别和沙箱解析。
- 保持宿主机、Docker Engine、基础镜像和反向代理处于受支持版本。
- 定期检查认证失败和异常发送相关日志。

## 12. 故障排查

| 现象 | 检查项 |
| --- | --- |
| noVNC 无法连接 | 检查 6080 绑定、容器状态、密码文件和 `docker compose logs mimicwx`。 |
| `db_available=false` | 确认微信已登录，等待密钥验证完成，检查密钥监听和数据库初始化日志。 |
| 构建下载缓慢 | 选择合适的 `MIMICWX_USE_MIRROR`，或配置 Docker 服务代理。 |
| API 返回 `401` | 确认 `config.local.toml` 中 Token 与 Bearer 请求头一致。 |
| 附件为 `available=false` | 等待微信完成本地写入，然后重试相同附件地址。 |
| 发送打开错误会话 | 优先使用稳定 `conversation_id`，并为同名聊天设置唯一备注。 |
| 重连后出现重复消息 | 按 `message_id` 持久化去重；接口采用至少一次投递语义。 |

提交问题时请提供 MimicWX 版本、微信 Linux 版本、Docker 版本、脱敏后的 `/status` 输出和相关日志。不要上传数据库、密钥文件、Token、联系人信息或聊天内容。
