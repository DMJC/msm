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
- [ ] DM discovery (m.direct), messaging, conversation UI, typing
- [ ] Group rooms/invites
- [ ] Presence, friend requests
- [ ] E2EE + verification
- [ ] Media, reactions, edits, redactions, search, receipts
- [ ] Integration tests vs local Synapse (partial: login, nickname sync, DM) + Element; full acceptance scenario
- [ ] Packaging, user docs
