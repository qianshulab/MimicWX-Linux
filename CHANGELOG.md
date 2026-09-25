# Changelog

All notable changes to MimicWX-Linux are documented in this file. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses semantic versioning for its public interface.

## [Unreleased]

### Documentation

- Added separate Chinese and English project overviews.
- Added a documentation hub, bilingual Docker deployment guides, contribution guidelines, and a security policy.
- Added structured issue forms and a pull request checklist.
- Standardized terminology, navigation, compatibility notes, and security guidance across the documentation set.

## [0.6.0] - 2026-09-25

### Added

- Multi-consumer real-time message buffer with independent cursors.
- Encrypted-database history catch-up and persistent table watermarks.
- Stable message IDs and explicit conversation, sender, direction, and group identity fields.
- Authenticated attachment downloads with path confinement, MIME hardening, and HTTP Range support.
- Conversation-aware text sending, reply-by-message-ID, and generic file sending.
- Automatic originating-sender mentions for group-message replies.
- WeChat 4.1 passphrase capture, persistent login watcher, per-database key derivation, page-1 HMAC validation, key-map hot reload, and new-database discovery.
- Production-oriented Docker Compose defaults, Docker secret support, health checks, and log rotation.

### Compatibility

- Retained the WeChat 4.0 legacy key-extraction path.
- Retained legacy message and send endpoints for existing consumers.

### Security

- Restricted attachment resolution to approved account directories.
- Added input limits for text, mention lists, history offsets, Base64 files, and upload names.
- Kept runtime credentials and persistent data outside image layers and version control.

## Earlier versions

The independent maintenance line started from upstream version 0.5.2. Earlier history remains available in the Git commit log and the [upstream repository](https://github.com/aisuyi065/MimicWX-Linux).
