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
  - SAS emoji verification against Element Web (Session 18), both directions in an encrypted DM, with msm's library driving one side (msm's own flow: `verification::watch_incoming/request/run`): Element "Verify User" → msm saw the request, accepted, and both showed the same 7 emoji (Key, Train, Spanner, Rocket, Banana, Unicorn, Elephant); after "They match" on both sides msm reached Done and Element showed "You've successfully verified" with a green shield on the DM header. Then msm requested: Element showed "wants to verify", both showed the same 7 emoji (Headphones, Apple, Pizza, Scissors, Corn, Heart, Hourglass) and finished. Element also showed msm as online (green presence dot in the DM header).
  - Not covered: Element's presence and profile picture shown inside msm; the GTK window itself was not used in these runs (library only).
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
- Verifying with a recovery key from a new device UI exists (Security → Enter recovery key).
- Search filters loaded messages as you type; Enter pages back through up to 30 pages of history (client-side, so encrypted rooms work); replies to unloaded messages show "an earlier message"; jump-to-original works only for loaded messages.
- Other users' avatar changes are now noticed live (Session 17: member-event watcher drops the cached avatar; integration test only, not seen in the GUI);  no avatar zoom/preview.
- Image previews decode untrusted data with gdk-pixbuf (size-limited, failures ignored); "Open" hands files to the default application, as other clients do.
- Upload progress/cancel exists (library-verified); no download progress (the SDK offers none), no thumbnails sent with images, no audio/video playback (they appear as files).
- Attachments verified against Synapse and this client only (no Element).
- SAS verification is implemented and tested between two msm clients (integration tests); the green ✔ on our own cross-signed device was seen in the GUI, but ✔ on other people's messages/devices has not been.
- Not implemented: recovery-key reset when this device lacks the secrets (identity reset covers that case), QR verification, in-app audio/video.
- The three-account acceptance scenario was observed in the GUI for nicknames and group display (see the last Session 17 addendum), not with Element in the same run; hide-to-tray via a real close click and the installed .desktop icon are unverified on the user's panel.
- Contact cache assumes a single account.

## Exact next task
Everything feasible without a real desktop panel, a camera, or a Windows machine is done and verified (see the Session 17 addenda). What is left needs something this environment lacks:
1. On a real Wayland/X11 desktop with a tray: confirm hide-to-tray on a real close click and that the installed .desktop icon shows on the panel (TODO.md Platform section).
2. Install the built .deb/.rpm (`scripts/build-packages.sh`) on a clean Debian/Fedora system; run the CI workflow once a remote exists.
3. Optional features: in-app audio/video playback, voice/video calls, download progress, Windows build, calendar extras (recurrence, edit/cancel, sync across installations).
4. Re-run the three-account scenario with Element in the same session if desired.
5. Try QR verification against a real client: `cargo run --example scan_qr` while showing Element's verification QR to the camera, and have Element scan msm's code (set `MSM_CAMERA` if /dev/video0 is wrong).

## Session 17 addendum: invite fix
- Full suite re-run found `invite_user` failing ("device is not verified") once the invitee had a cross-signing identity (accounts used in the Element SAS runs). Cause: matrix-sdk shares room history before inviting and aborts if this device is not cross-signed. `rooms::invite_user` now falls back to a plain invite request in that case.
- Results: 59 unit + 4 bin + 20 integration pass, clippy clean.
- Next task unchanged: GTK ✔ check for SAS-verified users, Element presence/avatar in msm, optional three-account GUI run.

## Session 17 addendum: ✔ path review
- The contact-list/conversation ✔ comes from `App::verified_users` (app.rs ~610), filled with `get_user_identity(..).is_verified()` — the same predicate `emoji_verification_between_two_users` asserts true after SAS (and false before/after a mismatch). Verified by code path + that test; not by a live GTK screenshot with two real accounts.
- Next task: optional three-account GUI run; Element presence/avatar inside msm.

## Session 17 addendum: licensing
- GPLv3-or-later header (Copyright 2026 James Carthew, no tool attribution) added to all 36 .rs files; `COPYING` holds the GPL-3 text; Cargo.toml `license = "GPL-3.0-or-later"`. Clippy clean, tests pass (integration tests skip without MSM_TEST_HOMESERVER).

## Session 17 addendum: join rules
- Room settings now has "Who can join" (Invite only / Anyone can join / Other, read-only), gated by `m.room.join_rules` power level; `rooms::set_join_access`, `RoomSettings.join_access/can_set_join_access`.
- Verified: integration test (non-admin refused and shown can_set=false; admin changes it, member sees it sync; `Other` rejected) and the live GTK dialog (shows "Invite only", control greyed out for a non-admin). The enabled-control click in the GUI was not exercised. Room-directory visibility is not implemented.

## Session 17 addendum: block/unblock
- `matrix::ignore` (block/unblock/blocked) edits the server's `m.ignored_user_list` (the SDK's own `ignore_user`/`unignore_user` edit the local copy and silently no-op/clobber if sync hasn't echoed a recent change). Contact menu has "Block user"/"Unblock user"; blocked contacts show "(blocked)". `App::refresh_blocked` runs each sync.
- Verified: new integration test (invalid/self rejected, block/list/unblock, visible from a second session) and live GTK (Sandy → "(blocked)" and menu flips to Unblock; unblocked again). Full suite: 59 unit + 4 bin + 21 integration pass, clippy clean.
- Not done: a blocked non-contact can't be unblocked from the UI; the test must not block a real test account (parallel tests would lose its messages).

## Session 17 addendum: blocked-users dialog
- Security → "Blocked users…" (`ui/blocked_dialog.rs`) lists the ignore list with Unblock buttons and blocks by Matrix ID. Verified live in the GTK GUI: block `@stranger:localhost` (non-contact) → listed → Unblock → "Nobody is blocked." Closes the previous known gap. Clippy clean; the library calls are covered by the existing integration test.

## Session 17 addendum: docs
- Added docs/ARCHITECTURE, REQUIREMENTS, SECURITY, TESTING, IMPLEMENTATION_STATUS (written from the code and test records; no code change). Logging/XDG items were already implemented; explicit clean-shutdown handling is still open.

## Session 17 addendum: clean shutdown
- `App::shutdown` (GApplication `shutdown`): aborts the sync task and publishes offline presence (2 s bound). Verified live: bob's view of alice went online → offline after the tray "Quit" (triggered through the dbusmenu `Event` call; exit confirmed in the log "set presence offline on quit").
- Side observation: a WM_DELETE_WINDOW close event sent to the main window left the process running (hide-to-tray behaviour with the tray registered) — consistent with, but not a screenshot-verified proof of, hide-to-tray.

## Session 17 addendum: upload progress / cancel
- `media::send_file_with(room, path, Transfer{progress, cancel})` (new dep `eyeball` for the SDK's progress observable). Conversation window shows "Uploading name… N%" with a Cancel button, refuses a second upload while one runs.
- Verified: integration test (progress reported; pre-signalled cancel → `Cancelled` and no message posted); full suite 59+4+21 pass, clippy clean. Not verified: the GUI Cancel click and visible percentage (local uploads finish instantly). Cancelling drops the request; if the media upload had already finished but the event not yet sent, an orphaned media file may remain on the server.
- Download progress/cancel not done.

## Session 17 addendum: search older history
- Typing in the conversation search still filters loaded messages; pressing Enter now pages backwards (up to 30 pages of 30) until the start of the room, showing "N found · searching older…" / "reached the start" / "stopped; press Enter to search further back". A new query cancels a running scan.
- Verified live in the GTK window: 80-message room, keyword only in message 3 → 0 found while typing, 1 found after Enter. Works on decrypted client-side history, so it also covers encrypted rooms. No automated test (UI-only logic). The ellipsized status label was added after the screenshot and not re-checked visually.

## Session 17 addendum: custom power levels
- Role menu gained "Custom level…" (0–100 prompt, validated); members at other levels show "(custom level)", ≥100 (including the room creator's effectively infinite level) "(admin)". Verified: integration test (level 25 round-trips) and live GTK (set Sandy to 25 → "(custom level)"). Note Synapse accepted a level above the creator's 100 in the test room, so "can't exceed your own level" is server-defined, not asserted.

## Session 17 addendum: directory listing
- Room settings checkbox "Show in the server's public room directory" (`rooms::set_listed`, `RoomSettings.listed`), enabled for those who may change join rules; the server decides (Synapse refuses unless `room_list_publication_rules` allows — the test script `scripts/test-homeserver.sh` now configures an allow rule, as Synapse otherwise returns 403 "Not allowed to publish room").
- Verified: integration test (unlisted → listed → unlisted, read back from the server) and live GTK toggle. Not observed: the room appearing in `/publicRooms` — Synapse only lists published rooms that are also joinable by anyone, and the test room was invite-only.

## Session 17 addendum: cross-signing identity reset
- Security → "Reset cross-signing identity…" (`encryption::reset_identity`): confirm, password (UIA), new cross-signing keys; if recovery existed its key is replaced and shown. OAuth-based accounts get an error pointing at the provider's approval URL (not exercised).
- Caveat: the SDK regenerates local keys before the server accepts them, so a wrong password leaves this device out of step until the reset is retried; the UI error says so only via the message, retry works (tested).
- Verified: integration test on a fresh user (wrong password refused with server keys unchanged; reset changes the master key and returns a new recovery key; status stays ready/Enabled) and live GTK (confirm → password → new key differs from the old). Full suite 59+4+22 pass, clippy clean. Signed alice out of the GUI during this check (her session was wiped; test accounts still work).

## Session 17 addendum: acceptance scenario in the GTK GUI
- Ran two msm instances at once (alice on Xephyr :5, bob on :6), each in `dbus-run-session` with its own `XDG_DATA_HOME` (fresh installations). With no Secret Service on the private bus, the first login took about a minute to show the main window (oo7 waited for the secrets D-Bus service to time out); a normal desktop with a keyring is unaffected.
- Server state: darkmage's public display name is "Darkmage"; alice's account data has nickname "James Carthew" for @darkmage:localhost, bob's has "DMJC".
- Observed: alice's fresh install lists the contact as "James Carthew", bob's as "DMJC" (the nicknames arrived through account data, i.e. synchronised between installations with no local state). In the 3-member group "Acceptance Group" darkmage's message shows as "James Carthew" for alice and "DMJC" for bob. The profile endpoint still returns "Darkmage", so no profile change. Same MXID throughout.
- Earlier evidence covers the rest: Element interop (messages, replies, encryption, SAS) and the nickname-sync integration test. Not re-done in this run: restarting the same installation and Element showing the group in the same session.

## Session 17 addendum: calendar invitations
- Design (src/calendar, interop-friendly): an invitation is an ordinary `m.room.message` (`m.text`, readable body "📅 Invitation: title — time") carrying a structured field `com.matrixmessenger.calendar.v1`; an answer is another text message with `kind: rsvp`, the invitation's event ID and tentative/accepted/declined. Other clients just show the text.
- Invitee's calendar = local SQLite `calendar_entries` (migration 3): an invitation is added only when the invitee answers Tentative or Accept; Decline keeps the record but hides it. Organisers' own events go on their calendar as accepted. Not synced between a user's installations (local only).
- UI: 📅 button in the conversation (title, date YYYY-MM-DD, time HH:MM local, details), an invitation card with Tentative/Accept/Decline, a Calendar button in the main window listing entries with Remove.
- Verified: 7 unit tests (content round trip, parsing, malformed input, local-time parsing, DB behaviour, message parsing), integration test over an encrypted room (invitation → structured read by Bob, answer read by Alice), and a live two-instance GUI run (alice invites → bob sees the card → Tentative → alice sees the answer, bob's Calendar window lists it as tentative). Full suite 66+4+23 pass, clippy clean.
- A GUI run caught a bug (my new 📅 button shadowed the existing "Invite…" button variable, so it opened the room-invite prompt); fixed before commit.
- Not done: recurrence, editing/cancelling an invitation, reminders, calendar sync across installations, time zones other than the viewer's local one, showing RSVP summaries on the invitation.

## Session 17 addendum: packaging
- `Cargo.toml` has `[package.metadata.deb]` and `[package.metadata.generate-rpm]`; `scripts/build-packages.sh` builds both into `target/packages/` (needs `cargo install cargo-deb cargo-generate-rpm`). Verified: both files build; `dpkg-deb -I/-c` shows the right control data, auto-detected GTK/cairo/glib dependencies, binary, desktop entry, icon, README, copyright; `rpm -qpl/-qpR` lists the same files and requires gtk3; the extracted .deb binary starts under Xephyr.
- Not verified: actually installing either package (needs root / an rpm-based distro), the GitHub Actions workflow (no remote configured, never run), the rpm's runtime dependency names on Fedora.
- Windows client/builds remain untouched.

## Session 17 addendum: GitHub
- Remote `origin` = https://github.com/DMJC/msm (default branch `main`; local `master` tracks it). Pushed with the GitHub initial LICENSE commit merged in. Tag `v0.1.0` pushed; the `packages` workflow (run 39393512006) succeeded on all steps: dependencies, `cargo test --lib`, `scripts/build-packages.sh`, artifact upload (`linux-packages`, the .deb and .rpm).
- Still unverified: installing those packages on a clean Debian/Fedora system.

## Post-packaging verification (2026-10-21)
- Full suite re-run against local Synapse after the packaging additions: 66 unit + 4 bin + 23 integration pass, clippy clean.
- Remaining items are all blocked on hardware/OS access (see "Exact next task").

## Session 18 addendum: QR verification
- `matrix-sdk` `qrcode` feature, `rqrr` decoder, `qrcode` for the module grid. `src/camera.rs`: frames from `ffmpeg` (V4L2, 640x480 gray, `MSM_CAMERA` overrides the device), `decode_frame` returns the binary payload.
- `matrix::verification`: the request now advertises SAS + QR show/scan + reciprocate explicitly (the SDK default does not offer scanning). `Ready { qr }` replaces the auto SAS start; `VerifyCmd::{StartEmoji, ScanQr}`; `QrScanned` asks the person to confirm. Dialog draws our code (cairo), has "Scan their QR code" and "Compare emoji instead"; emoji are laid out four per row.
- Bugs found by the GUI/harness and fixed: generating the QR twice made the SDK cancel both verifications; `we_started` follows the request, so when the accepting device picked emoji nobody accepted the SAS (now accepted unconditionally, a no-op for the sender); a device offline at sign-in may not know the other device (`refresh_own_devices`).
- SDK behaviour (not a bug here): the trusted device cannot send a self-verification request to a not-yet-signed new device; the new device must start it. The dialog says so.
- Verified (full suite: 69 unit + 4 bin + 26 integration pass, clippy clean): camera unit tests, integration tests (QR between two users, emoji started by the accepting device in both directions, emoji and mismatch), `camera_delivers_frames` (ignored; needs a webcam) with the real Logitech BRIO, and a two-instance GUI run (QR rendered on both, screenshot of the on-screen code decodes to a `MATRIX` payload, emoji started from the accepting device, both ended "Verified").
- Not verified: scanning with a phone or Element, and a live scan of a code through the real camera (the room was dark and nothing was held up).
