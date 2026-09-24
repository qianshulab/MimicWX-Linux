# NAS Docker Deployment

This guide targets an x86-64 NAS such as an Intel N100 system. MimicWX runs the official Linux WeChat client, a virtual desktop, noVNC, and the API in one container. The host only needs Docker Engine and Docker Compose.

## Directory layout

Keep the stack with the NAS's other Docker applications:

```text
/volume1/docker/mimicwx/
├── compose.nas.yaml
├── config.local.toml
├── .env
├── secrets/
│   └── vnc_password.txt
└── data/
    ├── xwechat/
    └── xwechat_files/
```

The two `data` directories preserve the WeChat account and downloaded files across container replacement. Do not commit `config.local.toml`, `.env`, `secrets/`, or `data/` from a live deployment.

## Initial configuration

1. Copy `.env.example` to `.env` and set the NAS LAN address. Keep `127.0.0.1` when access is only through a reverse proxy or SSH tunnel.
2. Copy `config.toml` to the ignored `config.local.toml`, then create a long random API token there.
3. Put the noVNC password in `secrets/vnc_password.txt` and restrict the file to its owner.
4. Create the persistent directories before starting the stack.

```bash
cp .env.example .env
cp config.toml config.local.toml
mkdir -p data/xwechat data/xwechat_files secrets
chmod 700 secrets
chmod 600 secrets/vnc_password.txt config.local.toml
docker compose -f compose.nas.yaml build
docker compose -f compose.nas.yaml up -d
```

The NAS can use a Docker daemon proxy for image and package downloads. Configure that at Docker service level rather than placing proxy credentials in the Compose file. Build arguments and secrets must not be baked into the final image.

## Login and automatic database keys

Open `http://NAS_ADDRESS:6080/vnc.html?autoconnect=true&resize=scale`, connect with the configured VNC password, and sign in to WeChat.

For WeChat 4.0, the compatibility path can scan process memory for an AES key. For WeChat 4.1, the container captures the 32-byte database passphrase during login, derives each database's AES key with the WCDB parameters, and validates it using the page-1 HMAC before use. One monitor discovers new database salts, while an armed login watcher captures a future rotated passphrase. The core detects the atomic key-map update, reopens its database connections, and discovers new `message_N.db` files without a container restart. The raw passphrase and derived keys stay inside the private persistent WeChat data directory and are never included in the image or repository.

Confirm readiness:

```bash
curl --fail http://NAS_ADDRESS:8899/status
```

The deployment is ready when the account is logged in and `db_available` is `true`.

## Network and hardening

- Bind ports to the NAS LAN address, not `0.0.0.0`.
- Do not publish VNC port 5901; use noVNC 6080 only on a trusted network.
- Keep API port 8899 private and require a token.
- The container needs `SYS_PTRACE` for automatic key capture and `SYS_ADMIN` for the existing database notification mechanism. These capabilities are security-sensitive; run this stack only on a trusted host.
- Runtime state uses bind mounts so upgrades can replace the container without deleting WeChat data.
- Logs are size-limited and rotated by Compose.
- Keep the previous image tag until the new version has passed login, database, receive, download, and send tests.

## Upgrade and rollback

Build the new immutable tag first, then update `MIMICWX_IMAGE` in `.env` and recreate only this service:

```bash
docker compose -f compose.nas.yaml build
docker compose -f compose.nas.yaml up -d --force-recreate mimicwx
```

For rollback, restore the previous `MIMICWX_IMAGE` value and recreate the service. Do not delete `data/` as part of an application rollback.

## Backup

Stop the service or ensure WeChat is idle, then back up `data/xwechat`, `data/xwechat_files`, `config.local.toml`, `.env`, and `secrets/vnc_password.txt` with NAS-native snapshots or backup software. Protect backups as sensitive data because they contain the WeChat profile and API/VNC credentials.
