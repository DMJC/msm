# Matrix Messenger Development Instructions

Read MASTER_PROMPT.md before beginning development.

This project is a Rust/GTK3 Matrix messenger inspired by MSN Messenger.

## Mandatory rules
- GTK3 only. Never migrate to GTK4.
- Use Rust and matrix-rust-sdk.
- Preserve Matrix interoperability.
- Compile and test after meaningful changes.
- Never claim a feature is complete without verification.
- Do not replace working implementations unnecessarily.

## Autonomous development
1. Read MASTER_PROMPT.md.
2. Read PROGRESS.md and TODO.md if present.
3. Inspect the repository and current implementation.
4. Select the next incomplete requirement.
5. Implement, compile, debug, and test.
6. Update PROGRESS.md and TODO.md.
7. Commit verified changes.
8. Continue with the next task.

Do not ask for confirmation between ordinary development tasks.

## Session recovery
Before ending any session, record:
- Completed features
- Current implementation state
- Known problems
- Test results
- Exact next task

On a new session, resume from these records rather
than restarting the project analysis.
