# TODO
## Phase 1–2
- [x] Verify toolchain, GTK3 and matrix-sdk dependency compatibility
- [x] Cargo project + module skeleton (contacts, storage, ui)
- [x] rustfmt + clippy installed and clean
- [ ] Logging config, XDG paths, clean shutdown
- [x] GTK window launches under Wayland (no visual check)
- [ ] docs/: ARCHITECTURE, REQUIREMENTS, MATRIX_COMPATIBILITY, SECURITY, TESTING, IMPLEMENTATION_STATUS
## Phase 3
- [x] Login + discovery + sync loop with backoff (login verified vs Synapse)
- [x] Session restore / logout verified vs Synapse; profile fetch/set display name & avatar
- [x] Secure credential storage (oo7), verified with real keyring
## Phase 4
- [x] Contact model, resolver, manager, groups, LWW merge, SQLite cache (unit-tested)
- [x] Account-data read/write with merge, verified across two installations vs Synapse
- [x] Contact list UI, nickname editing UI using resolver (launch-verified; interactions not exercised)
- [ ] Ignored-user block/unblock
## Phase 5+
- [x] DM discovery (m.direct), messaging, conversation UI, typing (verified vs Synapse and in the GUI; not vs Element)
- [x] Group rooms/invites: list, create, invite, accept/decline, leave, room windows (verified vs Synapse + GUI)
- [ ] Membership management beyond invite/leave (kick/ban, room settings)
- [x] Own status + tray + presence publishing (verified vs Synapse and on Wayland/MATE panel)
- [x] Automatic Away timer (Wayland ext-idle-notify; verified on real Wayfire session)
- [ ] Contact presence display verified with two users sharing a room
- [ ] Friend requests
- [x] Cross-signing + recovery key + key backup (verified vs Synapse and in the GUI)
- [ ] Interactive device verification (SAS), per-message encryption indicators
- [x] Redaction (delete own message), read receipts, unread counts
- [x] Reactions, sending edits, emoji picker (verified vs Synapse + GUI)
- [x] File/image attachments (encrypted round trip verified; GUI verified)
- [ ] Avatars, in-conversation search, replies
- [ ] Integration tests vs local Synapse (partial: login, nickname sync, DM) + Element; full acceptance scenario
- [ ] Packaging, user docs

## Platform
- [x] Native Wayland (GTK3), StatusNotifierItem tray with status menu
- [ ] Verify taskbar icon via installed .desktop on the user's panel
- [ ] Hide-to-tray on window close verified by a real click
- [ ] Debian .deb package automated builds
- [ ] Redhat .rpm package automated builds
- [ ] Microsoft Windows compatible client
- [ ] Windows arm/x86_64 .exe automated builds.
