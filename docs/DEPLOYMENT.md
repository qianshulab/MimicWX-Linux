# Docker Deployment Guide

This guide describes a production-oriented Docker deployment of MimicWX-Linux on an x86-64 Linux host. The container includes the official WeChat Linux client, a lightweight desktop, noVNC, the MimicWX service, and its database-key lifecycle helpers.

## 1. Requirements

- x86-64 Linux host
- Docker Engine 24 or later
- Docker Compose v2
- 2 GB or more available memory
- 5 GB or more available disk space, plus space for WeChat data and attachments
- Outbound access to the WeChat package URL, Ubuntu package repositories, Rust toolchain, and crate registry

The service requires the following Linux capabilities:

- `SYS_PTRACE` for automatic database passphrase capture
- `SYS_ADMIN` for the current database change notification mechanism

These capabilities are security-sensitive. Run the container only on a trusted, isolated host.

## 2. Directory layout

Keep application files, persistent state, and secrets in one dedicated directory:

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

The repository can itself be the deployment directory. `data/`, `secrets/`, `.env`, and `config.local.toml` are excluded from Git and must remain outside image layers.

## 3. Initial setup

```bash
git clone https://github.com/qianshulab/MimicWX-Linux.git /opt/mimicwx
cd /opt/mimicwx

cp .env.example .env
cp config.toml config.local.toml
install -d -m 700 secrets data/xwechat data/xwechat_files
openssl rand -hex 4 > secrets/vnc_password.txt
chmod 600 .env config.local.toml secrets/vnc_password.txt
```

Set a long random API token in `config.local.toml`:

```toml
[api]
token = "replace-with-a-long-random-token"
```

The VNC protocol uses only the first eight password characters. The command above generates eight random hexadecimal characters. Keep noVNC restricted to a trusted network even when a password is configured.

## 4. Network configuration

The default `.env` binds the API and noVNC to loopback:

```dotenv
MIMICWX_BIND_IP=127.0.0.1
```

Choose one of the following access patterns:

1. Keep `127.0.0.1` and publish services through an authenticated TLS reverse proxy.
2. Keep `127.0.0.1` and use an SSH tunnel.
3. Bind to a specific private-network address when direct LAN access is required.

Do not use `0.0.0.0` unless host firewall rules strictly limit access. The raw VNC port 5901 is intentionally not published.

Published services:

| Port | Service | Authentication |
| ---: | --- | --- |
| 6080 | noVNC browser desktop | VNC password |
| 8899 | REST and WebSocket API | Bearer Token, except `/status` |

## 5. Build sources and proxies

The default build uses package and Rust mirrors suitable for networks in mainland China:

```dotenv
MIMICWX_USE_MIRROR=1
```

Set `MIMICWX_USE_MIRROR=0` to use upstream Ubuntu and Rust sources.

If the host requires an HTTP proxy, configure it for the Docker daemon rather than embedding proxy credentials in the Compose file or image. A systemd-based Docker installation commonly uses a drop-in such as:

```ini
# /etc/systemd/system/docker.service.d/http-proxy.conf
[Service]
Environment="HTTP_PROXY=http://PROXY_HOST:PORT"
Environment="HTTPS_PROXY=http://PROXY_HOST:PORT"
Environment="NO_PROXY=localhost,127.0.0.1"
```

Reload and restart Docker after changing daemon settings:

```bash
sudo systemctl daemon-reload
sudo systemctl restart docker
```

Proxy credentials are secrets. Store them using the host's protected service configuration and never commit them to this repository.

## 6. Start and sign in

Build and start the service:

```bash
docker compose up -d --build
docker compose ps
```

Open the browser desktop:

```text
http://HOST:6080/vnc.html?autoconnect=true&resize=scale
```

Connect using the password stored in `secrets/vnc_password.txt`, then sign in to WeChat inside the virtual desktop.

For WeChat 4.0, the compatibility path can scan process memory for the database key. For WeChat 4.1, the login watcher captures the 32-byte database passphrase, derives each database key using its salt, and accepts only keys that pass page-1 HMAC verification. The watcher remains active for future rotations, and newly created databases are discovered without replacing the container.

## 7. Readiness checks

Check the public status endpoint:

```bash
curl --fail http://HOST:8899/status
```

The service is ready for message ingestion when the account is logged in and `db_available` is `true`.

Check container health and recent logs:

```bash
docker compose ps
docker compose logs --tail=200 mimicwx
```

The Compose health check queries `/status`. It confirms that the API process responds; consumers must also inspect `status` and `db_available` before reading messages.

## 8. API verification

Set the connection values without storing them in shell history when possible:

```bash
export MIMICWX_BASE="http://HOST:8899"
export MIMICWX_TOKEN="YOUR_API_TOKEN"
```

Verify an authenticated endpoint:

```bash
curl --fail \
  -H "Authorization: Bearer $MIMICWX_TOKEN" \
  "$MIMICWX_BASE/sessions"
```

See [API.md](API.md) or [API.zh-CN.md](API.zh-CN.md) for message consumption, attachment download, sending, and reply flows.

## 9. Persistent data and backup

Back up the following paths:

| Path | Contents |
| --- | --- |
| `data/xwechat/` | WeChat profile, encrypted databases, and derived key map |
| `data/xwechat_files/` | Downloaded or cached WeChat files |
| `config.local.toml` | API Token and listener configuration |
| `.env` | Host binding and image settings |
| `secrets/vnc_password.txt` | VNC credential source |

Stop the service or ensure WeChat is idle before taking a file-level backup:

```bash
docker compose stop mimicwx
# Run the host backup or snapshot operation here.
docker compose start mimicwx
```

Protect backups as sensitive data. They can contain account state, credentials, contact metadata, and message attachments.

## 10. Upgrade and rollback

Before upgrading:

1. Back up persistent data and configuration.
2. Record the current Git revision and image tag.
3. Build the new version under a distinct immutable image tag.

```bash
git pull --ff-only
```

Update `MIMICWX_IMAGE` in `.env`, then build and recreate only this service:

```bash
docker compose build mimicwx
docker compose up -d --force-recreate mimicwx
```

Validate the following before removing the previous image:

- WeChat login remains valid or can be restored.
- `/status` reports `db_available=true`.
- A new incoming message appears through WebSocket or `/messages`.
- A recent attachment can be downloaded.
- Text reply and file send complete successfully.

To roll back, restore the previous application revision and `MIMICWX_IMAGE` value, then recreate the service. Do not delete `data/` during an application rollback.

## 11. Operational security

- Require a long API Token and rotate it after any suspected disclosure.
- Limit ports 6080 and 8899 with host firewall rules.
- Put remote access behind TLS and a separate authentication layer.
- Do not expose noVNC directly to the public Internet.
- Treat every inbound file as untrusted, regardless of file extension.
- Apply size limits, malware scanning, content detection, and sandboxed parsing in downstream systems.
- Keep the host, Docker Engine, base image, and reverse proxy patched.
- Review container logs for repeated authentication failures or unexpected send activity.

## 12. Troubleshooting

| Symptom | Checks |
| --- | --- |
| noVNC does not connect | Confirm port 6080 binding, container health, password file presence, and `docker compose logs mimicwx`. |
| `db_available=false` | Confirm WeChat is logged in, wait for key validation, then inspect key watcher and database initialization logs. |
| Build downloads are slow | Select the appropriate `MIMICWX_USE_MIRROR` value or configure a Docker daemon proxy. |
| API returns `401` | Confirm the token in `config.local.toml` and the `Authorization: Bearer ...` header match. |
| A file reports `available=false` | Wait for WeChat to finish writing the local file and retry the same attachment endpoint. |
| Sending opens the wrong chat | Use the stable `conversation_id` when available and assign unique remarks to chats with duplicate display names. |
| Messages are repeated after reconnect | Deduplicate durably by `message_id`; the delivery model is intentionally at least once. |

When reporting an issue, include the MimicWX version, WeChat Linux version, Docker version, sanitized `/status` output, and relevant sanitized logs. Never attach databases, key files, tokens, contact data, or message contents.
