# Manual tests for a second machine

These are the checks automated testing could not do. Install msm on the second PC
(`cargo build --release`, or the .deb/.rpm from the GitHub release) and sign in as a
second account (or the same one) on the same homeserver. Note the result of each in
PROGRESS.md.

0. **Browser (Google/SSO) sign-in.** Homeserver `matrix.org`, press "Sign in with your browser…", complete Google sign-in; expected: the contact list appears and the session survives a restart. Report any error text.
1. **QR verification with a real camera.** On the new machine run
   `cargo run --example scan_qr` and hold a QR code (e.g. from Element's "Verify
   session") up to the camera; expected: the payload is printed. Then in msm:
   Security → Verify another device…, choose "Scan their QR code" and confirm both
   sides show ✅ Verified. Set `MSM_CAMERA=/dev/videoN` if the first camera is wrong.
   Needs `ffmpeg` installed.
2. **Verification with Element.** Verify msm from Element (emoji and QR) and the reverse.
3. **Packages.** Install the .deb (Debian/Ubuntu) or .rpm (Fedora) on a clean machine:
   the app starts, the icon and menu entry appear, and sign-in works.
4. **Tray and close.** Closing the window hides it to the tray; the tray menu shows it
   again and quits. Test on Wayland and X11.
5. **Cross-machine messaging.** Chat between the two installs: text, attachments,
   typing indicator, presence changes, nicknames (visible only on the machine that set
   them), friend requests and groups.
6. **Encrypted rooms.** Send in an encrypted room from both machines, including after a
   restart (history decrypts, keys restored with the recovery key on the new device).
7. **Calendar.** Send an invitation to the other machine; accept it there; edit and
   cancel it from the organiser; check the reminder notification appears at the chosen
   lead time and does not repeat after restarting the app.
8. **Personal repeating events.** Add a weekly event, check the repeat label and that
   the next occurrence is shown; change "Remind me" in the Calendar window.
9. **Voice calls (two machines).** Both signed in and already chatting in a direct
   chat (the other person must have joined it). Press 📞 in the chat window: the other
   machine shows a notification and a call window; Answer; speak and listen with
   headsets (there is no echo cancellation, speakers will feed back); try Mute and Hang up
   from both sides; try Decline; try calling while on a call (the caller gets "busy").
   Needs `pulseaudio-utils` (`parec`, `pacat`). Report one-way audio or a failed
   connection; across different networks this needs the homeserver's TURN.
10. **Voice calls with Element.** Call msm from Element (1:1 voice) and call Element
    from msm; check both directions connect and audio flows.
