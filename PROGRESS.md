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

## Session 4 (this session)
- Phase 5 (direct messaging) core + UI:
  - `matrix::messages`: Conversation model (ordering, dedupe, edits, redactions, local echo, retry) + event parser incl. redacted/encrypted shapes.
  - `matrix::rooms`: DM discovery via `m.direct` (prefers rooms where the peer is joined), create (encrypted by default), history, send with
    transaction ID (retry is idempotent), redact, typing, read receipts, unread counts, live event feed (messages/typing/presence).
  - `ui::conversation_window`: per-DM window, Enter sends / Shift+Enter newline, pending/failed/retry markers, delete own message,
    typing indicator, encryption label, undecryptable placeholder, older-history button, unread badges, desktop notifications (resolver names).
- Requirement from user: Wayland + tray icon with status.
  - Native Wayland verified (app logs `GdkWaylandDisplay`). GTK3 only.
  - `tray.rs`: StatusNotifierItem tray (ksni). Menu: Online/Away/Busy/Appear offline, Show/Hide window, Quit; left click toggles window;
    window close hides to tray when a tray exists. Verified on the user's real MATE-on-Wayland panel via D-Bus: registered with the watcher,
    menu layout read back, status clicks propagated to the server (offline/unavailable/online), toggle and Quit work.
  - Status selector + status message in the main window; status persisted and re-published at startup; tray/window icon coloured by status.
  - Contact presence dots (unknown shown hollow, never as offline).
  - `data/` desktop entry + SVG icon, `scripts/install-local.sh`. NOT verified: how the MATE panel shows the taskbar icon for the installed .desktop.
- Bugs found by testing and fixed: redacted encrypted events dropped from history; `/sync` overriding chosen presence;
  presence PUT slowness (Synapse) stalling the sync loop; unread counts inflated by replayed history; wrong DM room chosen.
- **Test hazard fixed:** the keyring integration test used the real app secret namespace, so running the suite could overwrite or delete a
  saved login. Tests now use `com.matrixmessenger.Msm.test`; verified that a seeded real-namespace session survives a full test run.
  (Before the fix, only sessions created by this agent's own manual testing were affected; none were the user's.)
- Tests: 40 unit + 8 integration (Synapse 1.163, local) pass; presence test passed 5/5 repeated runs; clippy/fmt clean apart from unused-code warnings.

## Known problems / not verified
- Not tested against Element or other third-party clients; no cross-client encryption test; no device verification / key backup UI.
- "Load older messages" button can show when there is nothing older (server returns an end token).
- Closing the window to hide it was implemented but not exercised through a real click (toggle via tray menu was).
- Automatic Away timer, friend requests, group rooms, media, reactions, sending edits, search, avatars: not implemented.
- Unread counts sum across all DMs with a contact (fine normally; inflated in the test accounts that have many leftover rooms).
- Contact cache assumes a single account.
- Synapse for tests runs from a pip venv (`scripts/test-homeserver.sh <dir>`); it was stopped at the end of this session.

## Exact next task
1. Phase 6: group rooms (invites accept/reject UI, room windows with participant nickname resolution, membership events, room naming).
2. Automatic Away on inactivity (configurable timer) feeding `App::set_my_presence`.
3. Element interoperability test (run Element Web/another client against local Synapse) and encrypted-room interop; device verification UI.
