# MimicWX API Reference

> **API version:** v0.6 · **Transport:** HTTP/JSON + WebSocket · **Authentication:** Bearer Token
>
> 中文版本：[API.zh-CN.md](API.zh-CN.md)

MimicWX exposes WeChat messages, conversation metadata, local attachments, and client-driven sending through HTTP and WebSocket. This reference covers the v0.6 interface and its integration constraints.

## Contents

- [Conventions](#1-conventions)
- [Message identity model](#2-message-identity-model)
- [Health and bootstrap](#3-health-and-bootstrap)
- [Receiving messages](#4-receiving-messages)
- [Attachments](#5-attachments)
- [Sending and replying](#6-sending-and-replying)
- [Minimal Python consumer](#7-minimal-python-consumer)
- [Security notes](#8-security-notes)

## Endpoint index

| Method | Path | Authentication | Purpose |
| --- | --- | --- | --- |
| `GET` | `/status` | No | Service, login, database, and cursor status |
| `GET` | `/contacts` | Yes | Contacts from the decrypted database |
| `GET` | `/sessions` | Yes | Current conversations |
| `GET` | `/messages` | Yes | Real-time buffer with client-managed cursors |
| `GET` | `/messages/history` | Yes | Database history catch-up |
| `GET` | `/attachments/{id}` | Yes | Attachment stream with Range support |
| `POST` | `/messages/send` | Yes | Send text to a conversation |
| `POST` | `/messages/reply` | Yes | Reply using a recent message ID |
| `POST` | `/messages/send-file` | Yes | Send a Base64-encoded file |
| `GET` | `/ws` | Yes | Real-time WebSocket stream |

## 1. Conventions

- HTTP base URL: `http://HOST:8899`
- WebSocket URL: `ws://HOST:8899/ws`
- JSON encoding: UTF-8
- Timestamps: Unix seconds
- Maximum page size: 500 messages
- Maximum outbound text: 64 KiB
- JSON request body limit: 2 MiB in the default server configuration, including Base64 data and JSON overhead
- Outbound file validation limit: 100 MiB after Base64 decoding; the smaller JSON request limit currently takes precedence
- Handler errors generally use `{"error":"reason"}`. Authentication and request-extraction errors may have an empty or plain-text body; check the HTTP status before parsing JSON.

Common status codes:

| Status | Meaning |
| ---: | --- |
| `400` | Invalid input, pagination, file name, Base64 data, or size limit |
| `401` | Missing or invalid API Token |
| `404` | Unknown recent message or unavailable attachment |
| `413` | Request body exceeds the server or reverse-proxy limit |
| `416` | Invalid or unsatisfiable attachment Range |
| `500` | Internal processing or I/O failure |
| `503` | Database or input engine is not currently available |

Set a non-empty `[api].token` in the application configuration to enable authentication. With a token configured, every endpoint except `GET /status` requires:

```http
Authorization: Bearer YOUR_API_TOKEN
```

An absent or empty token disables authentication. The endpoint index assumes authentication is enabled.

For WebSocket clients that cannot set an authorization header, use `ws://HOST:8899/ws?token=YOUR_URL_ENCODED_TOKEN`. Query-string authentication can leak through URLs and logs, so prefer a header and TLS-capable reverse proxy whenever possible.

## 2. Message identity model

Database message records distinguish the conversation from the sender. Use IDs for storage and routing; display names can change and need not be unique.

| Field | Meaning |
| --- | --- |
| `message_id` | Deduplication/reply ID: `wx:<server_id>` when available; otherwise `local:<hash>` derived from the conversation, local row ID, and creation time. |
| `conversation_id` | Chat identity. For a private chat this identifies the contact; for a group it identifies the group. |
| `conversation_name` | Current display name of the chat. |
| `sender_id` | Actual message sender. In a group this identifies the member who spoke. |
| `sender_name` | Current display name of the sender. |
| `direction` | `incoming`, `outgoing`, or `system`. |
| `is_group` | Whether the conversation is a group. |
| `is_self` | Whether this account sent the message. |
| `create_time` | WeChat message creation time, in Unix seconds. |
| `parsed` | Tagged structured message content. |
| `attachment` | Download metadata for a file message, otherwise `null`. |

The older `chat`, `chat_display_name`, `talker`, and `talker_display_name` fields are retained for backward compatibility. New integrations should use the explicit `conversation_*` and `sender_*` fields.

Example private message:

```json
{
  "message_id": "wx:123456789",
  "create_time": 1770000000,
  "conversation_id": "wxid_contact",
  "conversation_name": "Customer A",
  "sender_id": "wxid_contact",
  "sender_name": "Customer A",
  "direction": "incoming",
  "is_group": false,
  "is_self": false,
  "parsed": {"type":"Text","data":{"text":"Please index this file"}},
  "attachment": null
}
```

For a group message, `conversation_id` is the group ID and `sender_id` is the member ID. This distinction lets a consumer route a response back to the group while retaining the identity of the person who sent the message.

## 3. Health and bootstrap

### `GET /status`

No authentication is required. A service should only ingest messages when `status` reports a logged-in state and `db_available` is `true`.

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

The API starts before login and database initialization. A successful HTTP response confirms that the status endpoint is reachable. `db_available=true` means the database manager has been initialized; it is not a fresh validation of every database or attachment.

`message_cursor` is process-local and resets when the service restarts. The API does not expose a persistent instance ID. A decreased cursor or uptime can indicate a restart, but clients should perform history catch-up after every reconnect rather than rely on these values alone.

### `GET /contacts`

Returns contacts resolved from the encrypted WeChat database.

### `GET /sessions`

Returns the current WeChat session list. This endpoint is useful for discovery; stable message routing should still persist the IDs returned with messages.

## 4. Receiving messages

### WebSocket `/ws`

Database events have `type: "db_message"` and otherwise contain the same fields as an item returned by `GET /messages`, including `cursor`. Other event types may also appear; filter by `type` before processing database messages.

The server sends a WebSocket Ping every 30 seconds. Clients must reconnect with exponential backoff and deduplicate with `message_id`. The WebSocket stream does not replay missed events by itself.

### `GET /messages`

Returns up to the latest 4,096 database messages held in memory. Each caller stores its own cursor; reading messages does not remove them for other callers. The server does not persist client cursors or acknowledgements.

Query parameters:

| Parameter | Default | Description |
| --- | ---: | --- |
| `after` | `0` | Return entries with `cursor > after`. |
| `limit` | `100` | 1–500. |
| `chat` | unset | Match conversation ID or conversation name. |
| `sender` | unset | Match sender ID or sender name. |
| `direction` | unset | `incoming`, `outgoing`, or `system`. |

```json
{
  "messages": [
    {
      "cursor": 129,
      "message_id": "wx:123456789",
      "conversation_id": "wxid_contact",
      "sender_id": "wxid_contact",
      "direction": "incoming"
    }
  ],
  "next_cursor": 129,
  "latest_cursor": 129,
  "has_more": false
}
```

Use `after=next_cursor` for the next request to the same service process. If `has_more` is true, request the next page immediately. If no messages match the filters, `next_cursor` stays unchanged.

Old entries are discarded when the buffer fills, and all entries are lost on restart. `has_more=false` means no further matching entries are currently buffered; it does not establish that the caller has received every message. Use history catch-up after an interruption or when the buffer may have overrun.

### `GET /messages/history`

Reads from the encrypted WeChat database. Use this endpoint for initial import and reconnect/restart catch-up.

Query parameters:

| Parameter | Default | Description |
| --- | ---: | --- |
| `since` | `0` | Inclusive Unix timestamp. Keep it unchanged while paging. |
| `offset` | `0` | Offset for the same `since` value and filters; maximum 100000. |
| `limit` | `100` | 1–500. |
| `chat` | unset | Match conversation ID or name. |
| `sender` | unset | Match sender ID or name. |
| `direction` | unset | Direction filter. |

Results are ordered from oldest to newest. The response contains `messages`, `next_offset`, `checkpoint_time`, the compatibility alias `next_since`, and `has_more`. Keep the original `since` fixed and request the next page with `offset=next_offset` until `has_more=false`. Optional sender/direction filters are applied after scanning a base page, so a page can contain fewer than `limit` messages or even be empty while `has_more=true`.

After the final page has been processed durably, persist `checkpoint_time` as the next run's inclusive `since`. Deduplicate by `message_id`: several messages can share the same second, and a crash between processing and checkpointing can replay them.

History is limited to messages retained in the local WeChat database. Pagination is not a database snapshot; delayed synchronization or deletion can change results between requests. Use an overlapping time window when catching up and deduplicate the overlap. This endpoint does not recover records absent from the local client.

### Consumer checkpoints and recovery

1. Persist every processed `message_id` and the latest `create_time` in the consuming service.
2. Connect the WebSocket and temporarily buffer live events.
3. Call `/messages/history?since=LAST_CREATE_TIME&offset=0`; while `has_more=true`, keep `since` fixed and advance to `next_offset`.
4. Drain the buffered WebSocket events, again deduplicating by `message_id`.
5. Continue consuming WebSocket events; periodically checkpoint `create_time` and processed IDs.
6. After a disconnect or service restart, repeat from step 2.

This is a client-side recovery strategy, not an at-least-once delivery guarantee. MimicWX persists database scan watermarks, but the API has no durable event queue or consumer acknowledgements. Recovery depends on the messages remaining available in the local WeChat database. The consumer is responsible for durable storage, checkpoints, and idempotent processing.

`GET /messages/new` remains available for old clients. Its no-parameter mode is a single shared consumer and should not be used for new multi-consumer integrations.

## 5. Attachments

A parsed file message can contain:

```json
{
  "attachment": {
    "id": "URL_SAFE_OPAQUE_ID",
    "name": "archive.zip",
    "size": 1048576,
    "extension": "zip",
    "md5": "optional-md5",
    "available": true,
    "download_url": "/attachments/URL_SAFE_OPAQUE_ID"
  }
}
```

### `GET /attachments/{id}`

Streams the file as `application/octet-stream`. The response forces download, sends `X-Content-Type-Options: nosniff`, disables caching, and supports a single `Range: bytes=...` request for resumable downloads.

The opaque ID contains file metadata, not a server path. Resolution is restricted to the active WeChat account's approved attachment directories; path traversal and symbolic-link escapes are rejected.

`available: false` means no matching local file was resolved. The file may not have been downloaded, may have been removed, or may not match the message metadata. The endpoint rechecks availability on every request, but does not instruct WeChat to download the file. Retry after the file is available in the WeChat client.

This endpoint serves parsed file messages. It does not provide a general download mechanism for image, voice, video, link, or mini-program content. Office documents, archives, APKs, and executables can be returned as bytes when their local files are available; the endpoint does not execute them. Consumers should apply file-size limits, content-type detection, scanning, and sandboxed parsing before indexing.

Example:

```bash
curl --fail --location \
  -H "Authorization: Bearer $MIMICWX_TOKEN" \
  -o attachment.bin \
  "http://HOST:8899/attachments/URL_SAFE_OPAQUE_ID"
```

## 6. Sending and replying

### `POST /messages/send`

Send text to a conversation. `conversation` accepts a known `conversation_id` or an unambiguous WeChat display/remark name.

```json
{
  "conversation": "wxid_or_group_id",
  "text": "The document has been indexed.",
  "at": []
}
```

Response:

```json
{
  "sent": true,
  "verified": true,
  "message": "sent",
  "conversation_id": "wxid_or_group_id",
  "conversation_name": "Customer A",
  "reply_to_message_id": null
}
```

Use unique WeChat remarks when two chats have the same display name, because the Linux client UI ultimately opens a conversation by its visible search result.

`at` contains group members' display names, not their IDs. `sent` reports the result of the client send operation; `verified` reports confirmation from local database events or accessibility inspection. Neither is a recipient delivery/read receipt. Sending endpoints do not accept an idempotency key: retrying after a timeout can send the message again.

### `POST /messages/reply`

Reply to the original conversation of a recent real-time message:

```json
{
  "message_id": "wx:123456789",
  "text": "Indexing finished: 128 passages created.",
  "mention_sender": true
}
```

`mention_sender` defaults to `true`. For an incoming group message with a resolved sender name, it requests a mention of that member. The referenced message must still be present in the process's live buffer. If it is not, use the persisted `conversation_id` with `/messages/send` and explicitly populate `at` when needed. This endpoint routes a new message to the original conversation; it does not create a native WeChat quoted reply.

### `POST /messages/send-file`

Alias: `POST /send_file`.

```json
{
  "conversation": "wxid_or_group_id",
  "name": "answer.pdf",
  "file": "BASE64_BYTES"
}
```

The handler validates a maximum decoded size of 100 MiB, but the current default JSON body limit is 2 MiB. Base64 and JSON overhead reduce the effective file size to slightly less than 1.5 MiB; a reverse proxy can impose a lower limit. The request-body limit is inherited from [Axum's default configuration](https://docs.rs/axum/0.8/axum/extract/struct.DefaultBodyLimit.html).

Files are written with owner-only permissions, selected through WeChat's native file chooser, and deleted after a grace period. File names containing path separators or control characters are rejected. There is no multipart or streaming upload endpoint.

### Compatibility endpoints

- `POST /send`: legacy text send using `{ "to", "text", "at" }`
- `POST /send_image`: legacy Base64 image send
- `POST /chat`: open a chat
- `GET|POST|DELETE /listen`: manage independent listening windows

## 7. Minimal Python consumer

```python
import asyncio
import json
import os
import aiohttp

BASE = os.environ["MIMICWX_BASE"].rstrip("/")
TOKEN = os.environ["MIMICWX_TOKEN"]
WS = BASE.replace("http://", "ws://").replace("https://", "wss://")

async def run():
    headers = {"Authorization": f"Bearer {TOKEN}"}
    async with aiohttp.ClientSession(headers=headers) as session:
        async with session.ws_connect(f"{WS}/ws", autoping=True) as socket:
            async for frame in socket:
                if frame.type != aiohttp.WSMsgType.TEXT:
                    continue
                event = json.loads(frame.data)
                if event.get("type") != "db_message":
                    continue
                print(event["message_id"], event["conversation_id"], event["sender_id"])

asyncio.run(run())
```

This example prints live events only. It does not implement reconnects, history catch-up, or durable checkpoints. Integrations should implement those behaviors, use bounded queues, and retain failed processing tasks for retry or review.

Filter or separately archive `direction=outgoing` to avoid consuming the application's own replies. Store the source `message_id`, `conversation_id`, `sender_id`, processing result, and send status together. Reconcile uncertain sends before retrying; local task deduplication alone cannot make an external send idempotent.

## 8. Security notes

- Keep the API on a trusted private network or behind an authenticated TLS reverse proxy.
- Always configure a long random API token; never commit it to Git.
- Do not expose noVNC or the API directly to the public Internet.
- Treat incoming Office files, archives, APKs, and executables as untrusted input.
- Store downloaded files outside web roots and parse them in a sandbox.
- Rotate the API token after accidental disclosure.
