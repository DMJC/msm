# TODO
## Phase 1–2
- [x] Verify toolchain, GTK3 and matrix-sdk dependency compatibility
- [x] Cargo project + module skeleton (contacts, storage, ui)
- [ ] Install/run rustfmt + clippy
- [ ] Logging config, XDG paths, clean shutdown
- [ ] Verify GTK window launches under a display
- [ ] docs/: ARCHITECTURE, REQUIREMENTS, MATRIX_COMPATIBILITY, SECURITY, TESTING, IMPLEMENTATION_STATUS
## Phase 3
- [ ] Login / discovery / session restore / logout / sync loop / profile
- [ ] Secure credential storage + fallback policy
## Phase 4
- [x] Contact model, resolver, manager, groups, LWW merge, SQLite cache (unit-tested)
- [ ] Account-data read/write with Matrix, conflict re-fetch on write
- [ ] Contact list UI, nickname editing UI, resolver used by all UI
- [ ] Ignored-user block/unblock
## Phase 5+
- [ ] DM discovery (m.direct), messaging, conversation UI, typing
- [ ] Group rooms/invites
- [ ] Presence, friend requests
- [ ] E2EE + verification
- [ ] Media, reactions, edits, redactions, search, receipts
- [ ] Integration tests vs local Synapse + Element; acceptance scenario
- [ ] Packaging, user docs
