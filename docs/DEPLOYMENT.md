# Docker Deployment Guide

> **Applies to:** MimicWX-Linux v0.6.x · **Platform:** x86-64 Linux with Docker Compose v2
>
> 中文版本：[DEPLOYMENT.zh-CN.md](DEPLOYMENT.zh-CN.md)

The Docker image includes the WeChat Linux client, an XFCE desktop, noVNC, the MimicWX service, and database-key extraction helpers. This guide covers configuration, startup, persistent storage, and maintenance on an x86-64 Linux host.

## Contents

- [Requirements](#1-requirements)
- [Directory layout](#2-directory-layout)
- [Initial setup](#3-initial-setup)
- [Network configuration](#4-network-configuration)
- [Build sources and proxies](#5-build-sources-and-proxies)
- [Start and sign in](#6-start-and-sign-in)
- [Readiness checks](#7-readiness-checks)
- [API verification](#8-api-verification)
- [Persistent data and backup](#9-persistent-data-and-backup)
- [Upgrade and rollback](#10-upgrade-and-rollback)
- [Operational security](#11-operational-security)
- [Troubleshooting](#12-troubleshooting)

## 1. Requirements

- x86-64 Linux host
- Docker Engine with Docker Compose support; version 24 or later is recommended
- Docker Compose v2
- Outbound access to the WeChat package URL, Ubuntu package repositories, Rust toolchain, and crate registry

For capacity planning, reserve at least 2 GB of available memory and 5 GB of disk space for the runtime, with additional space for messages and attachments. These are planning estimates, not validated minimums. Building Rust dependencies and Docker image layers requires additional memory and disk space; usage depends on the build cache and workload.

The service requires the following Linux capabilities:

- `SYS_PTRACE` for automatic database passphrase capture
- `SYS_ADMIN` for the current database change notification mechanism

The supplied Compose configuration also sets `seccomp:unconfined` and `apparmor:unconfined`. These settings relax container isolation for process inspection and desktop integration. Review them against the host's security requirements before deployment; the supplied configuration is not a sandbox for untrusted workloads.

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

Generate an API token:

```bash
openssl rand -hex 32
```

Set the generated value in `config.local.toml`:

```toml
[api]
token = "YOUR_RANDOM_API_TOKEN"
```

An absent or empty token disables API authentication. Keep the populated `config.local.toml` separate from the tracked configuration template.

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
| 8899 | REST and WebSocket API | Bearer Token when configured; `/status` is public |

## 5. Build sources and proxies

The default build uses package and Rust mirrors suitable for networks in mainland China:

```dotenv
MIMICWX_USE_MIRROR=1
```

Set `MIMICWX_USE_MIRROR=0` to use upstream Ubuntu and Rust sources.

Image pulls and commands executed during an image build use different proxy settings. A [Docker daemon proxy](https://docs.docker.com/engine/daemon/proxy/) handles registry access. On a systemd-based installation, a service drop-in can contain:

```ini
# /etc/systemd/system/docker.service.d/http-proxy.conf
[Service]
Environment="HTTP_PROXY=http://PROXY_HOST:PORT"
Environment="HTTPS_PROXY=http://PROXY_HOST:PORT"
Environment="NO_PROXY=localhost,127.0.0.1"
```

Reload and restart Docker after changing daemon settings. This affects the host's Docker service and may interrupt other containers:

```bash
sudo systemctl daemon-reload
sudo systemctl restart docker
```

For package downloads inside the build, pass [Docker's predefined proxy build arguments](https://docs.docker.com/build/building/variables/#proxy-arguments):

```bash
docker compose build \
  --build-arg HTTP_PROXY=http://PROXY_HOST:PORT \
  --build-arg HTTPS_PROXY=http://PROXY_HOST:PORT \
  mimicwx
```

The proxy address must be reachable from the build container; `127.0.0.1` there does not refer to the host. Do not commit proxy credentials or bake them into the image. Use protected host configuration when credentials are required.

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

For WeChat 4.0, the startup script selects the process-memory key scanner. For WeChat 4.1, it starts a login watcher that captures the 32-byte database passphrase, derives keys using each database's salt, and validates them against the first-page HMAC. A background monitor checks existing keys and discovers additional databases.

When the cached passphrase no longer validates, key capture may require signing out and back in. Compatibility depends on the installed WeChat version and database format; a client update can require an extractor update. Consult the logs if login succeeds but the database remains unavailable.

## 7. Readiness checks

Check the public status endpoint:

```bash
curl --fail http://HOST:8899/status
```

Before consuming messages, confirm that the account is logged in and `db_available` is `true`. This field reports that the database manager has initialized, not that every database operation will succeed. Verify an authenticated request and a new message as part of deployment checks.

Check container health and recent logs:

```bash
docker compose ps
docker compose logs --tail=200 mimicwx
```

The Compose health check queries `/status`. It confirms that the API process responds; consumers must also inspect `status` and `db_available` before reading messages.

### Host reboot recovery

Compose uses `restart: always`. After a host reboot, Docker can restart the existing container when the Docker service starts and its bind mounts and configured host address are available. Ensure that Docker itself is enabled at boot. `docker compose down` removes the container, so the restart policy no longer applies until it is created again.

The API starts before WeChat login and database initialization. Once the API has started, `/status` can report a pending login. An in-container watchdog exits the container when VNC, WeChat, or noVNC is no longer running, allowing Docker to restart the stack. An `unhealthy` status alone does not trigger Docker's restart policy, and the watchdog does not detect every possible stalled process.

WeChat may require a QR-code login or confirmation on the phone after a reboot. Complete that step through noVNC. The key watcher then attempts to capture or validate the database key; check `/status` and the logs before resuming message consumption.

To create or start the service:

```bash
docker compose up -d mimicwx
```

To restart an existing service and inspect its logs:

```bash
docker compose restart mimicwx
docker compose logs --tail=200 mimicwx
```

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
| `data/xwechat/` | WeChat configuration, captured passphrase, and derived key map |
| `data/xwechat_files/` | Account directories, encrypted databases, attachments, and message scan watermarks |
| `config.local.toml` | API Token and listener configuration |
| `.env` | Host binding and image settings |
| `secrets/vnc_password.txt` | VNC credential source |

Stop the service before taking a file-level backup so that database files and their WAL files are copied consistently:

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
3. Build the new version under a distinct image tag, and retain the previous image for rollback.

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

To roll back, restore the previous application revision and `MIMICWX_IMAGE` value, then recreate the service using the retained image:

```bash
docker compose up -d --no-build --force-recreate mimicwx
```

Do not delete `data/` during an application rollback. Rolling back an image does not revert database changes made by a newer WeChat client; restore a compatible backup if required.

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
| Build downloads are slow | Select the appropriate `MIMICWX_USE_MIRROR` value. Check the daemon proxy for image pulls and build proxy arguments for package downloads. |
| API returns `401` | Confirm the token in `config.local.toml` and the `Authorization: Bearer ...` header match. |
| A file reports `available=false` | Confirm that the file is downloaded and still exists in the WeChat client. The API cannot trigger a download; missing or unmatched files remain unavailable. |
| Sending opens the wrong chat | Use the stable `conversation_id` when available and assign unique remarks to chats with duplicate display names. |
| Messages are repeated after reconnect | Deduplicate durably by `message_id`; history catch-up and overlapping time windows can return records already processed. |
| Container is running but messages are unavailable | Inspect login state, `db_available`, and logs. Container health is not a complete message-delivery check. |

When reporting an issue, include the MimicWX version, WeChat Linux version, Docker version, sanitized `/status` output, and relevant sanitized logs. Never attach databases, key files, tokens, contact data, or message contents.
