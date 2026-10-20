# Security & Quality Review: Features 46-49 (modular configuration, macOS TUI, Shortcuts, Zellij)

- **Date**: 2026-10-20
- **Auditors**: `rev` and `hacker`
- **Target specs**: `docs/features/46-modular-configuration-and-power-settings.md`, `47-macos-desktop-architecture-and-ui.md`, `48-apple-shortcuts-and-app-intents.md`, `49-zellij-multiplexer-support.md`
- **Scope**: design review of specs only; there is no code yet.
- **Gate status**: **Design CLEAR after corrections** (the first version of this file said CLEAR before the spec text was fixed; it was re-reviewed the same day and the findings below were applied to the specs).

## Findings

| ID | Severity | Spec | Finding | Fix (now in the spec) |
| :--- | :---: | :---: | :--- | :--- |
| F-01 | High | 46 | Imported settings set a socket path, upload directory, prompt template and Ollama endpoint. | Hostile-input model: key allow-list, ranges rejected (not clamped), no free paths, https/loopback endpoints, fixed template placeholders. |
| F-02 | High | 46 | `..` rejection alone misses absolute paths (`/etc/shadow`) and symlinks. | No free path settings; where unavoidable: relative or under home, symlinks resolved and re-checked. |
| F-03 | High | 46 | Approval wait 60-600 s conflicts with the 130 s hook timeout and the implemented 5-120 s. | The existing `approval_wait_secs` (5-120) is used. |
| F-04 | Medium | 46 | SHA-256 of a local file protects nothing against an editor. | Called an integrity check (damage only). |
| F-05 | Medium | 46 | Import could loosen security switches. | Those keys cannot be loosened by import. |
| F-06 | Medium | 46 | A background grace period contradicts Zero Battery Drain. | Removed. |
| F-07 | High | 47 | "Same key" implied the phone's key on the Mac. | The Mac has its own key, registered through pairing (feature 44). |
| F-08 | Medium | 47 | Panic hook did not restore the cursor. | `Show` added; drop guard for early returns. |
| F-09 | Low | 47 | OSC 52 and title sequences from agent text. | `strip_ansi_codes` plus a test that no `0x1b` reaches the backend. |
| F-10 | High | 48 | Intents could carry a command; voice approval showed no command. | Saved items only; no approve intent. |
| F-11 | Medium | 48 | `isDestructive` and `authenticationPolicy` were conflated; `quote_argument_for_shell` does not exist. | Each intent lists its real protection; `tmux::shell_quote` for the prompt. |
| F-12 | Medium | 48 | Background intent has about 30 s. | "Not started" result instead of silent failure. |
| F-13 | High | 49 | Zellij lacks the screen-dump and key-injection set the feed depends on. | Scope: attach and detach only. |
| F-14 | Low | 49 | Session names that differ could collapse after sanitising. | Names are rejected, not rewritten. |
| F-15 | Low | 49 | A Zellij test could touch the developer's own sessions. | Private socket directory, `ZELLIJ*` variables removed. |

Numbering note: the first version of this file reused `VULN-02` and `VULN-03` twice; the table above replaces it.

## Open until code exists

Everything above is a design statement. The hacker gate must be repeated on the code of each feature.
