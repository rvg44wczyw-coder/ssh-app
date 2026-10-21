use super::*;

pub(super) fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// Files that decide what the agent may do and who is asked: its own settings, the hook configs
/// of every supported agent and the app's folder. An agent that can edit them can switch off the
/// phone approval, so they are denied for edits. (A shell command can still write them; the
/// `Bash` rules and the approval itself are the guard for that.)
pub(super) const AGENT_CONFIG_DENY: &[&str] = &[
    "Edit(~/.claude/settings.json)",
    "Edit(~/.claude/settings.local.json)",
    "Edit(./.claude/settings.json)",
    "Edit(./.claude/settings.local.json)",
    "Edit(~/.gemini/config/hooks.json)",
    "Edit(~/.codex/hooks.json)",
    "Edit(~/.grok/hooks/**)",
    "Edit(~/.ssh-app/**)",
];

/// Built-in starting points; applying one only adds its rules to the editor.
#[uniffi::export]
pub fn permission_rule_templates() -> Vec<PermissionTemplate> {
    vec![
        PermissionTemplate {
            id: "read-only".into(),
            title: "Read-only tools without asking".into(),
            description: "Reading files and basic git inspection never prompts.".into(),
            allow: strings(&[
                "Read",
                "Glob",
                "Grep",
                "Bash(git status:*)",
                "Bash(git diff:*)",
                "Bash(git log:*)",
                "Bash(ls:*)",
            ]),
            ask: vec![],
            deny: vec![],
            default_mode: None,
        },
        PermissionTemplate {
            id: "ask-destructive".into(),
            title: "Always ask for destructive commands".into(),
            description: "Deleting, pushing, resetting and sudo always need a decision.".into(),
            allow: vec![],
            ask: strings(&[
                "Bash(rm:*)",
                "Bash(git push:*)",
                "Bash(git reset:*)",
                "Bash(sudo:*)",
            ]),
            deny: vec![],
            default_mode: None,
        },
        PermissionTemplate {
            id: "protect-secrets".into(),
            title: "Never read secrets".into(),
            description: "Blocks .env files, SSH keys and key material.".into(),
            allow: vec![],
            ask: vec![],
            deny: strings(&[
                "Read(./.env)",
                "Read(./.env.*)",
                "Read(~/.ssh/**)",
                "Read(**/*.pem)",
                "Read(**/*.key)",
            ]),
            default_mode: None,
        },
        PermissionTemplate {
            id: "protect-agent-config".into(),
            title: "Protect agent and hook settings".into(),
            description: "The agent cannot edit its own settings, the hook files of Claude Code, \
                          Antigravity, Codex and Grok, or the app's folder, so it cannot turn off \
                          phone approval."
                .into(),
            allow: vec![],
            ask: vec![],
            deny: strings(AGENT_CONFIG_DENY),
            default_mode: None,
        },
        PermissionTemplate {
            id: "project-work".into(),
            title: "Work in the project (accept edits + dev tools)".into(),
            description: "Edits inside the folder the session started in are accepted without asking, \
                          and the listed build/test/git commands run without asking. Pushing, deleting, \
                          sudo and secrets still need a decision or stay blocked."
                .into(),
            allow: strings(&[
                "Read",
                "Glob",
                "Grep",
                "Bash(git status:*)",
                "Bash(git diff:*)",
                "Bash(git log:*)",
                "Bash(git add:*)",
                "Bash(git commit:*)",
                "Bash(git switch:*)",
                "Bash(git branch:*)",
                "Bash(ls:*)",
                "Bash(make:*)",
                "Bash(cargo build:*)",
                "Bash(cargo test:*)",
                "Bash(cargo fmt:*)",
                "Bash(cargo clippy:*)",
                "Bash(npm run:*)",
                "Bash(npm test:*)",
                "Bash(xcodebuild:*)",
            ]),
            ask: strings(&[
                "Bash(rm:*)",
                "Bash(git push:*)",
                "Bash(git reset:*)",
                "Bash(sudo:*)",
            ]),
            deny: strings(&[
                "Read(./.env)",
                "Read(./.env.*)",
                "Read(~/.ssh/**)",
                "Read(**/*.pem)",
                "Read(**/*.key)",
            ])
            .into_iter()
            .chain(strings(AGENT_CONFIG_DENY))
            .collect(),
            default_mode: Some("acceptEdits".into()),
        },
    ]
}
