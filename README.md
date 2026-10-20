# Matrix Messenger (msm)

An MSN-Messenger-style desktop client for Matrix, written in Rust with GTK3 and matrix-rust-sdk. It talks to ordinary Matrix homeservers and works alongside Element and other clients.

## Install

Requirements: Rust (2024 edition toolchain), GTK 3 development files, a C toolchain, `pkg-config`, and for credential storage a Secret Service provider (GNOME Keyring, KWallet, …; an encrypted file keyring is used if none is running). On Debian/Ubuntu: `libgtk-3-dev build-essential pkg-config libssl-dev libsqlite3-dev`.

```
scripts/install-local.sh      # builds --release, installs ~/.local/bin/msm, the .desktop entry and icon
```

or `cargo run --release`.

## Using it

1. Sign in with your homeserver (`matrix.org`, or a full URL such as `http://localhost:8008`), username and password. The session is restored on the next start; **Sign out** wipes it.
2. **Add contact** takes a Matrix ID (`@user:server`). Right-click a contact for: send message, friend request, verify, rename (private nickname), favourites, move to group, block, remove.
3. **Nicknames are private.** They are saved in your own account data and sync between your installations; they never change anyone's Matrix profile or any room.
4. Status (Online/Away/Busy/Appear offline), status message and automatic Away after idle are in the main window; the tray icon offers the same. Closing the window hides it to the tray when a tray is available; **Quit** in the tray menu exits.
5. Conversations: replies (click a quote to jump to the original), edits, reactions, attachments (button, drag-and-drop, or paste an image), Ctrl+F search (Enter searches older history). **Members & settings…** in a room manages name, topic, picture, who can join, directory listing, roles, kick/ban.
6. **Security…** shows encryption status and devices, and offers recovery key setup/entry/reset, identity reset and the blocked-users list. Verify people with **Verify…** (emoji comparison).

## Development

See `CLAUDE.md`, `MASTER_PROMPT.md`, `PROGRESS.md`, `TODO.md` and `docs/` (architecture, security, testing, Matrix compatibility, implementation status). Run the tests with `cargo test`; add `MSM_TEST_HOMESERVER` (see `docs/TESTING.md`) for the integration suite.

## Licence

GNU General Public License v3.0 or later. See `COPYING`.
