# Implementation status

Verified means exercised against a local Synapse (integration test) and/or in the GTK window; see PROGRESS.md for the dated evidence.

| Area | State |
| --- | --- |
| Login, restore, logout, Secret Service storage | Verified |
| Sync loop with backoff, tray (StatusNotifierItem), idle → Away | Verified (tray close-click and panel icon not checked on a real panel) |
| Contacts, groups, favourites, nicknames, LWW sync | Verified |
| Friend requests (custom to-device) | Verified |
| Presence and status messages | Verified; Element presence inside msm not checked |
| DMs and group rooms, invites, leave | Verified |
| Messaging: replies (with jump), edits, redactions, reactions, typing, search (Enter scans older history) | Verified (also vs Element Web) |
| Attachments: send, drag-and-drop, paste image, download/open, upload progress + cancel | Verified (progress/cancel at library level; the GUI Cancel button not exercised) |
| Avatars incl. live change and initials | Verified |
| Room moderation: kick/ban/unban, roles, name/topic/picture, join rules | Verified |
| E2EE, SAS verification, recovery set up/enter/reset/off | Verified (SAS vs Element both ways) |
| Block / unblock | Verified |
| Calendar invitations with Tentative/Accept/Decline and a personal calendar | Verified (two instances in the GUI; integration test over an encrypted room); organiser edit and cancel verified by unit + integration tests (organiser-side buttons exercised in the GUI); personal entries can repeat daily, weekly or monthly (unit-tested, form not GUI-checked) |. Personal local-only entries can be added, edited and removed.. Reminders: desktop notification before an event (lead time selectable in the Calendar window, default 10 min; sent reminders survive restarts).
| QR verification | Implemented: show our code, scan theirs with a webcam (`src/camera.rs`, needs `ffmpeg`). Verified by integration test (grid → frame → decoder → scan → both Done), a real-camera frame test and a two-instance GUI run; a phone scanning our code or us scanning a phone is untested |
| 1:1 voice calls (Matrix VoIP v1, WebRTC/Opus) | Implemented: engine + signalling verified between two clients through Synapse (encrypted-room signalling, audio both ways, mute, hang up, decline) and the GUI window; NOT verified: Element interoperability, calls across NAT/TURN, real two-person audio quality. Video calls and in-app media playback: not implemented |
| Packaging | .deb and .rpm build locally via scripts/build-packages.sh (not installed on target distros; the CI workflow ran green on tag v0.1.0); Windows not started |
