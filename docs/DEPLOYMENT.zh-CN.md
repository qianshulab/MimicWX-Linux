# Docker 部署指南

> **适用版本：** MimicWX-Linux v0.6.x · **运行平台：** x86-64 Linux、Docker Compose v2
>
> English: [DEPLOYMENT.md](DEPLOYMENT.md)

Docker 镜像包含微信 Linux 客户端、XFCE 桌面、noVNC、MimicWX 服务和数据库密钥提取组件。本指南涵盖 x86-64 Linux 主机上的配置、启动、持久化存储和维护。

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
- 支持 Docker Compose 的 Docker Engine，建议使用 24 或更高版本
- Docker Compose v2
- 可访问微信软件包、Ubuntu 软件源、Rust 工具链和 crate 注册表

容量规划可从 2 GB 可用内存、5 GB 运行磁盘空间起步，并为消息和附件预留额外容量。这些数值是规划建议，未经最低资源验证。编译 Rust 依赖和构建镜像层还需要额外的内存与磁盘空间，实际占用取决于构建缓存和业务负载。

容器需要以下 Linux capability：

- `SYS_PTRACE`：自动捕获数据库登录口令
- `SYS_ADMIN`：现有数据库变更通知机制

随项目提供的 Compose 配置还设置了 `seccomp:unconfined` 和 `apparmor:unconfined`。这些选项为进程检查和桌面集成放宽了容器隔离限制，部署前应按主机安全要求评估；当前配置不应作为运行不可信负载的沙箱。

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

Token 未配置或为空时，API 认证不会启用。包含真实凭据的 `config.local.toml` 应与版本库中的配置模板分开保存。

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
| 8899 | REST 与 WebSocket API | 配置后启用 Bearer Token；`/status` 公开 |

## 5. 镜像源与代理

默认构建使用适合中国大陆网络的依赖镜像源：

```dotenv
MIMICWX_USE_MIRROR=1
```

使用 Ubuntu 和 Rust 上游源时改为：

```dotenv
MIMICWX_USE_MIRROR=0
```

拉取镜像与镜像构建期间的下载使用不同的代理配置。[Docker 服务代理](https://docs.docker.com/engine/daemon/proxy/)负责访问镜像仓库。使用 systemd 的主机可以创建服务配置片段：

```ini
# /etc/systemd/system/docker.service.d/http-proxy.conf
[Service]
Environment="HTTP_PROXY=http://PROXY_HOST:PORT"
Environment="HTTPS_PROXY=http://PROXY_HOST:PORT"
Environment="NO_PROXY=localhost,127.0.0.1"
```

重新加载配置并重启 Docker。此操作影响宿主机的 Docker 服务，可能中断其他容器：

```bash
sudo systemctl daemon-reload
sudo systemctl restart docker
```

构建期间的软件包下载使用 [Docker 预定义代理构建参数](https://docs.docker.com/build/building/variables/#proxy-arguments)：

```bash
docker compose build \
  --build-arg HTTP_PROXY=http://PROXY_HOST:PORT \
  --build-arg HTTPS_PROXY=http://PROXY_HOST:PORT \
  mimicwx
```

代理地址必须能从构建容器访问；其中的 `127.0.0.1` 不指向宿主机。不要把代理凭据提交到仓库或写入镜像，需要凭据时使用宿主机的受保护配置。

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

启动脚本为微信 4.0 选择进程内存密钥扫描器；为微信 4.1 启动登录监听器，在登录期间捕获 32 字节数据库口令，按每个数据库的 salt 派生密钥，并校验首页 HMAC。后台监测进程检查已有密钥和新增数据库。

缓存口令验证失败时，可能需要退出并重新登录微信才能捕获新口令。兼容性取决于实际安装的微信版本和数据库格式；客户端更新可能需要同步更新提取器。如果已经登录但数据库仍不可用，应检查相关日志。

## 7. 就绪检查

调用公开状态接口：

```bash
curl --fail http://HOST:8899/status
```

消费消息前应确认账号已登录且 `db_available=true`。该字段表示数据库管理器已经初始化，不代表每次数据库操作都会成功。部署检查还应包含受认证接口请求和新消息接收。

检查容器健康状态和近期日志：

```bash
docker compose ps
docker compose logs --tail=200 mimicwx
```

Compose 健康检查只确认 API 进程可以响应 `/status`。消息消费方仍需判断响应中的登录状态和 `db_available`。

### 主机重启恢复

Compose 使用 `restart: always`。主机重启后，Docker 服务启动且挂载目录、配置的主机地址可用时，可以恢复已有容器。需确保 Docker 本身已设为开机启动。`docker compose down` 会删除容器，在重新创建之前，重启策略不再生效。

API 先于微信登录和数据库初始化启动；API 启动后，`/status` 可以报告待登录状态。VNC、微信或 noVNC 进程消失时，容器内看门狗会让容器退出，由 Docker 重新启动整组服务。仅出现 `unhealthy` 状态不会触发 Docker 重启，看门狗也不能检测所有进程卡住的情况。

重启后微信可能要求扫码或手机确认登录，通过 noVNC 完成即可。密钥监听器随后尝试捕获或校验数据库密钥，恢复消费前应检查 `/status` 和日志。

创建或启动服务：

```bash
docker compose up -d mimicwx
```

重启已有服务并查看日志：

```bash
docker compose restart mimicwx
docker compose logs --tail=200 mimicwx
```

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
| `data/xwechat/` | 微信配置、捕获的数据库口令和派生密钥映射 |
| `data/xwechat_files/` | 账号目录、加密数据库、附件和消息扫描水位 |
| `config.local.toml` | API Token 和监听配置 |
| `.env` | 主机绑定、镜像标签与构建源配置 |
| `secrets/vnc_password.txt` | VNC 密码来源 |

文件级备份前应停止服务，使数据库及其 WAL 文件在复制期间保持一致：

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
3. 为新版本使用独立镜像标签，并保留旧镜像用于回滚。

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

回滚时恢复原应用版本和 `MIMICWX_IMAGE`，使用保留的旧镜像重新创建服务：

```bash
docker compose up -d --no-build --force-recreate mimicwx
```

应用回滚不得删除 `data/`。回滚镜像不会撤销新版微信客户端对数据库的修改，必要时需恢复兼容的备份。

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
| 构建下载缓慢 | 选择合适的 `MIMICWX_USE_MIRROR`；镜像拉取检查 Docker 服务代理，软件包下载检查构建代理参数。 |
| API 返回 `401` | 确认 `config.local.toml` 中 Token 与 Bearer 请求头一致。 |
| 附件为 `available=false` | 在微信客户端确认文件已下载且仍然存在。API 不能触发下载，缺失或无法匹配的文件会持续不可用。 |
| 发送打开错误会话 | 优先使用稳定 `conversation_id`，并为同名聊天设置唯一备注。 |
| 重连后出现重复消息 | 按 `message_id` 持久化去重；历史补拉和重叠时间窗口可能返回已经处理过的记录。 |
| 容器运行中但无法接收消息 | 检查登录状态、`db_available` 和日志；容器健康状态不等于完整的消息接收验证。 |

提交问题时请提供 MimicWX 版本、微信 Linux 版本、Docker 版本、脱敏后的 `/status` 输出和相关日志。不要上传数据库、密钥文件、Token、联系人信息或聊天内容。
