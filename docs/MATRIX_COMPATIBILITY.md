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

## Group rooms and invitations
Group rooms are joined rooms with no `m.direct` targets that are not spaces. New rooms are created private (`private_chat` preset)
and encrypted. Invitations show the inviter and room name; declining uses a normal leave. Room-specific display names come
from member state events and rank below local nicknames and above global profile names.
Verified on Synapse: create/invite/accept/decline/leave, history, encrypted messages between members.

## Not yet implemented
Friend requests, device verification UI, media, edits sent from this client,
reactions, kick/ban and room settings.
