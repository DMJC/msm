# AGENTS.md

## Build / test
- `cargo build`, `cargo test` (22 unit tests), `cargo check --all-targets`
- `cargo fmt` and `cargo clippy` are NOT installed on this machine (`rustup component add rustfmt clippy` needed); run them when available.
- Toolchain: rustc 1.96.1. System: GTK 3.24.52, SQLite 3.53, OpenSSL 3.6. libsecret-1 dev package NOT installed.

## Verified dependency set
gtk 0.19 (GTK3; re-exports glib 0.22 as `gtk::glib`), matrix-sdk 0.19.1 (needs rustc 1.96), tokio, rusqlite (bundled),
serde, thiserror, tracing. A clean build with all of these succeeds (first build ~2 min).

## Structure
- `src/contacts/` — pure logic, no GTK/Matrix deps: `contact`, `nickname_resolver` (the ONLY place names are resolved),
  `contact_manager` (ops + resolver sync), `synchronization` (account-data schema `com.matrixmessenger.contacts.v1`, per-contact LWW merge with tombstones).
- `src/storage/` — rusqlite DB with append-only migrations in `migrations.rs`.
- `src/ui/` — GTK3 (placeholder window only).
- `docs/` — not yet written (see TODO.md).

## Rules
GTK3 only; GTK calls on main thread only; Matrix/DB work off the UI thread; never log secrets; nicknames never touch Matrix profile/room state.
