# MASTER DEVELOPMENT PROMPT

## Rust / GTK3 Matrix Messenger — Autonomous Implementation

### 1. Project objective

Develop a complete, production-quality desktop instant messaging application inspired by Microsoft MSN Messenger and Windows Live Messenger, using **Rust, gtk-rs (GTK3), and the Matrix communication protocol**.

The application must provide the familiar MSN Messenger experience, including contact lists, customizable nicknames, online presence, instant messaging, group conversations, notifications, and contact organization.

Unlike the original MSN Messenger, the application must communicate with standard Matrix homeservers and remain interoperable with existing Matrix clients, including Element and FluffyChat.

The application must run natively on Linux, including Wayland and X11 environments.

**This is an implementation task, not merely a planning, documentation, or prototype task.**

Continue developing, compiling, testing, fixing, and improving the application until every mandatory requirement has been implemented and verified, or until a genuine external blocker prevents further progress.

Do not stop after producing an architecture document, creating an initial skeleton, or implementing only the basic messaging functionality.

---

# 2. Mandatory technology requirements

Use the following technology stack:

| Component            | Technology              |
| -------------------- | ----------------------- |
| Programming language | Rust                    |
| GUI toolkit          | GTK3 via gtk-rs         |
| GLib integration     | glib / gio              |
| Matrix protocol      | matrix-rust-sdk         |
| Async runtime        | Tokio                   |
| Local database       | SQLite                  |
| Encryption           | Matrix SDK E2EE support |
| Build system         | Cargo                   |
| Target OS            | Linux                   |
| Display servers      | Wayland and X11         |

### Restrictions

- Do not migrate the application to GTK4.
- Do not use Electron, Tauri, webviews, or a browser-based interface.
- Do not substitute another GUI toolkit.
- Do not implement a proprietary replacement for the Matrix messaging protocol.
- Prefer established Rust libraries over unnecessary custom implementations.
- Avoid deprecated APIs when supported alternatives exist.
- Verify dependency compatibility before selecting versions.
- Keep GTK operations on the GTK main thread.
- Keep network and database operations asynchronous or off the UI thread.
- Do not block the GTK event loop with synchronous Matrix operations.

GTK3 support is mandatory even if newer libraries primarily target GTK4.

Use a compatible, reproducible dependency set. Verify actual APIs against the installed versions and current documentation rather than assuming APIs from other releases.

---

# 3. Core identity and nickname model

The application must support three distinct identity layers.

### 3.1 Matrix identity

Every user has a Matrix user ID (MXID), for example:

`@dmjc:example.com`

This is the underlying protocol identity.

It must be used for routing, authentication, membership, and identifying message senders.

Do not treat display names as unique identifiers.

### 3.2 Public display name

Users may select any public display name, for example:

- DMJC
- Darkmage
- James
- Nightwalker

The public display name is stored using the standard Matrix profile API.

Other Matrix clients should see this name, subject to their own display rules.

Duplicate public display names must be permitted.

Users must be able to change their display name without changing their MXID.

### 3.3 Private contact nickname

Each user may privately assign a nickname to another Matrix user.

Example:

Matrix user: `@dmjc:example.com`

Public display name: `Darkmage`

Alice's nickname: `James Carthew`

Bob's nickname: `DMJC`

Alice must see `James Carthew` everywhere her client displays this user.

Bob must see `DMJC`.

Other Matrix clients may continue to display `Darkmage`.

A private nickname must never modify the target user's Matrix profile or shared room membership state.

### 3.4 Global nickname resolution

Implement a centralized name-resolution service with the following priority:

1. Locally assigned contact nickname.
2. Appropriate Matrix room-specific display name, where applicable.
3. Matrix global profile display name.
4. Matrix user ID.

All relevant application components must use this resolver:

- Contact list
- Direct message windows
- Group conversations
- Message sender labels
- Conversation titles
- Desktop notifications
- Search results
- User profiles
- Mentions and autocomplete display
- Conversation history

Names must update immediately when contact nicknames change.

Do not duplicate name-resolution logic throughout the UI.

---

# 4. Matrix interoperability

The application must function as a standards-compatible Matrix client.

Implement standard Matrix mechanisms for:

- Homeserver discovery and authentication.
- User profiles and avatars.
- Room creation and invitations.
- Direct messaging.
- Group messaging.
- Room membership.
- Message synchronization.
- Message editing and redaction.
- Reactions.
- Read receipts.
- Typing notifications.
- Presence where supported.
- Media attachments.
- End-to-end encryption.
- Device management and verification.
- Logout and session restoration.

Use standard Matrix events whenever possible.

Custom application events or account data must not interfere with other Matrix clients.

Unknown custom events must be safe for other Matrix clients to ignore.

Do not assume that Matrix clients implement the application's private contact-list extensions.

### 4.1 Direct message interoperability

Use the standard `m.direct` account-data mechanism to identify direct message rooms.

Support direct messages created by other Matrix clients.

Do not assume:

- A user has only one DM room.
- A DM room always has exactly two current members.
- Every room with two participants is necessarily a DM.
- Every contact already has an existing DM room.

Support opening an existing conversation or creating a new DM room when necessary.

### 4.2 Room interoperability

Users must be able to:

- Receive invitations from other Matrix clients.
- Accept and reject invitations.
- Participate in rooms created by Element or other clients.
- View and send standard Matrix messages.
- Receive encrypted messages when their session and keys permit.
- Leave rooms.
- Handle room membership changes.
- Handle unsupported events gracefully.

Never rewrite Matrix event sender identities to implement local nicknames.

---

# 5. MSN Messenger-style contacts

Contacts must be treated as a separate application concept from Matrix rooms.

A user must be able to add another Matrix user to their contacts without immediately creating a chat room.

### 5.1 Contact features

Implement:

- Add contact by Matrix ID.
- Remove contact.
- Rename contact privately.
- Reset nickname to public display name.
- Organize contacts into groups.
- Move contacts between groups.
- Favourite contacts.
- Block and unblock users using standard Matrix ignored-user functionality.
- Search contacts.
- Show contact avatars.
- Show presence when available.
- Display custom status messages where supported.
- Open conversations from the contact list.

### 5.2 Contact groups

Provide default groups:

- Friends
- Family
- Work
- Other

Allow users to create, rename, delete, and reorder groups.

Contacts may be assigned to custom groups.

Group names and assignments must synchronize across the user's own application installations.

### 5.3 Friends and approval

Design an application-specific friend-request mechanism without confusing it with Matrix room invitations.

Support:

- Outgoing friend requests.
- Incoming friend requests.
- Accepting requests.
- Rejecting requests.
- Removing friendships.
- Pending request status.

The design must acknowledge that third-party Matrix clients may not understand the application's friendship protocol.

Adding a Matrix user as a local contact must remain possible independently of mutual friendship approval.

Do not claim that a friendship has been mutually accepted without verifiable acknowledgement.

Avoid requiring a proprietary homeserver modification for standard messaging.

---

# 6. Contact synchronization

Use Matrix private account data to synchronize contact information between devices.

Use a custom namespaced account-data type, for example:

`com.matrixmessenger.contacts.v1`

The namespace must be configurable or consistently defined throughout the project.

Design a versioned data schema.

Example:

```json
{
  "schema_version": 1,
  "contacts": {
    "@dmjc:example.com": {
      "nickname": "James Carthew",
      "group": "Friends",
      "favorite": true
    }
  }
}
```

The implementation must support:

- Initial synchronization.
- Offline cached contacts.
- Updating individual contacts.
- Deleting contacts.
- Concurrent changes on multiple devices.
- Schema migration.
- Data validation.
- Recovery from malformed account data.

Matrix account-data updates replace the content of an event type and do not provide automatic field-level merge semantics.

Therefore, implement an explicit conflict-resolution strategy.

Evaluate per-contact account-data events, versioned records, local SQLite caching, and merge strategies.

Do not assume that separate event types automatically provide transactional consistency or conflict-free updates.

Private account data is not automatically end-to-end encrypted. Document this and implement appropriate privacy protections for stored personal contact information.

---

# 7. Presence and status

Implement an MSN-style presence system using Matrix capabilities.

Supported UI states:

- Online
- Away
- Busy / Do Not Disturb
- Offline
- Unknown / unavailable

Matrix presence does not necessarily support all these states directly.

Implement a documented mapping between application states and standard Matrix presence values.

Use `m.presence` where supported.

Implement a configurable inactivity timer for automatic Away status.

Support optional user-entered status messages.

Do not represent unavailable presence information as definitively offline.

Do not assume a user can subscribe to arbitrary Matrix accounts' presence.

The application must degrade gracefully when a homeserver disables or restricts presence.

---

# 8. GTK3 user interface

Create a polished native GTK3 interface inspired by MSN Messenger while remaining appropriate for modern Linux desktops.

Use gtk-rs GTK3 widgets, GtkBuilder resources, CSS styling, and GLib/GIO where appropriate.

Avoid hardcoded pixel positioning where responsive GTK layouts are more suitable.

### 8.1 Main contact window

The primary window must include:

- Current user's avatar.
- Public display name.
- Matrix ID in profile details.
- Presence selector.
- Personal status message.
- Search contacts field.
- Expandable contact groups.
- Online and offline contact indicators.
- Contact context menus.
- Unread conversation indicators.
- Notifications for pending invitations.
- Application settings access.

The contact list should resemble the organization and usability of MSN Messenger.

### 8.2 Conversation windows

Support both direct messages and group conversations.

Provide:

- Conversation header.
- Participant avatar and resolved name.
- Scrollable message history.
- Message composer.
- Send button.
- Keyboard shortcuts.
- Timestamps.
- Message delivery and failure states.
- Typing indicators.
- Read receipts.
- File attachments.
- Emoji selection.
- Reactions.
- Message editing.
- Message deletion/redaction.
- Search within conversation.
- Unread indicators.

Support multiple conversations through tabs or separate windows, with a configurable preference if practical.

### 8.3 User profiles

Provide a profile dialog showing:

- Local nickname.
- Public Matrix display name.
- Matrix ID.
- Avatar.
- Presence.
- Status message.
- Shared rooms where available.
- Actions for renaming, blocking, and starting conversations.

Ensure that users can inspect the underlying Matrix ID to distinguish accounts with identical display names.

### 8.4 Settings

Implement a settings window containing:

- Account information.
- Public display name editing.
- Avatar editing.
- Presence preferences.
- Notifications.
- Startup behaviour.
- Conversation appearance.
- Contact list appearance.
- Security and encryption settings.
- Session and device management.
- Homeserver information.

Persist settings appropriately.

---

# 9. Messaging functionality

Implement reliable text messaging with Matrix synchronization.

Required functionality:

- Send and receive messages.
- Display historical messages.
- Paginate older messages.
- Handle messages arriving while the application is open.
- Recover missed messages after reconnecting.
- Handle message ordering.
- Handle failed sends.
- Retry recoverable failures.
- Avoid duplicate messages after retry.
- Show pending, sent, and failed states.
- Support message edits and redactions.
- Support replies and reactions.
- Support Unicode and multiline text.
- Preserve Matrix event IDs and relationships.

The UI must remain responsive during network delays.

Use appropriate Matrix SDK facilities for synchronization, local persistence, and event handling.

Avoid implementing a second independent Matrix synchronization engine unnecessarily.

---

# 10. End-to-end encryption

Implement Matrix end-to-end encryption using supported matrix-rust-sdk facilities.

Mandatory functionality:

- Encrypted room messaging.
- Secure local crypto-store persistence.
- Device verification workflows.
- Cross-signing and key recovery where supported.
- Handling undecryptable messages.
- Displaying encryption status.
- Session restoration without unnecessary loss of encryption keys.

Never silently downgrade an encrypted conversation to plaintext.

Never log access tokens, private keys, recovery keys, or decrypted message contents.

Do not invent custom cryptographic algorithms.

Use the Matrix SDK's supported encryption implementation.

---

# 11. Database and persistence

Use SQLite for application-specific local state where necessary.

Potential tables include:

```text
contacts
contact_groups
contact_preferences
application_settings
notification_preferences
local_ui_state
schema_migrations
```

Avoid duplicating message storage already reliably maintained by the Matrix SDK unless required.

Use database migrations.

Support clean startup, shutdown, and recovery after interruption.

Implement a single-writer or otherwise safe concurrency strategy.

Persist window state and user preferences.

Store access tokens and encryption secrets in a suitable secure credential store when available, with a documented fallback policy.

Never store credentials in plaintext application configuration files.

---

# 12. Rust application architecture

Organize the code into independent modules.

Suggested structure:

```text
src/
    main.rs
    app.rs

    matrix/
        mod.rs
        client.rs
        authentication.rs
        synchronization.rs
        rooms.rs
        messages.rs
        presence.rs
        encryption.rs
        media.rs

    contacts/
        mod.rs
        contact.rs
        contact_manager.rs
        nickname_resolver.rs
        groups.rs
        friendship.rs
        synchronization.rs

    ui/
        mod.rs
        main_window.rs
        contact_list.rs
        contact_row.rs
        conversation_window.rs
        message_row.rs
        profile_dialog.rs
        settings_dialog.rs
        login_window.rs

    storage/
        mod.rs
        database.rs
        migrations.rs
        settings.rs

    services/
        mod.rs
        notifications.rsonly problem
        avatar_cache.rs
        event_dispatch.rs

resources/
    ui/
    css/
    icons/

tests/
```

This structure is a starting point, not an inflexible requirement.

If the repository already has an architecture, adapt it rather than unnecessarily replacing working code.

### 12.1 GTK and asynchronous integration

GTK3 operations must execute on the GTK main thread.

Matrix SDK and network operations must run asynchronously without blocking the UI.

Use safe message passing between async tasks and GTK components.

Use compatible GLib main-context mechanisms, channels, or other supported dispatch facilities.

Do not access GTK widgets directly from background Tokio tasks.

### 12.2 State management

Keep Matrix protocol state, contact state, and UI state separate.

Use clearly defined models and events.

Prevent multiple UI components from maintaining conflicting copies of the same contact nickname or presence state.

Use stable Matrix IDs and room IDs internally.

---

# 13. Error handling and resilience

Handle:

- Invalid homeserver addresses.
- Authentication failures.
- Network disconnections.
- Server timeouts.
- Rate limiting.
- Invalid Matrix events.
- Missing room permissions.
- Media download failures.
- Encrypted message failures.
- Database errors.
- Invalid local configuration.
- Session expiration.
- Concurrent account-data updates.

Show understandable error messages.

Log diagnostic information without exposing secrets.

Retry transient errors with appropriate backoff.

Do not enter uncontrolled retry loops.

The application must recover from ordinary network interruptions without requiring a restart.

---

# 14. Testing requirements

Testing is mandatory.

Implement automated tests for:

### Identity and contacts

- Nickname priority.
- Missing display names.
- Duplicate display names.
- Unicode nicknames.
- Contact creation and deletion.
- Contact group management.
- Contact synchronization.
- Account-data conflict handling.
- Local nickname isolation.

### Messaging

- Message event parsing.
- Room mapping.
- Direct message identification.
- Duplicate event handling.
- Message editing.
- Message redaction.
- Failed send recovery.
- Offline synchronization.

### Persistence

- Database migrations.
- Contact serialization.
- Corrupted configuration recovery.
- Restart persistence.

### Integration

Where practical, test against a real local Matrix homeserver.

Verify:

1. Your client sends a message.
2. Element receives the message.
3. Element replies.
4. Your client receives the reply.
5. Your client applies its private contact nickname.
6. Element continues displaying the standard Matrix identity.
7. Restarting your client preserves the nickname.
8. A second installation of your client synchronizes the nickname.

Include encrypted-room interoperability testing.

Do not claim interoperability has been verified unless the test was actually performed.

---

# 15. Development phases

Implement the project in the following phases.

## Phase 1 — Repository inspection and architecture

- Inspect all existing source files.
- Determine what is already implemented.
- Identify the current dependency versions.
- Verify GTK3 and Matrix SDK compatibility.
- Create an implementation checklist.
- Identify architectural deficiencies.
- Preserve working functionality.

Deliverable: documented architecture and buildable foundation.

## Phase 2 — Application framework

- Initialize GTK3 application.
- Implement main window.
- Establish async runtime integration.
- Add logging and configuration.
- Establish persistent storage.
- Implement clean startup and shutdown.

Deliverable: functioning native GTK3 application.

## Phase 3 — Matrix authentication

- Homeserver configuration.
- Login.
- Session restoration.
- Logout.
- Basic user profile.
- Matrix synchronization.

Deliverable: client can authenticate and maintain a Matrix session.

## Phase 4 — Contact system

- Contact data model.
- Contact list.
- Custom nicknames.
- Nickname resolver.
- Contact groups.
- Contact persistence.
- Matrix account-data synchronization.

Deliverable: working MSN-style contact management.

## Phase 5 — Direct messaging

- DM discovery and creation.
- Message sending.
- Message receiving.
- Message history.
- Conversation UI.
- Notifications.
- Typing indicators.

Deliverable: interoperable direct messaging.

## Phase 6 — Group messaging

- Room invitations.
- Group conversation windows.
- Membership management.
- Group message history.
- Participant nickname resolution.
- Room naming.

Deliverable: interoperable Matrix group conversations.

## Phase 7 — Presence and friendship

- Matrix presence integration.
- Status selector.
- Automatic Away.
- Presence privacy handling.
- Application-specific friend requests.
- Pending and accepted friendship state.

Deliverable: MSN-style contact and presence experience, with documented Matrix limitations.

## Phase 8 — Encryption and security

- End-to-end encryption.
- Device verification.
- Secure session persistence.
- Encryption status indicators.
- Error handling.

Deliverable: secure encrypted Matrix conversations.

## Phase 9 — Advanced messaging

- File transfers.
- Avatars.
- Reactions.
- Message editing.
- Redaction.
- Search.
- Read receipts.
- Improved notifications.

Deliverable: feature-complete messaging experience.

## Phase 10 — Quality and release

- UI refinement.
- Performance testing.
- Memory and resource cleanup.
- Network recovery testing.
- Database migration testing.
- Automated integration tests.
- Documentation.
- Release build.
- Linux installation packaging.

Deliverable: usable Linux desktop application.

---

# 16. Autonomous development procedure

You are operating as an autonomous software development agent.

Your responsibility is to make continuous, verifiable progress toward the completed application.

Follow this cycle:

1. Inspect the current repository and progress records.
2. Identify the highest-priority incomplete requirement.
3. Examine relevant code and documentation.
4. Implement a coherent unit of functionality.
5. Run formatting and compilation checks.
6. Run relevant tests.
7. Investigate failures.
8. Fix identified problems.
9. Re-run affected tests.
10. Update progress documentation.
11. Commit verified changes when appropriate.
12. Select the next incomplete requirement.
13. Repeat until the completion criteria are satisfied.

**Do not stop merely because one phase or feature has been implemented.**

Do not repeatedly ask the user whether to continue.

Proceed automatically whenever the next action is reasonably determined by the project requirements.

Do not interpret a successful `cargo build` as proof that the application is complete.

### 16.1 Failure recovery

If compilation fails:

- Read the actual compiler diagnostics.
- Identify the root cause.
- Make the smallest appropriate correction.
- Recompile.
- Repeat until fixed or genuinely blocked.

If tests fail:

- Determine whether the implementation or test expectation is incorrect.
- Fix the underlying problem.
- Do not simply delete or disable failing tests.
- Re-run the relevant tests.

If an API is unavailable:

- Inspect the installed dependency version.
- Consult available source or documentation.
- Identify a supported alternative.
- Update the implementation appropriately.

Do not invent APIs or repeatedly guess at function signatures.

### 16.2 Avoid unproductive loops

Detect repeated failures and lack of progress.

After several unsuccessful attempts on the same issue:

- Summarize the failed approaches.
- Re-examine assumptions.
- Investigate alternative solutions.
- Record the blocker.
- Continue with an independent task if possible.

Never claim a problem is solved without verification.

### 16.3 Tool and session limitations

Continue within the available execution session and tool permissions.

If execution is interrupted or the context window is exhausted, preserve sufficient progress information for the next agent session.

Do not claim to be running continuously in the background unless an actual external orchestration system is configured.

Never fabricate command output, test results, file modifications, or Git commits.

---

# 17. Persistent project documentation

Maintain the following files in the repository:

```text
docs/
    ARCHITECTURE.md
    REQUIREMENTS.md
    MATRIX_COMPATIBILITY.md
    SECURITY.md
    TESTING.md
    IMPLEMENTATION_STATUS.md

AGENTS.md
PROGRESS.md
TODO.md
```

### PROGRESS.md

Update after every meaningful implementation cycle.

Include:

- Current phase.
- Completed work.
- Files modified.
- Tests executed.
- Results.
- Known issues.
- Next actions.
- Blockers.

### TODO.md

Maintain an actionable task list.

Use:

- [ ] Not started
- [x] Completed and verified

Do not mark tasks complete merely because partial code exists.

### AGENTS.md

Document:

- Build commands.
- Test commands.
- Project structure.
- Architectural rules.
- Coding conventions.
- Important dependencies.
- Known limitations.
- Instructions for resuming work.

The documentation must allow a new LLM session to resume development without repeating the entire analysis.

---

# 18. Code quality requirements

Use idiomatic Rust.

Follow these principles:

- Prefer strong types over stringly typed interfaces.
- Avoid unnecessary `unwrap()` and `expect()` calls in production paths.
- Use structured error types and meaningful error propagation.
- Minimize global mutable state.
- Keep modules focused.
- Avoid excessive abstraction.
- Avoid duplicated logic.
- Document non-obvious design decisions.
- Use clear ownership and concurrency boundaries.
- Avoid blocking GTK's main loop.
- Use Cargo formatting and linting tools.

Regularly run:

```bash
cargo fmt --all -- --check
cargo check --all-targets
cargo clippy --all-targets
cargo test
```

Use the appropriate project-specific feature flags and system dependencies.

Address meaningful warnings rather than suppressing them without justification.

---

# 19. Completion criteria

The project is complete only when all mandatory features have been implemented and the available tests have passed.

At minimum, the application must:

1. Build successfully on the supported Linux environment.
2. Start as a native GTK3 desktop application.
3. Log into a standard Matrix homeserver.
4. Restore an existing session.
5. Display the user's profile and contact list.
6. Add, remove, rename, and organize contacts.
7. Synchronize private nicknames between installations.
8. Display locally assigned nicknames throughout the interface.
9. Send and receive Matrix direct messages.
10. Send and receive group messages.
11. Interoperate with standard Matrix clients.
12. Support encrypted conversations.
13. Display presence correctly when available.
14. Recover from network interruptions.
15. Persist application state.
16. Handle ordinary errors without crashing.
17. Pass the automated test suite.
18. Include installation and usage documentation.

Also verify that assigning a nickname to another user does not change that user's public Matrix profile or any shared room state.

### Final acceptance scenario

Create or use three Matrix accounts:

- Alice
- Bob
- Darkmage

Alice assigns Darkmage the nickname `James Carthew`.

Bob assigns Darkmage the nickname `DMJC`.

Verify:

- Alice sees James Carthew.
- Bob sees DMJC.
- Element sees Darkmage, unless separately customized there.
- All three identities refer to the same Matrix user ID.
- Messages work between your client and Element.
- Group conversations display Alice's and Bob's independent nicknames.
- Nicknames persist after restart.
- Nicknames synchronize between installations of your application.
- Encrypted messages work across compatible clients.

This scenario is a mandatory acceptance test.

---

# 20. Initial execution instructions

Begin immediately.

First inspect the current working directory and Git repository.

Determine whether an application already exists.

If it exists, audit its implementation and continue from its current state.

If no application exists, initialize a new Rust Cargo project targeting GTK3 and establish the initial architecture.

Before extensive implementation, verify the installed Rust toolchain, GTK3 development libraries, and compatibility of the selected Matrix SDK version.

Create the project documentation and implementation checklist.

Then begin implementing Phase 1 and proceed through subsequent phases.

After each implementation cycle, update PROGRESS.md and TODO.md.

Continue autonomously until the completion criteria have been satisfied or an unavoidable external dependency, missing credential, unavailable environment, or required user decision blocks further progress.

When blocked, record the exact blocker and complete any remaining independent tasks.

**Your goal is to deliver a fully functional, interoperable, MSN Messenger-style Matrix client written in Rust using GTK3, not merely to produce a design or proof of concept.**
