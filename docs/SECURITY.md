# Security notes

- **End-to-end encryption** is the SDK's Olm/Megolm implementation. Rooms with encryption show a lock; each message shows whether the sending device is verified.
- **Secrets**: the session access token and store passphrase go to the freedesktop Secret Service via `oo7` (encrypted file keyring fallback). If neither opens, nothing is persisted and the user logs in again next start; credentials are never written to plaintext config. Secrets, tokens and recovery keys are never logged.
- **Verification**: emoji (SAS) verification with cross-signing; a verified person shows ✔. Interop checked with Element Web in both directions. QR verification is not implemented.
- **Recovery**: set up, enter, reset and turn off the recovery key from the Security dialog. Destructive actions ask for confirmation with Cancel as the default.
- **Nicknames are not E2EE**: they sit in unencrypted account data and are visible to the homeserver operator. They are never published to other users.
- **Blocking** uses Matrix's `m.ignored_user_list`, so it applies account-wide.
- **Invites to encrypted rooms**: the SDK shares room history on invite and refuses when this device is not cross-signed; msm then sends a plain invite (the invitee does not receive earlier history keys).
- **Attachments** are capped (5 MB for pictures) and temporary files from pasted images are deleted after upload.
