# Architecture

Rust, GTK3 (gtk-rs 0.19), matrix-rust-sdk 0.19 (e2e, sqlite store), Tokio. GTK4 is out of scope.

## Crates and layout
`msm` is one package with a library (`src/lib.rs`, everything testable without a display) and a binary (`src/main.rs`, `app.rs`, `ui/`, `tray.rs`, `idle.rs`, which need GTK).

| Path | Role |
| --- | --- |
| `src/contacts/` | Contact model, nickname resolver, manager (groups, favourites, friendship state), last-writer-wins merge document (`synchronization.rs`). No Matrix or GTK dependency. |
| `src/matrix/` | All SDK use: `authentication` (login/restore/logout), `synchronization` (sync loop with backoff), `rooms` (DMs, groups, history, members, settings, power levels), `messages` (timeline model, edits, replies, reactions, trust), `media`, `avatars`, `presence`, `friendship` (custom to-device events), `contacts_sync` (account data), `encryption`, `verification` (SAS), `ignore` (block list), `secrets` (Secret Service). |
| `src/storage/` | App SQLite database (contact cache, migrations). The SDK keeps its own store. |
| `src/ui/` | GTK3 windows: login, main contact list, conversation, room settings, security, verification, blocked users, avatars. |
| `src/app.rs` | `App`: shared UI state (`Rc`), session start, sync-event dispatch, refresh. |

## Threading
GTK widgets live on the main thread. Network and database work runs on a Tokio runtime via `app::spawn_async(future, on_done)`; the result callback runs back on the GTK main loop. Background streams (sync events, typing, avatar changes, friend events) reach the UI through `async-channel`.

## Identity model
The Matrix ID is the identity. A contact's public display name comes from the profile; the private nickname lives only in the user's own account data (`com.matrixmessenger.contacts.v1`) and is never written to a profile or room. `NicknameResolver` picks nickname, then public name, then MXID.

## Contact sync
Account data replaces whole events, so each contact carries `updated_at` and a tombstone; documents are merged per contact, last writer wins with deterministic ties, so devices converge. Friend requests use custom to-device events (`com.matrixmessenger.friend.v1`) that other clients ignore.

## Rendering refresh
`main_window::refresh` rebuilds the contact list only when a signature of its inputs changes; avatars are cached by `u:<mxid>` / `r:<room>#<url>` keys and redrawn when `avatar_gen` bumps.
