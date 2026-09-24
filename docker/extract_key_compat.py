#!/usr/bin/env python3
"""MimicWX key lifecycle support for WeChat 4.1+.

Uses the pinned MIT-licensed wcdb-key-tool implementation for passphrase
capture and PBKDF2 derivation, then writes the file format expected by
MimicWX. Existing keys and every derived key are accepted only after the
database page-one HMAC check succeeds.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import pathlib
import tempfile
import time
from typing import Any


KEY_DIR = pathlib.Path("/home/wechat/.xwechat")
KEY_FILE = KEY_DIR / "wechat_key.txt"
KEY_MAP_FILE = KEY_DIR / "wechat_keys.json"
AUDIT_FILE = KEY_DIR / "wcdb_all_keys.json"
PASSPHRASE_FILE = KEY_DIR / ".wcdb-key-tool" / "wechat-passphrase.json"
TOOL_PATH = pathlib.Path("/usr/local/lib/mimicwx/wcdb_key_tool.py")


def log(message: str) -> None:
    print(f"[extract_key_compat] {message}", flush=True)


def load_tool() -> Any:
    spec = importlib.util.spec_from_file_location("wcdb_key_tool", TOOL_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {TOOL_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.PASSPHRASE_FILE = str(PASSPHRASE_FILE)
    module.find_runtime_base = fixed_runtime_base
    return module


def fixed_runtime_base(pid: int, binary_path: str | pathlib.Path) -> int:
    """Return the PIE load bias, including packages with a pre-text RO map."""
    binary_name = pathlib.Path(binary_path).name
    lines = pathlib.Path(f"/proc/{pid}/maps").read_text(encoding="utf-8").splitlines()
    matching: list[list[str]] = []
    for line in lines:
        parts = line.split()
        if len(parts) >= 6 and parts[5].endswith("/" + binary_name):
            matching.append(parts)
    for parts in matching:
        if int(parts[2], 16) == 0:
            return int(parts[0].split("-")[0], 16)
    for parts in matching:
        if "x" in parts[1]:
            return int(parts[0].split("-")[0], 16) - int(parts[2], 16)
    raise RuntimeError(f"cannot find runtime base for PID {pid}")


def find_db_dir() -> pathlib.Path:
    root = pathlib.Path("/home/wechat/Documents/xwechat_files")
    candidates = sorted(root.glob("*/db_storage"))
    for candidate in candidates:
        if any(candidate.rglob("*.db")):
            return candidate
    raise RuntimeError("WeChat db_storage directory not found")


def checked_hex(value: object, length: int) -> str:
    if not isinstance(value, str) or len(value) != length:
        raise ValueError(f"expected {length} hexadecimal characters")
    bytes.fromhex(value)
    return value.lower()


def atomic_write(path: pathlib.Path, content: str, owner: tuple[int, int] | None) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            handle.write(content)
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(temporary, 0o600)
        if owner is not None:
            os.chown(temporary, owner[0], owner[1])
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def load_mimic_mapping() -> dict[str, str]:
    try:
        value = json.loads(KEY_MAP_FILE.read_text(encoding="utf-8"))
        if isinstance(value, dict):
            return {str(key).replace("\\", "/"): str(raw) for key, raw in value.items()}
    except (OSError, json.JSONDecodeError):
        pass
    return {}


def validate_mapping(tool: Any, db_files: list, mapping: dict[str, str]) -> tuple[int, int]:
    valid = 0
    for relative, _path, _size, _salt, page_one in db_files:
        raw_key = mapping.get(relative.replace("\\", "/"), "")
        try:
            enc_key = bytes.fromhex(raw_key[:64])
        except ValueError:
            continue
        if len(raw_key) == 96 and tool.verify_enc_key(enc_key, page_one):
            valid += 1
    return valid, len(db_files)


def save_derived_keys(tool: Any, db_files: list, salt_to_dbs: dict, passphrase_hex: str) -> None:
    passphrase = bytes.fromhex(checked_hex(passphrase_hex, 64))
    key_map = tool._derive_keys_from_passphrase(passphrase, db_files, salt_to_dbs)
    if not key_map:
        raise RuntimeError("PBKDF2 produced no HMAC-verified keys")

    mapping: dict[str, str] = {}
    audit: dict[str, object] = {"_db_dir": str(find_db_dir())}
    for relative, _path, size, salt_hex, _page_one in db_files:
        enc_key = key_map.get(salt_hex)
        if enc_key is None:
            continue
        enc_key = checked_hex(enc_key, 64)
        salt_hex = checked_hex(salt_hex, 32)
        normalized = relative.replace("\\", "/")
        mapping[normalized] = enc_key + salt_hex
        audit[normalized] = {
            "enc_key": enc_key,
            "salt": salt_hex,
            "size_mb": round(size / 1024 / 1024, 1),
        }

    if len(mapping) != len(db_files):
        raise RuntimeError(f"only {len(mapping)}/{len(db_files)} database keys verified")

    primary = mapping.get("message/message_0.db") or next(iter(mapping.values()))
    owner = (1000, 1000)
    atomic_write(KEY_MAP_FILE, json.dumps(mapping, indent=2, ensure_ascii=False) + "\n", owner)
    atomic_write(KEY_FILE, primary + "\n", owner)
    atomic_write(AUDIT_FILE, json.dumps(audit, indent=2, ensure_ascii=False) + "\n", owner)
    log(f"saved {len(mapping)} HMAC-verified database keys")


def refresh_keys(tool: Any, capture: bool, capture_timeout: int) -> bool:
    db_dir = find_db_dir()
    db_files, salt_to_dbs = tool.collect_db_files(str(db_dir))
    if not db_files:
        raise RuntimeError(f"no databases found in {db_dir}")

    existing = load_mimic_mapping()
    valid, total = validate_mapping(tool, db_files, existing)
    if valid == total:
        log(f"existing keys verified ({valid}/{total})")
        return True
    log(f"key refresh required ({valid}/{total} currently valid)")

    passphrase_hex = tool.load_passphrase()
    if passphrase_hex:
        try:
            save_derived_keys(tool, db_files, salt_to_dbs, passphrase_hex)
            return True
        except Exception as error:
            log(f"cached passphrase no longer validates: {error}")

    if not capture:
        return False

    log("waiting for WeChat logout and login to capture the 4.1+ passphrase")
    passphrase_hex = tool.capture_passphrase(timeout=capture_timeout)
    tool.save_passphrase(passphrase_hex)
    PASSPHRASE_FILE.parent.chmod(0o700)
    save_derived_keys(tool, db_files, salt_to_dbs, passphrase_hex)
    return True


def run_once(capture_timeout: int) -> int:
    tool = load_tool()
    try:
        return 0 if refresh_keys(tool, capture=True, capture_timeout=capture_timeout) else 1
    except Exception as error:
        log(f"key extraction failed: {error}")
        return 1


def run_monitor(interval: int) -> int:
    tool = load_tool()
    last_result: bool | None = None
    while True:
        time.sleep(interval)
        try:
            current = refresh_keys(tool, capture=False, capture_timeout=0)
            if current != last_result:
                log("automatic key refresh monitor is healthy" if current else "new login is required")
            last_result = current
        except Exception as error:
            if last_result is not False:
                log(f"monitor warning: {error}")
            last_result = False


def run_capture_watcher(capture_timeout: int, retry_delay: int) -> int:
    """Stay attached so a future login can refresh a rotated passphrase."""
    tool = load_tool()
    while True:
        try:
            log("login key watcher armed")
            passphrase_hex = checked_hex(
                tool.capture_passphrase(timeout=capture_timeout), 64
            )
            tool.save_passphrase(passphrase_hex)
            PASSPHRASE_FILE.parent.chmod(0o700)
            db_dir = find_db_dir()
            db_files, salt_to_dbs = tool.collect_db_files(str(db_dir))
            save_derived_keys(tool, db_files, salt_to_dbs, passphrase_hex)
            log("login key rotation captured and published")
        except Exception as error:
            log(f"login key watcher retry: {error}")
        time.sleep(retry_delay)


def main() -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    once = subparsers.add_parser("once")
    once.add_argument("--capture-timeout", type=int, default=600)
    monitor = subparsers.add_parser("monitor")
    monitor.add_argument("--interval", type=int, default=60)
    watch = subparsers.add_parser("watch")
    watch.add_argument("--capture-timeout", type=int, default=86400)
    watch.add_argument("--retry-delay", type=int, default=10)
    args = parser.parse_args()
    if args.command == "once":
        return run_once(args.capture_timeout)
    if args.command == "watch":
        return run_capture_watcher(
            max(60, args.capture_timeout), max(5, args.retry_delay)
        )
    return run_monitor(max(10, args.interval))


if __name__ == "__main__":
    raise SystemExit(main())
