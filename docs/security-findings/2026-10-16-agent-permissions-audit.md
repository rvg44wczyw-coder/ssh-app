# Security audit: Feature 14 (agent permissions & hook settings)

Scope: `crates/core/src/permissions.rs`, `agents.rs` (`approval_matcher`, `approval_events`), `session.rs` (`read/preview/apply_agent_permissions`), `PermissionsEditorSheetView.swift`.

## Checks and results

| Attack | Result |
|---|---|
| Shell metacharacters in a rule specifier (`'`, `$(...)`, backticks, `"; cmd #`) | Stored verbatim through the quoted writer, read back identical, no command run (loopback test). |
| Rule with newline, NUL or other control characters | Rejected on write (error names the entry, shortened and made display-safe). |
| Hostile approval matcher (`*'; touch ... #`, spaces, newline, >200 chars) | Rejected before anything is written; no file created (unit + loopback test). |
| Event name outside the preset (`Bogus`, `PermissionRequest` on notify-only, newline) | Rejected. |
| Unparsable, duplicate-key or wrongly typed `settings.json` | Read, preview and apply all refuse; file stays byte-identical, no backup written. |
| Secrets in `settings.json` (`env`, `apiKeyHelper`) reaching the screen | Preview and read expose only the `permissions` object. |
| Symlinked `settings.json` | Written through, link preserved. |
| Wrong account edited | Per-profile config dir; path shown; orange warning for the default dir; independent dirs verified. |
| Unreadable file read as empty, then overwritten | The one-time backup `cp` fails under `set -e`, so the write aborts. |
| Mis-tap on broad rules or `bypassPermissions` | Warnings in the preview, confirmation dialog for `bypassPermissions`, explicit Apply. |
| Unbounded input | At most 200 rules per list, 400 characters per rule. |

## Findings

None open. Residual, named tradeoffs (unchanged from Feature 14 doc): the read-to-write lost-update window (same as hooks); a file with one malformed rule cannot be edited from the phone and must be fixed on the host.

CHAIN_STEP step=hacker result=clean
