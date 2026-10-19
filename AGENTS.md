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

## Wayland, tray and desktop integration
- GTK3 runs natively on Wayland (`GdkWaylandDisplay` is logged at startup); no X11-only APIs are used.
- Tray icon: `src/tray.rs`, StatusNotifierItem over D-Bus (ksni), NOT `gtk::StatusIcon` (X11-only). Works with panels that run a
  StatusNotifierWatcher (MATE panel, KDE, XFCE, GNOME + AppIndicator extension). With no watcher, the tray is absent and closing the window quits.
  The tray menu sets status; left click toggles the window; closing the window hides it to the tray.
- Icons are drawn in code (`src/icon.rs`), so no icon theme is needed. `data/*.desktop` + SVG + `scripts/install-local.sh` give Wayland
  panels the app_id -> icon mapping (the application id is `com.matrixmessenger.Msm`).
- Own status flows: UI/tray -> `App::set_my_presence` -> watch channel -> sync loop (`matrix::synchronization::run`), which publishes it.

## Testing hazards
- Integration tests must never use the real secret-store namespace: call `matrix::secrets::set_namespace(..)` first (the keyring test does).
- Manual GUI testing without touching the real desktop: run Xephyr (`Xephyr :5`), then `DISPLAY=:5 GDK_BACKEND=x11` for the app and xdotool.
  Never send xdotool input / screenshots to the real display; the tray can be driven over D-Bus
  (`busctl --user call org.kde.StatusNotifierItem-<pid>-1 /MenuBar com.canonical.dbusmenu Event isvu <id> clicked i 0 0`).
