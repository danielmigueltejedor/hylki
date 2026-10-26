# Agent instructions — hylki

## Repository-specific deterministic quality contract (2026-10-26)

**Project:** Rust + GTK Linux email client with IMAP/POP3/SMTP.

**Engineering requirements:** Never expose mail tokens, secret fields or raw message content. Preserve mail parsing, TLS validation and mailbox state transitions.

**Verification commands:** cargo fmt --all -- --check; cargo metadata --locked --no-deps --format-version 1; targeted cargo test.

Do not assert any test passed without actual output. Add tests for changed behavior, malformed input and regressions. Never remove/weaken tests, reduce thresholds, suppress errors, or change CI merely to produce a green result. Require review of `QUALITY_GATES.md`, workflow and contract edits. Make the `quality` check required via branch protection. Native tooling blockers must be reported explicitly; preserve existing agent instructions.
