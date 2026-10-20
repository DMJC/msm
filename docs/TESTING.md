# Testing

```
cargo test                                    # unit tests; integration tests skip without a homeserver
scripts/test-homeserver.sh <workdir>          # throwaway Synapse on :8008 with alice/bob/darkmage (pw-<name>-123)
MSM_TEST_HOMESERVER=http://localhost:8008 cargo test   # adds tests/homeserver.rs (≈4–5 min)
cargo clippy --all-targets
```

- Unit tests cover contacts, merge, resolver, message model, avatars, presence and friendship parsing.
- `tests/homeserver.rs` exercises login, session restore, contacts sync across installations, DMs, groups, invites, moderation and roles, join rules, room pictures, edits/replies/reactions, media, presence, friendship, SAS verification (match and mismatch), recovery, block list and live avatar changes against a real Synapse.
- Integration tests run in parallel against shared accounts: never block, kick or change a real test account's state without restoring it, and do not block alice/bob/darkmage.
- GUI checks are done manually under Xephyr (`Xephyr :5 -screen 520x760`, `GDK_BACKEND=x11`) driven with xdotool; Element Web interop was checked with headless Chrome. These are recorded in PROGRESS.md, not automated.
