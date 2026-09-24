# Changelog

## 0.6.0

- Added a multi-consumer real-time message buffer with independent cursors.
- Added encrypted-database history catch-up and persistent table watermarks.
- Added stable message IDs plus explicit conversation and sender identity fields.
- Added authenticated, range-capable attachment downloads with path confinement.
- Added conversation-aware text send, message reply, and generic file send APIs.
- Added automatic group-sender mentions when replying to a recent group message.
- Added WeChat 4.1 passphrase capture, an armed login watcher, WCDB key derivation, HMAC verification, hot connection reload, and new-database discovery while retaining the legacy key-extraction path.
- Added NAS-focused Compose defaults, secret handling, and detailed API/deployment documentation.
