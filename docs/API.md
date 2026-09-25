# MimicWX API Reference

This document describes the v0.6 HTTP and WebSocket interface for using WeChat as a bidirectional data channel. Consumers can ingest messages and files, preserve conversation and sender identity, and send processing results back to the originating chat.

## 1. Conventions

- HTTP base URL: `http://HOST:8899`
- WebSocket URL: `ws://HOST:8899/ws`
- JSON encoding: UTF-8
- Timestamps: Unix seconds
- Maximum page size: 500 messages
- Maximum outbound text: 64 KiB
- Maximum outbound file: 100 MiB after Base64 decoding
- API errors use an HTTP status code and a JSON body such as `{"error":"reason"}`.

Except for `GET /status`, every endpoint requires the configured token:

```http
Authorization: Bearer YOUR_API_TOKEN
```

For WebSocket clients that cannot set an authorization header, use `ws://HOST:8899/ws?token=YOUR_URL_ENCODED_TOKEN`. Query-string authentication can leak through URLs and logs, so prefer a header and TLS-capable reverse proxy whenever possible.

## 2. Message identity model

Every database message includes both conversation identity and sender identity. Do not use a display name as a primary key.

| Field | Meaning |
| --- | --- |
| `message_id` | Stable deduplication/reply ID. It normally uses the WeChat server message ID. |
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
  "version": "0.6.0",
  "listen_count": 0,
  "db_available": true,
  "contacts": 42,
  "uptime_secs": 3600,
  "message_cursor": 128
}
```

`message_cursor` is process-local. It resets when the service restarts. Use `uptime_secs` or an independently stored instance marker/timestamp to detect a restart and fall back to history catch-up.

### `GET /contacts`

Returns contacts resolved from the encrypted WeChat database.

### `GET /sessions`

Returns the current WeChat session list. This endpoint is useful for discovery; stable message routing should still persist the IDs returned with messages.

## 4. Receiving messages

### WebSocket `/ws`

The WebSocket stream is the lowest-latency interface. Database events have `type: "db_message"` and otherwise contain the same fields as an item returned by `GET /messages`, including `cursor`.

The server sends a WebSocket Ping every 30 seconds. Clients must reconnect with exponential backoff and deduplicate with `message_id`. The WebSocket stream does not replay missed events by itself.

### `GET /messages`

Returns the in-memory real-time buffer without consuming messages globally. Every caller owns an independent cursor.

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

Persist `next_cursor` only for the lifetime of the same service process. If `has_more` is true, request the next page immediately.

### `GET /messages/history`

Reads from the encrypted WeChat database. Use this endpoint for initial import and reconnect/restart catch-up.

Query parameters:

| Parameter | Default | Description |
| --- | ---: | --- |
| `since` | `0` | Inclusive Unix timestamp. Keep it unchanged while paging. |
| `offset` | `0` | Stable offset for this exact `since` value; maximum 100000. |
| `limit` | `100` | 1–500. |
| `chat` | unset | Match conversation ID or name. |
| `sender` | unset | Match sender ID or name. |
| `direction` | unset | Direction filter. |

Results are ordered from oldest to newest. The response contains `messages`, `next_offset`, `checkpoint_time`, the compatibility alias `next_since`, and `has_more`. Keep the original `since` fixed and request the next page with `offset=next_offset` until `has_more=false`. Optional sender/direction filters are applied after scanning a base page, so a page can contain fewer than `limit` messages or even be empty while `has_more=true`.

After the final page, persist `checkpoint_time` as the next run's inclusive `since`. Always deduplicate by `message_id`: several messages can share the same second, and a crash after processing but before saving the checkpoint intentionally causes safe replay.

### Recommended reliable consumer flow

1. Persist every processed `message_id` and the latest `create_time` in the consuming service.
2. Connect the WebSocket and temporarily buffer live events.
3. Call `/messages/history?since=LAST_CREATE_TIME&offset=0`; while `has_more=true`, keep `since` fixed and advance to `next_offset`.
4. Drain the buffered WebSocket events, again deduplicating by `message_id`.
5. Continue consuming WebSocket events; periodically checkpoint `create_time` and processed IDs.
6. After a disconnect or service restart, repeat from step 2.

This provides at-least-once delivery semantics. MimicWX persists only database table watermarks, not plaintext messages; the consuming service remains responsible for durable business-level deduplication.

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

`available: false` means WeChat has not yet written a matching local file. The client may retry the download endpoint because availability is checked again on every request. Executable and archive attachments are returned only as bytes; MimicWX never executes them. Consumers should apply their own file-size limit, malware scanning, content-type detection, and sandboxed parsing before indexing.

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

### `POST /messages/reply`

Reply to the original conversation of a recent real-time message:

```json
{
  "message_id": "wx:123456789",
  "text": "Indexing finished: 128 passages created.",
  "mention_sender": true
}
```

For a group message, `mention_sender: true` automatically mentions the originating group member. The referenced message must still be present in the process's live buffer. If it is not, use the persisted `conversation_id` with `/messages/send` and explicitly populate `at` when needed.

### `POST /messages/send-file`

Alias: `POST /send_file`.

```json
{
  "conversation": "wxid_or_group_id",
  "name": "answer.pdf",
  "file": "BASE64_BYTES"
}
```

The decoded file is limited to 100 MiB, written with owner-only permissions, selected through WeChat's native file chooser, and deleted after a grace period. File names containing path separators or control characters are rejected.

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
                # Enqueue by message_id, preserve conversation_id/sender_id,
                # Download an attachment if present, then dispatch downstream.
                print(event["message_id"], event["conversation_id"], event["sender_id"])

asyncio.run(run())
```

Production consumers should implement the history/bootstrap flow above, durable deduplication, bounded queues, retry/backoff, observability, and a dead-letter path for messages or files that cannot be processed.

## 8. Security notes

- Keep the API on a trusted private network or behind an authenticated TLS reverse proxy.
- Always configure a long random API token; never commit it to Git.
- Do not expose noVNC or the API directly to the public Internet.
- Treat incoming Office files, archives, APKs, and executables as untrusted input.
- Store downloaded files outside web roots and parse them in a sandbox.
- Rotate the API token after accidental disclosure.
