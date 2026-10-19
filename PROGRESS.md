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

## Session 5
- Phase 6 (group rooms + invitations):
  - `matrix::rooms`: pending invites (with inviter), accept/decline, leave, create encrypted private room with invitees, invite more,
    group-room listing (non-DM, non-space), room titles, member names (room-specific display names).
  - UI: "Invitations" section (Accept/Decline) with notification on new invite, "Rooms" section (unread badges, double-click to open,
    right-click Open/Leave), "New room" button, room windows with Invite/Leave and member count; room display names feed the shared
    resolver (below local nicknames).
  - Verified in the GUI (nested X server) against Synapse: invite appeared live, Accept moved it to Rooms, room opened, message sent,
    state survives app restart (E2EE keys persisted, own message decrypted after restart); contact presence dot turned green once a room was shared.
  - Integration test `group_room_invite_accept_reject_message_and_members` passes (encrypted room, invite/accept/decline/leave, history).
- Bug fixed: the contact list was rebuilt on every sync event, which could swallow a double-click. It is now rebuilt only when
  the displayed data changes (`view_signature`).
- Tests: 40 unit + 9 integration pass; clippy and fmt clean.

## Session 6
- Automatic Away:
  - `matrix::presence::AutoAway` (pure, unit-tested): only Online users go Away; manual choices cancel it; activity restores Online only if Away was automatic.
  - `idle.rs`: Wayland `ext-idle-notify-v1` on a background thread (own connection, respects idle inhibitors); greyed-out setting when unsupported.
  - UI: "Away after N min idle (0 = off)" spin button; persisted (`away_minutes`, default 10); `MSM_AWAY_SECONDS` testing override.
  - Verified on the user's real Wayfire session: app logged `Idle` then `Resumed` events, tray tooltip went Online -> Away -> Online and the
    server presence followed. Idle event logging added (status changes made manually are also logged).
- Unresolved oddity: in one earlier run the tray tooltip showed Away -> Busy -> Online within ~8 s with no code path that sets Busy
  automatically; most likely the user clicking the tray menu, but not proven. A later run with logging showed no manual changes and no anomaly.
- Tests: 42 unit + 9 integration (integration last run before this feature; the feature touches only the app crate and presence.rs) ;
  clippy/fmt clean.

## Session 7
- Phase 8 (security), first part:
  - `matrix::encryption`: security status (cross-signing, recovery state, key backup, this-device verification), own device list,
    `setup_recovery` (cross-signing bootstrap incl. the server's password challenge, then secret storage + key backup -> recovery key),
    `recover` (unlock with recovery key). Secrets are never logged; the recovery key is not stored by the app.
  - Integration test `recovery_key_lets_a_new_device_read_old_encrypted_history` (fresh account per run): old encrypted message is
    unreadable on a new device without the key, wrong key is rejected, and it decrypts after `recover` + backup download. Passed 3/3 runs.
  - UI: "Security..." dialog (status, devices with verified marks, Set up recovery / Enter recovery key, buttons enabled only when applicable),
    masked prompt for password / key, one-time recovery-key dialog with copy button. Verified in the GUI (nested X server): setup via the
    password prompt, key shown, status refreshed, and state persisted after an app restart.
  - Note: our own device is always locally trusted in the crypto store, so "verified" for this device means *cross-signed by its owner*.
- **Crash fixed:** `spawn_async` callbacks ran inside a GLib future, so any callback opening a modal dialog (`Dialog::run`) panicked with
  `EnterError` and killed the app. Callbacks now run from an idle source. (Found by the GUI test; would have hit any error dialog shown after an async call.)
- Tests: 42 unit + 10 integration pass; clippy/fmt clean.

## Session 8
- Phase 9 (advanced messaging), reactions + edits + emoji:
  - `matrix::messages`: reactions (grouped by emoji, deduped, removable via redaction of the reaction event, early/late ordering handled),
    edits now carry sender + timestamp: **edits by anyone other than the original sender are ignored** and a stale edit never overwrites a newer one
    (previously any edit was accepted). Parser handles `m.reaction`.
  - `matrix::rooms`: `send_edit` (m.replace), `send_reaction`.
  - UI: right-click menu on a message (React submenu, Edit message..., Delete message) added via the label's own `populate-popup`
    (a selectable GtkLabel swallows right-clicks, so the old Delete menu never appeared); clickable reaction chips (bold = includes mine, tooltip lists
    people with nicknames applied); emoji picker button in the composer; "(edited)" no longer shown on deleted messages.
  - Verified in the GUI (nested X server, two accounts): reaction arrived live, chip toggle, send, Edit... (shows "(edited)"), emoji picker insert + send,
    Delete -> "message deleted". Integration test `edits_and_reactions_between_two_users` (foreign edit accepted by server but ignored; reactions add/remove) passes.
- Tests: 46 unit + 11 integration pass; clippy/fmt clean.

## Session 9
- Phase 9: file / image attachments.
  - `matrix::media`: `send_file` (size limit 50 MB, declares size + kind, MIME guessed, encrypted automatically in encrypted rooms),
    `download` (SDK decrypts; declared-size cap 100 MB), `sanitize_filename` (hostile names: `../..`, backslashes, control chars, dotfiles, length), `human_size`.
  - `matrix::messages`: `Attachment` (kind, filename, size, mime, source) parsed from m.file / m.image / m.audio / m.video.
  - UI: 📎 button (file chooser), attachment rows with size, Save... (file chooser, sanitised default name) and Open (downloads into
    `<data dir>/opened/<message>/<sanitised name>` and launches the default app), inline decrypted image previews (<= 8 MB, scaled, cached),
    list scrolls to the end after layout.
  - Verified: integration test (encrypted room, 70 KB binary + PNG round trip byte-for-byte, oversize refused) and the GUI (nested X server): preview rendered,
    file sent from the app, Save wrote a byte-identical copy, everything reloaded after restart.
- Tests: 51 unit + 12 integration pass; clippy/fmt clean.

## Known problems / not verified
- Image previews decode untrusted data with gdk-pixbuf (size-limited, failures ignored); "Open" hands files to the default application, as other clients do.
- No upload progress / cancel, no drag-and-drop or paste-image, no thumbnails sent with images, no audio/video playback (they appear as files).
- Attachments verified against Synapse and this client only (no Element).
- No interactive device verification (SAS), no per-message encryption indicators, no avatars, search or replies yet.
- Not implemented: friend requests, kick/ban and room settings, recovery reset/disable.
- The three-account GUI acceptance scenario has not been run end to end; hide-to-tray via a real close click and the installed .desktop icon are unverified on the user's panel.
- "Load older messages" can show when nothing older exists. Contact cache assumes a single account.

## Exact next task
1. Avatars: own avatar upload/edit in settings, contact and room avatars in the lists (cached, scaled), shown in conversation headers.
2. In-conversation search and replies.
3. Interactive verification (SAS emoji) + per-message encryption indicators.
4. Friend requests; third-party interop test (Element Web vs local Synapse); three-account acceptance scenario.
