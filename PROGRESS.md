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

## Known problems / not verified
- No Element interoperability test; no E2EE test (DM test used an unencrypted DM? create_dm may enable encryption by default — not checked).
- Restore-session path, logout, and keyring usage untested.
- Docker unusable (permission denied); Synapse runs from a pip venv instead.
- libsecret headers absent (oo7 is pure Rust, so not required).
- No UI beyond placeholder; the resolver isn't used by UI yet.

## Exact next task
Build the real login window + main contact-list window (GTK3): login form -> `login`/`restore`, tokio runtime held in a OnceLock,
`SyncEvent` channel consumed with `glib::MainContext::spawn_local`; contact list from `ContactManager` loaded from `Database`,
grouped/expandable, context menu for rename/move/favourite/remove, using `NameResolver` everywhere. Then add tests for restore/logout against Synapse.
