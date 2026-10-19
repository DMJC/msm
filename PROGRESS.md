# PROGRESS

## Current phase
Phase 1 complete (repo audit, dependency verification). Phase 2 partially done. Phase 4 core logic done (not yet wired to UI/Matrix).

## Completed this session (all verified by `cargo test`: 22 passed, 0 failed)
- Cargo project, dependency set verified to resolve and compile (gtk 0.19, matrix-sdk 0.19.1, rusqlite, tokio).
- `contacts`: contact model, mxid validation, NameResolver (priority nickname > room name > profile name > MXID),
  ContactManager (add/remove/rename/reset/favourite/groups create/rename/delete/reorder/search),
  versioned account-data schema with defensive parsing and per-contact LWW merge + tombstones.
- `storage`: SQLite with migrations, contacts/groups cache, settings; newer-DB rejection; restart persistence test.
- Placeholder GTK3 window (compiles; NOT yet launched/visually verified).

## Files
Cargo.toml, src/main.rs, src/contacts/*, src/storage/*, src/ui/*, AGENTS.md, .gitignore

## Known problems / blockers
- rustfmt and clippy not installed; formatting/lint checks not run.
- libsecret dev headers absent: credential store choice (keyring crate vs fallback) undecided; needed before login work.
- Only dead-code warnings, since modules are not yet used by the app.
- No Matrix code written yet; no interoperability verified. No local homeserver set up (docker exists at /usr/bin/docker).
- GTK window never run (needs display); only compiled.

## Exact next task
Phase 3: `src/matrix/` — login (homeserver discovery + password), session persistence (SDK sqlite store under XDG data dir,
tokens via secret store with documented fallback), sync loop on a Tokio runtime with a channel into the GTK main loop
(glib::MainContext), logout. Then load/save `ContactsDocument` via account data and wire into a real contact-list UI.
Consider starting a Synapse container for integration tests.
