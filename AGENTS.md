# AGENTS.md

## Build / test
- `cargo build`, `cargo test` (22 unit tests), `cargo check --all-targets`
- `cargo fmt --all` and `cargo clippy --all-targets` are installed; keep them clean.
- Integration tests: `scripts/test-homeserver.sh <workdir>` starts local Synapse (pip; Docker is not usable), then `MSM_TEST_HOMESERVER=http://localhost:8008 cargo test`. Without the env var they skip silently.
- Toolchain: rustc 1.96.1. System: GTK 3.24.52, SQLite 3.53, OpenSSL 3.6. libsecret-1 dev package NOT installed.

## Verified dependency set
gtk 0.19 (GTK3; re-exports glib 0.22 as `gtk::glib`), matrix-sdk 0.19.1 (needs rustc 1.96), tokio, rusqlite (bundled),
serde, thiserror, tracing. A clean build with all of these succeeds (first build ~2 min).

## Structure
- `src/contacts/` — pure logic, no GTK/Matrix deps: `contact`, `nickname_resolver` (the ONLY place names are resolved),
  `contact_manager` (ops + resolver sync), `synchronization` (account-data schema `com.matrixmessenger.contacts.v1`, per-contact LWW merge with tombstones).
- `src/storage/` — rusqlite DB with append-only migrations in `migrations.rs`.
- `src/matrix/` — authentication, secrets (oo7), synchronization (sync loop), contacts_sync (account data).
- `src/lib.rs` exposes contacts/matrix/storage; `src/main.rs` + `src/ui/` is the GTK bin.
- `src/ui/` — GTK3 (placeholder window only).
- `docs/` — not yet written (see TODO.md).

## Rules
GTK3 only; GTK calls on main thread only; Matrix/DB work off the UI thread; never log secrets; nicknames never touch Matrix profile/room state.
