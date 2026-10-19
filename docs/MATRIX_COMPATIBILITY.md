# Matrix compatibility notes

## Private nicknames
Nicknames live only in the user's private account data (`com.matrixmessenger.contacts.v1`) and local SQLite cache.
They never modify the target's Matrix profile or any room state, and sender identities on events are never rewritten.
Account data is **not** end-to-end encrypted: the homeserver operator can read nicknames and contact groups.
Merging is per contact, last-writer-wins by `updated_at`, with deletion tombstones. Verified against Synapse 1.163.

## Direct messages
DMs are found via `m.direct` (SDK `get_dm_rooms`); several DMs with one user are possible. When choosing,
a room where the peer is joined beats one where they are only invited. New DMs are created encrypted
(SDK default: `m.room.encryption` initial state). Verified: two instances of this client exchanging
encrypted DMs, history, redaction and idempotent retry (same transaction ID) on Synapse.
NOT yet verified against Element or other third-party clients.

## Presence
Matrix has three presence values, so MSN statuses map as follows:

| UI status      | `presence`    | `status_msg`                 |
|----------------|---------------|------------------------------|
| Online         | `online`      | user message                 |
| Away           | `unavailable` | user message                 |
| Busy           | `unavailable` | `Busy` / `Busy - <message>`  |
| Appear offline | `offline`     | user message                 |

Other clients see Busy as "unavailable" with a "Busy" message. Contacts without presence information are
shown as *unknown*, never offline: servers may disable presence, and Synapse only shows presence of users
you share a room with (a 403 otherwise).
Implementation notes (found by testing on Synapse):
- `/sync` marks the user online unless it carries `set_presence`, so the sync loop sends the wanted presence.
- The presence PUT can take many seconds; it runs off the sync loop, after one quick sync, in order.
- Synapse may take ~10-20 s to reflect a change.

## Automatic Away
Idle time comes from the Wayland `ext-idle-notify-v1` protocol (own connection on a background thread; respects idle inhibitors, so
playing a video is not "away"). After the configured number of idle minutes an *Online* user is shown as Away; Busy, Away and
Appear offline are never changed automatically, a manual choice cancels automatic Away, and activity restores Online.
Compositors without the protocol (X11 sessions, older compositors): the setting is greyed out and automatic Away is off.
`MSM_AWAY_SECONDS` overrides the timeout for testing.

## Encryption, recovery and verification
Rooms created by this client are encrypted. A new device cannot read messages sent before it existed unless the user sets up
recovery (secret storage + key backup) and enters the recovery key there; verified on Synapse end to end. Setting recovery up first
bootstraps cross-signing, which the server protects with a password challenge (sent only to the homeserver). The recovery key is shown
once and never stored by the app. Your own device always counts as locally trusted, so "verified" here means cross-signed.
Interactive verification of other devices/users is not implemented yet.

## Reactions and edits
Reactions are standard `m.reaction` annotations (removing one redacts the reaction event); edits are `m.replace` relations.
Per spec an edit is honoured only if it comes from the original message's sender; the newest edit wins regardless of arrival order.
Verified between two accounts on Synapse, including that another user's "edit" is ignored.

## Verification
Emoji (SAS v1) verification of users and our own devices uses the standard `m.key.verification.*` flow (in-room for users, to-device for own devices), so it interoperates with other clients. Not implemented: QR codes.

## Replies
Replies use the standard `m.relates_to` / `m.in_reply_to` relation, so other clients show them as replies. We do not add the legacy quoted-text fallback.

## Attachments
Files and images are sent as `m.file` / `m.image` (also received: audio, video as files) with size and MIME type declared. In encrypted rooms the SDK
encrypts the upload and decrypts downloads. Filenames from other users are untrusted and sanitised before being used as paths.
Limits: 50 MB upload, 100 MB declared download, 8 MB for inline previews.

## Avatars
Own picture is the standard Matrix profile `avatar_url` (uploaded media, so other clients see it). Other users' and rooms' avatars are fetched as
server-scaled thumbnails and decoded defensively. Nicknames are unrelated and stay local.

## Group rooms and invitations
Group rooms are joined rooms with no `m.direct` targets that are not spaces. New rooms are created private (`private_chat` preset)
and encrypted. Invitations show the inviter and room name; declining uses a normal leave. Room-specific display names come
from member state events and rank below local nicknames and above global profile names.
Verified on Synapse: create/invite/accept/decline/leave, history, encrypted messages between members.

## Interoperability check with Element Web
Tested with Element Web 1.12.32 against a local Synapse: DM creation/invite, encrypted text, replies, edits, reactions and redactions interoperate in both directions. Encrypted attachments (images and files, both directions) and our profile picture also interoperate. Verification and presence have not been checked against Element.

## Friend requests
- Carried as the custom to-device event type `com.matrixmessenger.friend.v1` with content `{"action": "request" | "accept" | "reject" | "remove"}`. It is not a room invitation or message, so no room appears for third-party users; clients that don't know the type ignore it.
- Friendship is only recorded as mutual after a verifiable acknowledgement. Third-party clients never answer, so their users stay plain contacts (`Outgoing` until they answer, which they cannot).
- To-device events reach only the devices the recipient has when sent.

## Room moderation
Kick, ban, unban and room name/topic use the standard Matrix membership and state events. The UI offers an action only when the room's power levels permit it; the homeserver is the authority. Roles (Member 0, Moderator 50, Admin 100) are changed by updating `m.room.power_levels` users.

## Not yet implemented
Custom power-level values, room avatars and join rules, QR-code verification, recovery reset/disable, audio/video playback, upload progress, server-side message search.
