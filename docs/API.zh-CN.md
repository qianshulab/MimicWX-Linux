# MimicWX API 参考（中文）

> **API 版本：** v0.6 · **传输协议：** HTTP/JSON + WebSocket · **认证方式：** Bearer Token
>
> English: [API.md](API.md)

MimicWX 通过 HTTP 与 WebSocket 提供微信消息、会话信息、本地附件访问和客户端发送能力。本参考适用于 v0.6 接口，包含请求格式、返回字段和集成限制。

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
| `GET` | `/messages` | 是 | 由调用方管理游标的实时缓存 |
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
- 默认 JSON 请求体上限：2 MiB，包含 Base64 数据和 JSON 结构
- 文件处理层校验上限：Base64 解码后 100 MiB；当前实际请求先受较小的 JSON 请求体限制
- 业务处理错误通常返回 `{"error":"错误原因"}`。认证和请求解析错误可能返回空响应或纯文本，应先检查 HTTP 状态，再解析 JSON。

常见状态码：

| 状态码 | 含义 |
| ---: | --- |
| `400` | 输入、分页、文件名、Base64 数据或大小限制无效 |
| `401` | API Token 缺失或无效 |
| `404` | 近期消息不存在或附件尚不可用 |
| `413` | 请求体超过服务端或反向代理限制 |
| `416` | 附件 Range 无效或超出范围 |
| `500` | 内部处理或 I/O 失败 |
| `503` | 数据库或输入引擎当前不可用 |

在应用配置中设置非空的 `[api].token` 后，除 `GET /status` 外的接口均需携带：

```http
Authorization: Bearer YOUR_API_TOKEN
```

Token 未配置或为空时，认证不会启用。端点索引中的认证要求以已启用 Token 为前提。

无法设置请求头的 WebSocket 客户端可使用 `?token=URL编码后的Token`，但生产环境更建议用请求头，并在可信局域网或带 TLS 的反向代理后使用。

## 消息身份字段

数据库消息记录区分所属会话和实际发送者。存储与路由应使用 ID；显示名可能变化，也可能重复。

| 字段 | 含义 |
| --- | --- |
| `message_id` | 去重和回复 ID。优先使用 `wx:<server_id>`；不可用时使用会话、本地记录 ID 与创建时间派生的 `local:<hash>`。 |
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
  "version": "0.6.1",
  "listen_count": 0,
  "db_available": true,
  "contacts": 42,
  "uptime_secs": 3600,
  "message_cursor": 128
}
```

消息消费前应确认登录状态正常且 `db_available=true`。API 先于登录和数据库初始化启动，HTTP 请求成功仅表示状态接口可以响应。`db_available=true` 表示数据库管理器已经初始化，并不代表本次请求重新验证了所有数据库和附件。

`message_cursor` 只在本次进程运行期间有效，重启后会重置。API 不提供持久化实例 ID；游标或运行时长变小可作为重启线索，但不能单独作为判断依据。调用方每次重连后都应通过历史接口补拉。

### `GET /contacts`

从已解密数据库返回联系人。

### `GET /sessions`

返回当前会话列表，可用于发现会话；业务侧仍应保存消息中的稳定 ID。

## 实时接收

### WebSocket `/ws`

数据库消息事件的 `type` 为 `db_message`，其余字段与 `/messages` 中的消息项相同，包含实时 `cursor`。WebSocket 还可能包含其他类型事件，处理数据库消息前应按 `type` 过滤。服务每 30 秒发送 Ping，连接本身不提供历史重放。

客户端要求：

1. 断线指数退避重连；
2. 按 `message_id` 持久化去重；
3. WebSocket 中断期间通过历史接口补拉；
4. 用有界队列接收，避免下游处理变慢时无限占用内存。

### `GET /messages`

读取进程内最近最多 4,096 条数据库消息。每个调用方保存自己的 `after`，读取不会删除其他调用方可见的消息。服务端不持久化客户端游标，也不记录消费确认。

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

同一次进程运行中，下次请求传 `after=next_cursor`。`has_more=true` 时立即翻页；没有匹配消息时，`next_cursor` 保持不变。

缓存满后会移除最旧的消息，进程重启会清空全部缓存。`has_more=false` 只表示当前缓存中没有更多匹配项，不代表已经收齐全部消息。连接中断或可能超出缓存容量时，应通过历史接口补拉。

### `GET /messages/history`

直接读取加密微信数据库，用于首次导入、断线和重启后的补拉。

| 参数 | 默认值 | 说明 |
| --- | ---: | --- |
| `since` | `0` | 起始 Unix 秒，包含该秒；一轮分页期间保持不变。 |
| `offset` | `0` | 相同 `since` 和过滤条件下的结果偏移，最大 100000。 |
| `limit` | `100` | 1–500。 |
| `chat` | 无 | 会话 ID/名称。 |
| `sender` | 无 | 发送者 ID/名称。 |
| `direction` | 无 | 消息方向。 |

结果按时间从旧到新排列。响应包含 `messages`、`next_offset`、`checkpoint_time`、兼容字段 `next_since` 和 `has_more`。分页时固定原始 `since`，使用返回的 `next_offset` 继续请求，直到 `has_more=false`。`sender`/`direction` 等可选过滤是在基础页扫描后应用，因此返回数量可能少于 `limit`，甚至本页为空但 `has_more=true`，仍需继续翻页。

最后一页的处理结果持久化后，把 `checkpoint_time` 保存为下一轮包含式 `since`。同一秒可能有多条消息；处理完成但检查点尚未保存时发生崩溃，也会导致重复读取，因此需始终按 `message_id` 去重。

历史接口只能查询本地微信数据库中仍保留的消息。分页不是数据库快照，延迟同步或删除操作可能使分页期间的结果变化。补拉时可保留一段重叠时间窗口，并对重叠记录去重；本地客户端没有保存的记录无法通过此接口恢复。

### 消费检查点与恢复

1. 消费方持久化已处理的 `message_id` 和最新 `create_time`。
2. 先建立 WebSocket 并暂存实时事件。
3. 调用 `/messages/history?since=上次时间&offset=0`；固定 `since`，按 `next_offset` 翻页并跳过已处理 ID。
4. 再处理暂存的 WebSocket 事件，同样按 ID 去重。
5. 正常持续接收，并定期持久化处理水位。
6. 断线或 MimicWX 重启后回到第 2 步。

这是客户端恢复策略，不构成至少一次投递保证。MimicWX 持久化数据库扫描水位，但 API 不提供持久化事件队列或消费确认。补拉依赖消息仍保留在本地微信数据库中；业务数据持久化、检查点和幂等处理由调用方负责。

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

`available=false` 表示未找到匹配的本地文件。文件可能尚未下载、已经删除，或无法与消息元数据匹配。下载接口每次都会重新检查，但不会触发微信下载；应在微信客户端中确认文件已下载后再重试。

此端点适用于解析为文件类型的消息，不是图片、语音、视频、链接或小程序内容的通用下载接口。Office 文档、ZIP、APK、EXE 等文件在本地可用时可按字节返回，端点不会执行文件。下游系统应在处理前执行大小限制、内容类型识别、扫描和沙箱解析。

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

响应示例：

```json
{
  "sent": true,
  "verified": true,
  "message": "sent",
  "conversation_id": "wxid_or_group_id",
  "conversation_name": "联系人 A",
  "reply_to_message_id": null
}
```

即使传入 `conversation_id`，底层 Linux 客户端最终仍通过显示名对应的搜索结果打开会话。同名聊天应在微信中设置唯一备注。

`at` 接收群成员的显示名，不是成员 ID。`sent` 表示客户端发送操作结果，`verified` 表示本地数据库事件或无障碍接口的确认结果；二者均不是收件人送达或已读回执。发送接口不支持幂等键，超时后直接重试可能重复发送。

### `POST /messages/reply`

```json
{
  "message_id": "wx:123456789",
  "text": "处理完成。",
  "mention_sender": true
}
```

`mention_sender` 默认为 `true`。对于收到的群消息，发送者名称可解析时会请求 @ 该成员。目标消息必须仍在当前进程的实时缓存中。若服务已重启或消息过旧，使用业务侧保存的 `conversation_id` 调用 `/messages/send`，群聊需要时显式填写 `at`。此接口向原会话发送一条新消息，不生成微信原生引用回复。

### `POST /messages/send-file`

别名：`POST /send_file`。

```json
{
  "conversation": "会话ID或名称",
  "name": "answer.pdf",
  "file": "BASE64_BYTES"
}
```

处理函数校验解码后文件不超过 100 MiB，但当前默认 JSON 请求体限制为 2 MiB。计入 Base64 与 JSON 开销后，实际文件需略小于 1.5 MiB；反向代理还可能施加更小的限制。请求体上限来自 [Axum 默认配置](https://docs.rs/axum/0.8/axum/extract/struct.DefaultBodyLimit.html)。

文件以 0600 权限写入临时目录，通过微信原生文件选择器交给微信，保留一段时间供微信异步读取后删除。文件名含路径分隔符、控制字符或目录穿越形式时会被拒绝。目前没有 multipart 或流式上传接口。

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
- 保存原 `message_id`、`conversation_id`、`sender_id`、处理结果和发送状态；发送结果不确定时，先核对再重试。业务任务去重不能单独保证微信发送幂等。
- 对发送设置速率限制和人工兜底，避免异常循环刷屏。
- 不要把 API/noVNC 直接暴露到公网；Token、微信数据和附件备份均按敏感数据保护。

英文版及 Python 客户端示例见 [API.md](API.md)。示例仅打印实时事件，不包含重连、历史补拉和检查点持久化；实际集成需实现这些行为，并保留失败任务供重试或人工处理。
