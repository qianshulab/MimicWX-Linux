# MimicWX API 参考（中文）

> **API 版本：** v0.6 · **传输协议：** HTTP/JSON + WebSocket · **认证方式：** Bearer Token
>
> English: [API.md](API.md)

本文档对应 v0.6，说明如何通过 HTTP 与 WebSocket 接收微信文本和文件、保留“会话/发送者”关系，并把处理结果回复到原聊天。

## 目录

- [基础约定](#基础约定)
- [消息身份字段](#消息身份字段)
- [服务状态](#服务状态)
- [实时接收](#实时接收)
- [接收附件](#接收附件)
- [回复与推送](#回复与推送)
- [生产集成建议](#生产集成建议)

## 端点索引

| 方法 | 路径 | 需要认证 | 用途 |
| --- | --- | --- | --- |
| `GET` | `/status` | 否 | 服务、登录、数据库和游标状态 |
| `GET` | `/contacts` | 是 | 已解密数据库中的联系人 |
| `GET` | `/sessions` | 是 | 当前会话 |
| `GET` | `/messages` | 是 | 多调用方独立游标实时缓存 |
| `GET` | `/messages/history` | 是 | 数据库历史补拉 |
| `GET` | `/attachments/{id}` | 是 | 支持 Range 的附件流 |
| `POST` | `/messages/send` | 是 | 向会话发送文本 |
| `POST` | `/messages/reply` | 是 | 按近期消息 ID 回复 |
| `POST` | `/messages/send-file` | 是 | 发送 Base64 文件 |
| `GET` | `/ws` | 是 | WebSocket 实时消息流 |

## 基础约定

- HTTP：`http://服务地址:8899`
- WebSocket：`ws://服务地址:8899/ws`
- 编码：UTF-8 JSON
- 时间：Unix 秒
- 分页上限：500 条
- 发出文本上限：64 KiB
- 发出文件上限：Base64 解码后 100 MiB
- 错误格式：`{"error":"错误原因"}`

常见状态码：

| 状态码 | 含义 |
| ---: | --- |
| `400` | 输入、分页、文件名、Base64 数据或大小限制无效 |
| `401` | API Token 缺失或无效 |
| `404` | 近期消息不存在或附件尚不可用 |
| `416` | 附件 Range 无效或超出范围 |
| `500` | 内部处理或 I/O 失败 |
| `503` | 数据库或输入引擎当前不可用 |

除 `GET /status` 外，接口均需携带：

```http
Authorization: Bearer YOUR_API_TOKEN
```

无法设置请求头的 WebSocket 客户端可使用 `?token=URL编码后的Token`，但生产环境更建议用请求头，并在可信局域网或带 TLS 的反向代理后使用。

## 消息身份字段

每条消息都明确区分聊天对象和实际发言人：

| 字段 | 含义 |
| --- | --- |
| `message_id` | 稳定消息 ID，用于去重和回复。优先使用微信 server ID。 |
| `conversation_id` | 所属会话 ID；私聊为联系人，群聊为群。 |
| `conversation_name` | 会话当前显示名。 |
| `sender_id` | 实际发送者 ID；群聊中为具体群成员。 |
| `sender_name` | 实际发送者当前显示名。 |
| `direction` | `incoming`、`outgoing` 或 `system`。 |
| `is_group` | 是否群聊。 |
| `is_self` | 是否本账号发出。 |
| `create_time` | 消息产生时间。 |
| `parsed` | 按消息类型解析后的结构化内容。 |
| `attachment` | 文件下载信息；非文件消息为 `null`。 |

旧字段 `chat`、`chat_display_name`、`talker`、`talker_display_name` 仅用于兼容，新接入应使用 `conversation_*` 和 `sender_*`。

在私聊中，收到的消息通常同时以联系人作为会话和发送者；在群聊中，`conversation_id` 是群 ID，而 `sender_id` 是成员 ID。调用方应把回复投递到 `conversation_id`，并用 `sender_id` 关联实际发送者。

## 服务状态

### `GET /status`

无需认证：

```json
{
  "status": "已登录",
  "version": "0.6.0",
  "listen_count": 0,
  "db_available": true,
  "contacts": 42,
  "uptime_secs": 3600,
  "message_cursor": 128
}
```

只有登录状态正常且 `db_available=true` 时，才应启动消息消费。`message_cursor` 只在本次进程运行期间有效，进程重启后必须依靠历史接口补拉。

### `GET /contacts`

从已解密数据库返回联系人。

### `GET /sessions`

返回当前会话列表，可用于发现会话；业务侧仍应保存消息中的稳定 ID。

## 实时接收

### WebSocket `/ws`

数据库消息事件的 `type` 为 `db_message`，其余字段与 `/messages` 中的消息项相同，并额外带实时 `cursor`。服务每 30 秒发送 Ping。

客户端要求：

1. 断线指数退避重连；
2. 按 `message_id` 持久化去重；
3. WebSocket 中断期间通过历史接口补拉；
4. 用有界队列接收，避免下游处理变慢时无限占用内存。

### `GET /messages`

读取进程内实时缓存。每个调用方保存自己的 `after`，不会抢走其他调用方的消息。

| 参数 | 默认值 | 说明 |
| --- | ---: | --- |
| `after` | `0` | 只返回 `cursor > after`。 |
| `limit` | `100` | 1–500。 |
| `chat` | 无 | 匹配会话 ID 或名称。 |
| `sender` | 无 | 匹配发送者 ID 或名称。 |
| `direction` | 无 | 按方向过滤。 |

```json
{
  "messages": [{"cursor":129,"message_id":"wx:123456789"}],
  "next_cursor": 129,
  "latest_cursor": 129,
  "has_more": false
}
```

同一次进程运行中，下次请求传 `after=next_cursor`。`has_more=true` 时立即翻页。

### `GET /messages/history`

直接读取加密微信数据库，用于首次导入、断线和重启后的补拉。

| 参数 | 默认值 | 说明 |
| --- | ---: | --- |
| `since` | `0` | 起始 Unix 秒，包含该秒；一轮分页期间保持不变。 |
| `offset` | `0` | 此 `since` 结果集内的稳定偏移，最大 100000。 |
| `limit` | `100` | 1–500。 |
| `chat` | 无 | 会话 ID/名称。 |
| `sender` | 无 | 发送者 ID/名称。 |
| `direction` | 无 | 消息方向。 |

结果按时间从旧到新排列。响应包含 `messages`、`next_offset`、`checkpoint_time`、兼容字段 `next_since` 和 `has_more`。分页时固定原始 `since`，使用返回的 `next_offset` 继续请求，直到 `has_more=false`。`sender`/`direction` 等可选过滤是在基础页扫描后应用，因此返回数量可能少于 `limit`，甚至本页为空但 `has_more=true`，仍需继续翻页。

最后一页完成后，把 `checkpoint_time` 持久化为下一轮包含式 `since`。同一秒可能有多条消息，程序崩溃后也可能安全重放，所以始终按 `message_id` 去重。

### 推荐的可靠消费流程

1. 消费方持久化已处理的 `message_id` 和最新 `create_time`。
2. 先建立 WebSocket 并暂存实时事件。
3. 调用 `/messages/history?since=上次时间&offset=0`；固定 `since`，按 `next_offset` 翻页并跳过已处理 ID。
4. 再处理暂存的 WebSocket 事件，同样按 ID 去重。
5. 正常持续接收，并定期持久化处理水位。
6. 断线或 MimicWX 重启后回到第 2 步。

这形成至少一次投递语义：宁可重复，不静默丢失。MimicWX 只在本地持久化数据库表水位，不额外落盘明文消息；业务级持久化和幂等由消费方负责。

旧接口 `GET /messages/new` 继续兼容，但无参数时为共享单消费者，不建议新系统使用。

## 接收附件

文件消息的 `attachment` 示例：

```json
{
  "id": "URL_SAFE_OPAQUE_ID",
  "name": "archive.zip",
  "size": 1048576,
  "extension": "zip",
  "md5": "可选MD5",
  "available": true,
  "download_url": "/attachments/URL_SAFE_OPAQUE_ID"
}
```

### `GET /attachments/{id}`

以 `application/octet-stream` 流式返回文件，强制下载，禁止 MIME 嗅探和缓存，并支持单段 `Range: bytes=...` 断点续传。

附件 ID 是不含服务器路径的元数据定位符。后端只允许在当前微信账号的附件白名单目录中查找，并拒绝路径穿越、软链接逃逸和不匹配的文件。

`available=false` 表示微信暂未把文件写入本地；下载接口每次都会重新解析，因此调用方可以退避重试。Office 文档、ZIP、APK、EXE 等均只作为字节返回，MimicWX 不执行它们。下游系统应在处理前执行大小限制、哈希记录、病毒扫描、真实文件类型识别和沙箱解析。

```bash
curl --fail --location \
  -H "Authorization: Bearer $MIMICWX_TOKEN" \
  -o attachment.bin \
  "http://服务地址:8899/attachments/URL_SAFE_OPAQUE_ID"
```

## 回复与推送

### `POST /messages/send`

```json
{
  "conversation": "会话ID或唯一的微信显示名/备注名",
  "text": "文档已完成入库。",
  "at": []
}
```

响应会返回解析后的 `conversation_id`、`conversation_name`、`sent` 和 `verified`。若存在同名聊天，建议在微信中设置唯一备注；底层 Linux 客户端最终仍需通过可见搜索结果打开会话。

### `POST /messages/reply`

```json
{
  "message_id": "wx:123456789",
  "text": "处理完成。",
  "mention_sender": true
}
```

它会把结果发回消息所属会话；群聊默认 @ 原发送者。目标消息必须仍在当前进程的实时缓存中。若服务已重启或消息过旧，请使用业务侧保存的 `conversation_id` 调用 `/messages/send`，群聊需要时显式填写 `at`。

### `POST /messages/send-file`

别名：`POST /send_file`。

```json
{
  "conversation": "会话ID或名称",
  "name": "answer.pdf",
  "file": "BASE64_BYTES"
}
```

文件以 0600 权限写入临时目录，通过微信原生文件选择器交给微信，保留一段时间供微信异步读取后删除。文件名含路径分隔符、控制字符或目录穿越形式时会被拒绝。

兼容接口：

- `POST /send`：旧版文本发送 `{to,text,at}`
- `POST /send_image`：旧版 Base64 图片发送
- `POST /chat`：打开聊天
- `GET|POST|DELETE /listen`：管理独立监听窗口

## 生产集成建议

- 业务主键：`message_id`。
- 路由主键：`conversation_id`；提问者主键：`sender_id`。
- 对 `direction=outgoing` 做过滤或单独归档，避免机器人消费自己的回复形成循环。
- 先把消息放入持久化队列，再异步下载和解析附件。
- 记录附件哈希，不按扩展名信任内容类型。
- 回复时保存原 `message_id`、业务任务 ID 和回复状态，保证重试幂等。
- 对发送设置速率限制和人工兜底，避免异常循环刷屏。
- 不要把 API/noVNC 直接暴露到公网；Token、微信数据和附件备份均按敏感数据保护。

英文版及 Python 客户端示例见 [API.md](API.md)。
