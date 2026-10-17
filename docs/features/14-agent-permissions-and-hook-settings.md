# Feature 14: Configurable Agent Permissions & Hook Settings

- **Slug**: `14-agent-permissions-and-hook-settings`
- **Milestone**: M18
- **Status**: Implemented (M18)
- **Participating roles**: `doc-dev`, `rev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Hacker gate**: yes (writes the agent's permission policy on the host; parses untrusted host config)

## 1. Purpose

Two kinds of agent settings are today either fixed or not editable from the phone:

1. **Hook settings** – the phone-approval preset always matches every tool (`*`) and always installs `PermissionRequest` + `Notification` + `Stop`. The user wants to choose *which tools reach the phone* and *which events are installed*, per profile (e.g. only personal Claude, only `Bash|Edit|Write`).
2. **Permission rules** – Claude Code's own allow / ask / deny lists and default mode live in `settings.json` of the profile's config directory and can only be edited on the laptop. The user wants to view and edit them per profile from the app, with templates, validation and a preview before anything is written.

Scope is **per profile**: every change goes into that profile's config directory (`CLAUDE_CONFIG_DIR`), so a personal and a work Claude stay independent.

## 2. Non-goals

- Project-level (`.claude/settings.json`, `settings.local.json`) and enterprise-managed settings (read-only for us, not touched).
- Antigravity / Codex / Grok permissions (schemas not verified; they return an `Unsupported` error). The hook settings of Antigravity keep using presets.
- Editing arbitrary settings (env, model, `apiKeyHelper`, …). Only `permissions.allow|ask|deny|defaultMode` is read or written; every other key is preserved byte-for-structure and never displayed.

## 3. Domain model (Rust, UniFFI)

```rust
#[derive(uniffi::Record)] pub struct AgentPermissions {
    pub allow: Vec<String>,
    pub ask: Vec<String>,
    pub deny: Vec<String>,
    pub default_mode: Option<String>,   // None = leave the agent's default
}
#[derive(uniffi::Record)] pub struct PermissionTemplate {
    pub id: String, pub title: String, pub description: String,
    pub allow: Vec<String>, pub ask: Vec<String>, pub deny: Vec<String>,
}
// HookPreset gains (PhoneApproval / NotifyOnly only; ignored by Custom):
pub approval_matcher: String,        // tools that reach the phone; empty = "*"
pub approval_events: Vec<String>,    // empty = the preset's default events
```

Functions (all pure except the session methods):

| Function | Purpose |
|---|---|
| `permission_rule_templates() -> Vec<PermissionTemplate>` | built-in starting points (§5) |
| `validate_agent_permissions(AgentKind, AgentPermissions) -> Result<AgentPermissions, SshCoreError>` | normalises (trim, dedupe, stable order) and validates (§4) |
| `permission_warnings(AgentPermissions) -> Vec<String>` | non-blocking warnings (§4.3) |
| `permissions_from_settings(AgentKind, config_json) -> Result<AgentPermissions, SshCoreError>` | reads the four fields; unparsable JSON / duplicate keys / wrong types / an invalid rule (§4.1) → error |
| `merge_permissions(AgentKind, existing_json, AgentPermissions) -> Result<String, SshCoreError>` | returns the new settings text; touches only the four fields |
| `permissions_section_of(AgentKind, json) -> Result<String, SshCoreError>` | only the `permissions` object, for the preview |

Session (`SshSessionHandle`):

```rust
async fn read_agent_permissions(&self, agent, config_dir: Option<String>) -> Result<AgentPermissions, SshCoreError>;
async fn preview_agent_permissions(&self, agent, config_dir, new: AgentPermissions) -> Result<String, SshCoreError>;
async fn apply_agent_permissions(&self, agent, config_dir, new: AgentPermissions) -> Result<(), SshCoreError>;
```

`apply` validates, re-reads the file, merges and writes with the same guarded writer as hooks (symlink-aware, mode-preserving, one-time `.ssh-app.bak`, atomic rename). The config path comes from `agent_hooks_config_path` (validated, `settings.json` of the profile's config directory).

## 4. Behaviour

### 4.1 Rule syntax (Claude Code)

A rule is `Tool` or `Tool(specifier)`:
- `Tool` matches `^[A-Za-z][A-Za-z0-9_]{0,63}$` (built-in tools, and `mcp__server__tool` names).
- `(specifier)`: 1–300 characters, no control characters (including newline), must end with `)`; the specifier is opaque to us (Claude interprets `git push:*`, `domain:example.com`, `./.env`, …).
- Whole rule ≤ 400 characters; ≤ 200 rules per list.
- Invalid entries are **rejected with an error naming the entry**, never silently dropped.

### 4.2 Default mode

Allowed values: `default`, `acceptEdits`, `plan`, `bypassPermissions`. Anything else is rejected. `None` removes `defaultMode`.

### 4.3 Warnings (shown in the preview, never block)

- A rule present in two lists (deny wins; the other copy is dead).
- Over-broad allow rules: `Bash`, `Bash(*)`, `Bash(:*)`, `*`.
- `defaultMode = bypassPermissions` (agent stops asking; phone approvals no longer fire).
- An `allow` rule shadowed by a `deny` rule with the same text.

### 4.4 Merge rules

- Only `permissions.allow`, `permissions.ask`, `permissions.deny` and `permissions.defaultMode` are written. Other keys (`additionalDirectories`, unknown future keys, all non-permission settings) are preserved.
- An **empty list removes the key**; an empty `permissions` object is removed.
- Same refusals as hooks: unparsable JSON, duplicate keys at any level, a non-object root or `permissions` of the wrong type → error, nothing written.
- Output is pretty JSON (2-space indent), key order preserved (`serde_json` `preserve_order`).
- The preview shows **only** the resulting `permissions` section, never `env` / `apiKeyHelper`.

### 4.5 Hook settings (PhoneApproval / NotifyOnly)

- `approval_matcher`: tool matcher written to the `PermissionRequest` entry (Antigravity: `PreToolUse`). Validated: empty or `^[A-Za-z0-9_|*.:-]{1,200}$`.
- `approval_events`: subset of the preset's events (`PermissionRequest`, `Notification`, `Stop` for Claude; `PreToolUse`, `Stop` for Antigravity). Unknown events are rejected; an empty list means the default set; the result must contain at least one event.
- Both are stored in the saved profile on the phone and shown in the existing hook preview before install, exactly like the other presets.

### 4.6 Templates

| id | title | allow | ask | deny |
|---|---|---|---|---|
| `read-only` | Read-only tools without asking | `Read`, `Glob`, `Grep`, `Bash(git status:*)`, `Bash(git diff:*)`, `Bash(git log:*)`, `Bash(ls:*)` | | |
| `ask-destructive` | Always ask for destructive commands | | `Bash(rm:*)`, `Bash(git push:*)`, `Bash(git reset:*)`, `Bash(sudo:*)` | |
| `protect-secrets` | Never read secrets | | | `Read(./.env)`, `Read(./.env.*)`, `Read(~/.ssh/**)`, `Read(**/*.pem)`, `Read(**/*.key)` |
| `protect-agent-config` | Protect agent and hook settings | | | `Edit(~/.claude/settings.json)`, `Edit(~/.claude/settings.local.json)`, `Edit(./.claude/settings*.json)`, `Edit(~/.gemini/config/hooks.json)`, `Edit(~/.codex/hooks.json)`, `Edit(~/.grok/hooks/**)`, `Edit(~/.ssh-app/**)`; also part of `project-work` |

Applying a template **adds** its rules to the editor state (nothing is written until Apply).

## 5. iOS

- Profile row gets **Permissions** next to **Review & install** (Claude profiles only).
- `PermissionsEditorSheetView`: loads the current rules from the host; three sections (Allow / Ask / Deny) with add / delete, a default-mode picker (selecting `bypassPermissions` shows a confirmation), template buttons, the config path and – for a profile without a config directory – the same orange warning as the hook screen (it edits the default `~/.claude`).
- **Preview & apply**: calls `preview_agent_permissions`, shows warnings and the resulting `permissions` section, then **Apply** → `apply_agent_permissions`.
- Profile editor, section *Approval hooks*: matcher field and event toggles for PhoneApproval / NotifyOnly (stored with older profiles decoding to the defaults).
- Everything host-controlled is rendered with `Text(verbatim:)`.

## 6. Security

| Threat | Mitigation |
|---|---|
| Hostile or corrupt `settings.json` | Strict parse, duplicate-key and type checks; never partially applied; preview shows only `permissions`. |
| Rule text smuggling (newline, NUL, bidi) | Control characters are rejected on write. On read, a rule that is not valid under §4.1 makes the read fail with an error naming the entry and its list (the editor replaces whole lists, so loading it silently would lose or alter it on Apply); such a file must be fixed on the host. Named tradeoff: this blocks editing of a file with one malformed rule, in exchange for never losing rules unseen. |
| Over-broad allow or `bypassPermissions` set by a mis-tap | Warnings in the preview, a confirmation for `bypassPermissions`, explicit Apply step. |
| Editing the wrong account | Per-profile config directory; orange warning when it is the default; the path is shown before apply. |
| Command injection | Rules are data inside JSON written through the existing quoted writer; no shell interpolation of rule text. |
| Lost update (agent edits the file between read and write) | Same documented window as hooks (backlog item); the read happens immediately before the write. |
| Resource abuse | ≤ 200 rules per list, ≤ 400 characters each, file size capped by the exec output cap. |

## 7. Tests (policy: unit tests only for the critical logic; the rest integration + end to end)

- **Unit (critical logic)**: rule validation and normalisation, warnings, `merge_permissions` (preserves other keys, removes empty lists, refuses duplicate keys / bad types), hook matcher/event validation.
- **Integration (loopback sshd)**: read → preview → apply → read round trip on a profile config dir; other settings and a pre-existing user hook survive; unparsable file refused and left untouched; symlinked `settings.json` written through; two profile dirs stay independent; backup created once.
- **End to end**: install the PhoneApproval preset with a custom matcher + events through `install_agent_hooks_with_preset`, run the real hook for a matching tool and check the settings file contains exactly the planned entries.
- **Manual (user)**: edit personal-profile rules from the phone and confirm `~/.claude-personal/settings.json`, and that `~/.claude/settings.json` is untouched.

## 8. Out of scope / backlog

- Project-level settings, Antigravity/Codex/Grok permission editors, per-directory rule scopes, rule testing ("would this command be allowed?").

## Addendum: session permissions and invalid rules

The "Permissions" menu tile (per session: folder, detected config folder, start mode, suggestions) and the checked read that shows invalid rules escaped and drops them only after confirmation are described in feature 17 §4.
