# Session Notes

- Diesel replaces the original sqlx-based persistence plan, so any future docs or scaffolding helpers should avoid reintroducing sqlx assumptions.
- The current implementation streams Anthropic chunks through Redis-backed accumulation for persistence, but it does not yet support process-resume for an in-flight stream after a server restart.
- `COOKIE_SECRET` is required by configuration but not yet used to sign or encrypt the anonymous session cookie, so session tamper-resistance still needs a follow-up pass.
- The browser runtime now uses server-served HTML/JS for `cargo run` ergonomics, so `crates/app` is currently not part of the live request path and should either be reintegrated or removed in a future cleanup.
- Document grounding is mocked with a static SharePoint-like list and only injects selected document excerpts into the final user turn; a later retrieval pass should make selection and context assembly conversation-aware.
- Document authorization is still a server-local mock allowlist keyed by email, now preferably loaded from `DOCUMENT_ACCESS_RULES_FILE`, so access can drift from real Google Workspace groups or SharePoint ACLs until a synced source of truth is added.
- SharePoint Graph sync currently extracts snippets only from plaintext-like files; Office and PDF documents still need a richer content extraction path before they can ground prompts with real excerpts.
