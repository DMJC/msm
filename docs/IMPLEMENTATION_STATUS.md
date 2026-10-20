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
| Calendar invitations with Tentative/Accept/Decline and a personal calendar | Verified (two instances in the GUI; integration test over an encrypted room); no recurrence, edit or cancel |
| QR verification | Not implemented |
| Audio/video calls and playback | Not implemented |
| Packaging (.deb/.rpm/Windows) | Not started |
