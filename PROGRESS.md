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

## Session 10
- Phase 9: avatars.
  - `matrix::avatars`: `user_avatar` (profile lookup + 64px thumbnail), `room_avatar`, `set_own_avatar` (5 MB cap), `remove_own_avatar`; fetched bytes capped at 2 MB.
  - `ui::avatar`: defensive decode (dimension limit, centre-crop square, 40px) and stable colour placeholders; unit-tested.
  - App-level cache (`App::avatar`, keys `u:<mxid>` / `r:<room>#<avatar url>` so a room avatar change refetches); rows redraw when an avatar arrives.
  - UI: own picture button in the main window (Change picture... / Remove picture), avatars on contact and room rows and in conversation headers.
  - Avatars are Matrix profile/room state and are separate from local nicknames, which still never touch Matrix.
  - Verified: integration test `avatar_set_fetch_and_remove` (set, other user fetches, oversize refused, remove) and the GUI (nested X server): contact + room avatars,
    own picture chosen via the file dialog and confirmed on the server profile, header avatar.
- Tests: 52 unit + 3 bin + 13 integration pass; clippy/fmt clean.

## Session 11
- Phase 9: replies and in-conversation search.
  - Replies: `ChatMessage.reply_to` parsed from `m.in_reply_to`; `send_text(.., reply_to)` sends a standard reply relation (no quoted-text fallback, as in current spec);
    UI: right-click > Reply, reply bar above the composer with cancel, quote line above replies (name + snippet, "an earlier message" if not loaded), retry keeps the reply target.
  - Search: Ctrl+F / magnifier button, case/whitespace-insensitive filter over loaded messages with a match count (`ChatMessage::matches`).
  - `LiveEvent::Timeline` now boxes its event (clippy size lint).
  - Verified: unit tests, integration test (encrypted reply relation round trip, search match), and the GUI (reply bar, quote persisted across restart, search "1 found").
- Tests: 54 unit + 3 bin + 13 integration pass; clippy/fmt clean.

## Session 12
- Phase 9: interactive emoji (SAS) verification.
  - `matrix::verification`: `request` / `request_self`, `watch_incoming` (to-device and in-room requests; retries the SDK lookup briefly), `run` -> `Flow`
    (UI states Waiting/Requested/Starting/Emojis/Confirmed/Done/Cancelled; commands Accept/Match/Mismatch/Cancel). Only a user command confirms a match.
  - UI: verification window with the 7 emoji and names, "They match" / "They don't match" / Cancel; opens automatically for incoming requests;
    entry points: contact menu "Verify...", Security dialog "Verify another device...".
  - Fixed: right-clicking a contact also opened the enclosing group's menu.
  - Verified: integration tests (two fresh encrypted accounts: identical emoji both sides, nothing trusted before confirm, trust after confirm; mismatch cancels both sides without trust)
    and the GUI (nested X server): contact > Verify..., emoji identical to the second client's log, "Verified." after both confirmed.
- Tests: 54 unit + 3 bin + 15 integration pass; clippy/fmt clean.

## Session 13
- Per-message encryption indicators and verified badges.
  - `ChatMessage.trust` (`Unencrypted` / `Verified` / `Unverified`) from the SDK's `EncryptionInfo` for history and live events; a message's marker is ✔ (sent from a verified device),
    ⚠ (encrypted, sending device not verified), nothing for unencrypted; tooltip explains. Local echo adopts the trust of the server copy.
  - Contacts verified by us show ✔ in the list and "✔ verified" in the DM header (`App::refresh_trust` on each sync and after a verification completes).
  - Verified: unit test (echo adopts trust), integration assertion (decrypted messages carry a trust level), GUI: ⚠ on fresh messages from this (uncross-signed) device.
  - Not verified in the GUI: the green ✔ states (a verified contact / verified device) because the test device lacks cross-signing keys (needs recovery key entry on that device).
- Tests: 54 unit + 3 bin + 15 integration pass; clippy/fmt clean.

## Session 14
- Friend requests (application-level, separate from room invitations).
  - Transport: custom to-device event `com.matrixmessenger.friend.v1` `{action: request|accept|reject|remove}` (`src/matrix/friendship.rs`). No room or invite is created; other clients ignore it. The homeserver sets the sender.
  - State: `Contact.friendship` (`None`/`Outgoing`/`Incoming`/`Friends`), synced in the contacts account data and cached in SQLite (migration 2). `Friends` is only reached by their accept of our request, or our accept of their request (crossed requests also resolve to Friends and are acknowledged). Unsolicited accepts are ignored.
  - UI: "Friend requests" section with Accept/Decline; contact menu (send/resend/cancel request, remove friendship); "(request sent)" and 👥 markers; status line + desktop notification on incoming. Adding a contact never requires friendship.
  - Declining a request from someone who is only a contact because of the request drops the contact; an existing customised contact is kept.
  - Verified: 4 manager unit tests + parse test, integration test (request, accept, remove across two real accounts; no room/invite created), GUI (bob's request showed in the section, Accept turned it into 👥 and bob received the accept).
  - Limits: to-device messages reach only the recipient's devices that exist at send time (a device added later doesn't see an old request; "Resend friend request" covers it); the sender's side cannot tell a request was never delivered (no read ack).
- Three-account scenario (automated): `three_accounts_in_a_group_keep_independent_nicknames` registers alice/bob/carol, group room, everyone messages; each resolves the others by their own private nicknames, nicknames are stored only in their owner's account data, and public profiles/room member names are unchanged. Not run through the GUI with three live windows.
- Third-party interop (Element Web 1.12.32 in headless Chrome, driven with puppeteer, against local Synapse; msm library as the other side, since the GTK window can't share the headless run):
  - Verified both ways in an encrypted DM: msm-created DM is shown by Element as a direct chat invitation ("wants to chat"), Element decrypts msm's message; msm decrypts Element's messages; Element reply (`m.in_reply_to`) parsed with correct target; Element edit applied (`edited=true`, new text); Element reaction read; msm reply renders with quote in Element, msm reaction shows as a ❤️ pill, msm edit shows "(edited)", msm redaction shows "Message deleted".
  - Observed: every fresh login is a new device and cannot read earlier encrypted history (expected). Element flags msm's messages with a red shield (msm device not cross-signed).
  - Second Element run (encrypted DM): msm's image and text file render in Element with correct names/sizes/preview; msm's profile picture shows in Element (room list, header, timeline); Element's image and file are downloaded and decrypted by msm with matching mime types and byte sizes.
  - Not covered: SAS verification and presence against Element; Element's profile picture shown in msm; the GTK window itself was not used in these runs.
- Tests: 59 unit + 3 bin + 18 integration pass; clippy/fmt clean.

## Session 15
- Room settings and moderation: "Members & settings…" in group room windows (`src/ui/room_dialog.rs`, `rooms::room_settings/kick/ban/unban/set_room_name/set_room_topic`).
  - Lists joined, invited and banned members with admin/moderator tags; Kick/Ban/Unban buttons appear only if the room's power levels allow us to act on that member (never on ourselves); name/topic fields are disabled without permission. Kick/Ban ask for an optional reason. The server still enforces permissions.
  - Verified: integration test (creator vs ordinary member permissions, rename/topic reach others, kick, ban, banned can't be re-invited, unban) and GUI (non-admin sees disabled fields and no buttons; admin saved a topic, kicked, banned and unbanned with the list refreshing).
  - Not done: room join rules/visibility, reporting.

## Session 16
- "Load older messages" no longer shows when the first page already reached the start of the room (`load_history` drops the continuation token for a short page). Integration test `history_pagination_ends_at_the_start_of_the_room` (paging terminates, nothing older for a short room). Not re-checked in the GUI.
- Tests: 59 unit + 3 bin + 19 integration (the new one takes about a minute because of Synapse's send rate limit).

## Session 17
- Other users' avatar changes show up live: `avatars::watch_changes` reports a member event whose avatar differs from `prev_content`; the app drops the cached `u:<mxid>` avatar and refreshes. Integration test `avatar_change_in_a_shared_room_is_reported` passes (2 avatar tests, clippy clean); full suite and the GUI not re-run.
- Placeholder avatars show the contact's initial (cairo text on the flat colour; unit test for initials, rendering checked by dumping a PNG; not looked at in the running GUI).
- Reply quotes are clickable and scroll to the original (clearing a search that hides it; a hint if it is older than what is loaded). Verified in the GUI with the original scrolled out of view. Placeholder initials only for people, not rooms (checked live: room tiles were all "R" before).
- Drag-and-drop: dropping local files on a conversation window sends them (`send_file`, shared with the 📎 button). Verified in the GUI with a GTK drag source in Xephyr: the file was uploaded and shown as an attachment. Dropping onto the text input is handled by the text view itself and does not send.
- Paste image: Ctrl+V with an image on the clipboard sends it as a PNG attachment (temp file deleted afterwards); text paste is unchanged. Verified in the GUI with a clipboard-owning helper: uploaded and previewed.
- Full suite after Session 17 changes: 59 unit + 4 bin + 20 integration pass against local Synapse (integration takes about 61 s).
- Role changes: a "Role" menu (Member/Moderator/Admin) per joined member in Members & settings, shown only when power levels allow (`MemberRow.can_set_role`, `rooms::set_role`). Integration test (promote, moderator limits, demote; non-admin refused) and GUI (promoted and demoted a member, list refreshed) verified. Custom power levels other than 0/50/100 are shown but can't be set.
- Room picture: "Change picture…" / "Remove picture" in Members & settings, enabled only if we may send `m.room.avatar` (`avatars::set_room_avatar/remove_room_avatar`, `RoomSettings.can_set_avatar`). Integration test (creator sets, member refused, member sees and fetches it, removal) and GUI (picture chosen via the file dialog, shown in the conversation header) verified.
- Recovery: Security dialog gained "Reset recovery key…" and "Turn off recovery…" (enabled only when recovery is on and this device has the secrets; both confirm first; `encryption::reset_recovery_key/disable_recovery`). Integration test extended and passing (old key rejected and new key accepted on a fresh device; disabling leaves state Disabled and the key useless). GUI verified after the next fix: set up recovery, reset key (new key shown), turn off (confirm dialogs, status back to "not set up"). The device list is now in a scrolled area (max 200 px) so the dialog's buttons stay reachable with many devices; before, the dialog grew past the screen. Side effect: the green ✔ for a cross-signed own device was seen in the device list and "This device verified: yes".
- Verification-request unread count: investigated, it is SDK/server behaviour, left as a known problem.

## Known problems / not verified
- Verification needs both sides to already know each other's keys (e.g. a shared room) and the peer to have accepted the DM. The green ✔ states are not GUI-verified.
- The verification-request message in the DM counts as an unread message. This comes from the SDK's client-side unread count (any `m.room.message` from another user counts) and the server's count; not fixed.
- QR-code verification and verifying with a recovery key from a new device UI are not implemented.
- Search covers only messages already loaded (no server-side search); replies to unloaded messages show "an earlier message"; jump-to-original works only for loaded messages.
- Other users' avatar changes are now noticed live (Session 17: member-event watcher drops the cached avatar; integration test only, not seen in the GUI);  no avatar zoom/preview.
- Image previews decode untrusted data with gdk-pixbuf (size-limited, failures ignored); "Open" hands files to the default application, as other clients do.
- No upload progress / cancel, no thumbnails sent with images, no audio/video playback (they appear as files).
- Attachments verified against Synapse and this client only (no Element).
- No interactive device verification (SAS), green ✔ states unverified in GUI.
- Not implemented: join rules, resetting cross-signing identity, recovery-key reset when this device lacks the secrets.
- The three-account GUI acceptance scenario has not been run end to end; hide-to-tray via a real close click and the installed .desktop icon are unverified on the user's panel.
- Contact cache assumes a single account.

## Exact next task
1. Remaining interop checks vs Element (SAS verification, presence); optional three-account GUI run.
