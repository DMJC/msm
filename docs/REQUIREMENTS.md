# Requirements

The full specification is `MASTER_PROMPT.md`. In short:

- Rust, GTK3 (never GTK4), matrix-rust-sdk, Tokio, SQLite; Linux on Wayland and X11.
- MSN-style experience: contact list with groups, private nicknames, presence and status messages, conversation windows, notifications, tray.
- Standard Matrix interoperability (Element, FluffyChat): nicknames never touch Matrix profiles or room state.
- Contact sync across devices through account data; friend approval flow.
- End-to-end encryption, device verification, recovery.
- Non-blocking UI, graceful errors, persistence and session restore.

Status of each item is in `IMPLEMENTATION_STATUS.md` and `TODO.md`.
