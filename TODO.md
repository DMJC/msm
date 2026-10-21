# TODO
## Phase 1–2
- [x] Add GPLv3 License header to all files. Copyright James Carthew 2026 don't reference claude as a contributor.
- [x] Verify toolchain, GTK3 and matrix-sdk dependency compatibility
- [x] Cargo project + module skeleton (contacts, storage, ui)
- [x] rustfmt + clippy installed and clean
- [x] Logging (tracing, RUST_LOG), XDG data dir (directories crate); [x] clean shutdown (offline presence on quit, verified)
- [x] GTK window launches under Wayland (no visual check)
- [x] docs/: ARCHITECTURE, REQUIREMENTS, MATRIX_COMPATIBILITY, SECURITY, TESTING, IMPLEMENTATION_STATUS
## Phase 3
- [x] Login + discovery + sync loop with backoff (login verified vs Synapse)
- [x] Session restore / logout verified vs Synapse; profile fetch/set display name & avatar
- [x] Secure credential storage (oo7), verified with real keyring
## Phase 4
- [x] Contact model, resolver, manager, groups, LWW merge, SQLite cache (unit-tested)
- [x] Account-data read/write with merge, verified across two installations vs Synapse
- [x] Contact list UI, nickname editing UI using resolver (launch-verified; interactions not exercised)
- [x] Ignored-user block/unblock (contact context menu; account-wide m.ignored_user_list; verified vs Synapse + GUI; Security → "Blocked users…" lists, unblocks and blocks by ID)
- [x] (done, see Session 17 calendar addendum; organiser edit and cancel added in Session 18 (library + integration verified, buttons verified in the GUI on the organiser side); personal entries can repeat, Session 18) Create a calendar where users can send each other event invites but a user must tentatively accept an invite for it to be added to their calendar. They can RSVP to an invite. Personal entries (Add event… in the Calendar window, local only) added in Session 18. Desktop reminders 10 min before events (Session 18).
## Phase 5+
- [x] DM discovery (m.direct), messaging, conversation UI, typing (verified vs Synapse and in the GUI; text, replies, edits, reactions, redactions also vs Element Web)
- [x] Group rooms/invites: list, create, invite, accept/decline, leave, room windows (verified vs Synapse + GUI)
- [x] Membership management: kick/ban/unban, room name/topic (verified vs Synapse + GUI); [x] role changes (member/moderator/admin); [x] room picture; [x] join rules (invite-only / anyone; other rules shown read-only); [x] room directory listing
- [x] Own status + tray + presence publishing (verified vs Synapse and on Wayland/MATE panel)
- [x] Automatic Away timer (Wayland ext-idle-notify; verified on real Wayfire session)
- [ ] Contact presence display verified with two users sharing a room
- [x] Friend requests (to-device protocol; verified vs Synapse + GUI; not vs a second client implementation)
- [x] Cross-signing + recovery key + key backup (verified vs Synapse and in the GUI)
- [x] Interactive emoji (SAS) verification (verified vs Synapse + GUI)
- [x] QR verification: show our code, scan theirs with a webcam (ffmpeg); verified by integration test, real-camera frame test and two-instance GUI; phone/Element scan untested
- [x] Per-message encryption indicators, verified badges (⚠ path GUI-verified; ✔ path not)
- [x] Redaction (delete own message), read receipts, unread counts
- [x] Reactions, sending edits, emoji picker (verified vs Synapse + GUI)
- [x] File/image attachments (encrypted round trip verified; GUI verified)
- [x] Avatars (own set/remove, contacts, rooms, headers; verified vs Synapse + GUI)
- [x] In-conversation search (loaded messages), replies (verified vs Synapse + GUI)
- [x] Integration tests vs local Synapse (17 tests) and an Element Web interop run (text, replies, edits, reactions, redactions, attachments, avatar); [ ] Element checks for verification, presence
- [x] 1:1 voice calls (Session 18; automated tests + GUI check, Element/NAT/real audio untested)
- [ ] Video calls; group calls
- [x] README with install and usage (scripts/install-local.sh); [x] distro packaging (deb, rpm)

## Platform
- [x] Native Wayland (GTK3), StatusNotifierItem tray with status menu
- [ ] Verify taskbar icon via installed .desktop on the user's panel
- [ ] Hide-to-tray on window close verified by a real click
- [x] Debian .deb: `cargo deb` metadata + scripts/build-packages.sh (built and inspected locally; extracted binary starts); CI workflow .github/workflows/packages.yml ran green on the v0.1.0 tag (GitHub Actions, ubuntu-24.04)
- [x] RPM: `cargo generate-rpm` metadata, built and its file/dependency list inspected; not installed on an RPM-based system
- [ ] AppImage Package
- [ ] Microsoft Windows compatible client
- [ ] Windows arm/x86_64 .exe automated builds.
