# PROGRESS

## Current phase
Phase 3 (Matrix auth/sync) core done and verified against a real Synapse. Phase 4 core logic + account-data sync verified.
UI is still a placeholder window.

## Completed (verified)
- Session 1: contacts model/resolver/manager/sync schema/SQLite storage (25 unit tests pass).
- Session 2:
  - rustfmt + clippy now installed; code formatted, clippy clean (apart from dead-code warnings for not-yet-wired modules).
  - GTK placeholder window launched on Wayland for 5 s without errors (not visually inspected).
  - Crate split into lib (`contacts`, `matrix`, `storage`) + bin (`ui`).
  - `matrix::authentication`: login (discovery via server name or URL), `login_unsaved`, restore, logout; SDK sqlite store encrypted with random passphrase.
  - `matrix::secrets`: oo7 Secret Service/file keyring. Policy: no secure store => session not persisted (re-login). **Secret-store save/restore paths are NOT yet tested against a real keyring.**
  - `matrix::synchronization`: sync loop with capped exponential backoff, SessionExpired detection, channel to GTK thread. Backoff unit-tested; loop itself not yet exercised in a test.
  - `matrix::contacts_sync`: fetch -> parse -> merge -> write account data (newer-schema guard).
  - `tests/homeserver.rs` (env `MSM_TEST_HOMESERVER`, helper `scripts/test-homeserver.sh`): 3 tests PASS against Synapse 1.163:
    wrong password error; Alice's nickname for Darkmage syncs to a second installation, Bob's nickname is independent,
    Darkmage's public profile unchanged; DM create -> invite -> join -> message delivered (unicode).
  - Results: `cargo test` = 25 unit + 3 integration passed.

## Session 3
- Added test: session persist -> restore -> logout wipes (real keyring worked here; test skips if none). 4 integration tests pass.
- Real GTK UI: `app.rs` (state, tokio bridge `spawn_async`), login page, main window (own name/MXID, status, search,
  favourites + expandable groups, right-click contact menu: rename/reset/favourite/move/remove, group menu: rename/up/delete,
  Add contact, New group, Sign out). Names go through NameResolver; public display names fetched async; every edit persists to
  SQLite and merges/pushes to account data.
- Verified by running the app against Synapse with a seeded session: restored as @alice:localhost, showed "Connected", and
  showed Darkmage under Other as "James Carthew" (nickname synced from account data). Only that one screenshot-based check; menus/dialogs/login form were NOT clicked through (no input automation on Wayland here).
- Fixed a RefCell borrow smell in login handler; silenced SDK's expected-404 ERROR logs.

## Known problems / not verified
- No Element interoperability test; no E2EE test (DM test used an unencrypted DM? create_dm may enable encryption by default — not checked).
- Docker unusable (permission denied); Synapse runs from a pip venv instead.
- libsecret headers absent (oo7 is pure Rust, so not required).
- UI interactions (dialogs, context menus, login form) not exercised by hand or automation.
- No avatars, presence, messaging UI yet. Single-account assumption in the local contact cache.

## Exact next task
Phase 5: conversation windows. DM discovery via `m.direct` (+ create when absent), message timeline (matrix-sdk-ui Timeline or
event handlers), send/pending/failed states, typing indicator, unread badges on the contact list, desktop notifications using the resolver.
Add integration tests for DM discovery via m.direct created by another client.
