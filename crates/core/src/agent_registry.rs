//! The registry of agents the feed knows (docs/features/60-universal-agent-feed.md): one table, owned by
//! the core, that says for each agent how it is recognised, how its terminal UI answers to the keys the
//! app sends, and which commands the app offers. Swift reads the descriptors instead of branching on the
//! agent. Adding an agent is one row here plus its two adapters (host `SessionSource`, screen
//! `DialectParser`); nothing else in the feed changes.
//!
//! Verified keys (live, private tmux, 2026-10-24): Claude Code takes Ctrl+X Ctrl+S for "send now";
//! Antigravity takes Alt+M. In both, Up brings queued messages back into the box and Ctrl+U empties it.

use crate::agents::AgentKind;
use crate::screen::ScreenDialect;

/// How an agent's terminal UI answers to the keys the app drives it with.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentKeys {
    /// Pushes a queued message now, interrupting the turn. `None`: the agent has no such key.
    pub send_now_keys: Option<Vec<u8>>,
    /// Queues the typed message while a turn runs. `None`: Enter queues it.
    pub queue_keys: Option<Vec<u8>>,
    /// Brings the queued messages back into the input box.
    pub pull_queue_keys: Vec<u8>,
    /// Empties the line the cursor is on.
    pub clear_line_keys: Vec<u8>,
    /// Turns the grey suggestion of the next prompt into real text. `None`: the agent shows none.
    pub accept_suggestion_keys: Option<Vec<u8>>,
    /// Put before the path of a picture in a message ("@" makes Claude Code read it).
    pub image_reference_prefix: String,
}

/// One slash command offered in the app's menu (it opens a screen of the agent).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentCommand {
    /// The text typed into the agent, starting with `/`.
    pub name: String,
    pub label: String,
    /// An SF Symbol name.
    pub icon: String,
    /// Shown as destructive (it wipes something).
    pub destructive: bool,
    /// A divider goes before this entry.
    pub divider_before: bool,
}

/// Where an agent keeps the conversation the feed reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SessionStore {
    /// A JSONL file the host reads by offset.
    File,
    /// A database the pinned host helper reads (Antigravity); its subagents are child conversations.
    Database,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentDescriptor {
    /// Stable id: `claude`, `agy`, `generic`, ...
    pub id: String,
    /// `None` for the generic entry.
    pub kind: Option<AgentKind>,
    pub display_name: String,
    /// Program names (the last path component, lower case) that mean this agent in a process list.
    pub process_names: Vec<String>,
    pub dialect: ScreenDialect,
    pub session_store: SessionStore,
    pub keys: AgentKeys,
    pub commands: Vec<AgentCommand>,
    /// The name hooks, the usage report and the host's session registry use for this agent
    /// (`claude`, `antigravity`, `codex`, `grok`); empty for the generic row.
    pub hook_name: String,
    /// Arguments that continue the last conversation when the agent is started again; empty when it
    /// has no such flag.
    pub resume_args: Vec<String>,
    /// The id the quick-command model menus list models for; empty when there is no model list.
    pub models_id: String,
    /// Whether the usage strip may show a 5-hour message estimate for this agent (read from its
    /// local transcripts).
    pub five_hour_estimate: bool,
}

const UP: [u8; 3] = [0x1b, 0x5b, 0x41];
const SHIFT_LEFT: [u8; 6] = [0x1b, 0x5b, 0x31, 0x3b, 0x32, 0x44];
const CTRL_ENTER: [u8; 7] = [0x1b, 0x5b, 0x31, 0x33, 0x3b, 0x35, 0x75];
const TAB: u8 = 0x09;
const CTRL_U: u8 = 0x15;

fn cmd(name: &str, label: &str, icon: &str) -> AgentCommand {
    AgentCommand {
        name: name.into(),
        label: label.into(),
        icon: icon.into(),
        destructive: false,
        divider_before: false,
    }
}

fn after_divider(mut c: AgentCommand) -> AgentCommand {
    c.divider_before = true;
    c
}

fn destructive(mut c: AgentCommand) -> AgentCommand {
    c.destructive = true;
    c
}

fn claude() -> AgentDescriptor {
    AgentDescriptor {
        id: "claude".into(),
        kind: Some(AgentKind::ClaudeCode),
        display_name: "Claude Code".into(),
        process_names: vec!["claude".into()],
        dialect: ScreenDialect::Claude,
        session_store: SessionStore::File,
        keys: AgentKeys {
            queue_keys: None,
            send_now_keys: Some(vec![0x18, 0x13]),
            pull_queue_keys: UP.to_vec(),
            clear_line_keys: vec![CTRL_U],
            accept_suggestion_keys: Some(vec![0x09]),
            image_reference_prefix: "@".into(),
        },
        commands: vec![
            destructive(cmd("/clear", "Clear", "trash")),
            cmd("/compact", "Compact", "arrow.down.right.and.arrow.up.left"),
            cmd("/context", "Context", "chart.pie"),
            cmd("/resume", "Resume", "clock.arrow.circlepath"),
            cmd("/memory", "Memory", "brain"),
            after_divider(cmd("/model", "Model", "cpu")),
            cmd("/permissions", "Permissions", "checkmark.shield"),
            cmd("/config", "Settings", "gearshape"),
            cmd("/mcp", "MCP servers", "server.rack"),
            cmd("/agents", "Agents", "person.2"),
            cmd("/hooks", "Hooks", "link"),
            after_divider(cmd("/status", "Status", "info.circle")),
            cmd("/cost", "Cost", "dollarsign.circle"),
            cmd("/doctor", "Doctor", "stethoscope"),
            cmd("/help", "Help", "questionmark.circle"),
        ],
        hook_name: "claude".into(),
        resume_args: vec!["--continue".into()],
        models_id: "claude".into(),
        five_hour_estimate: true,
    }
}

/// Antigravity's own list (read from its `/help`); `/exit` and `/logout` are left out.
fn agy() -> AgentDescriptor {
    AgentDescriptor {
        id: "agy".into(),
        kind: Some(AgentKind::Antigravity),
        display_name: "Google Antigravity".into(),
        process_names: vec!["agy".into(), "antigravity".into()],
        dialect: ScreenDialect::Agy,
        session_store: SessionStore::Database,
        keys: AgentKeys {
            queue_keys: None,
            send_now_keys: Some(vec![0x1b, 0x6d]),
            pull_queue_keys: UP.to_vec(),
            clear_line_keys: vec![CTRL_U],
            accept_suggestion_keys: None,
            image_reference_prefix: String::new(),
        },
        commands: vec![
            destructive(cmd("/clear", "Clear", "trash")),
            cmd("/context", "Context", "chart.pie"),
            cmd("/resume", "Resume", "clock.arrow.circlepath"),
            cmd("/rewind", "Rewind", "arrow.uturn.backward"),
            cmd("/fork", "Fork", "arrow.triangle.branch"),
            cmd("/rename", "Rename", "pencil"),
            cmd("/copy", "Copy", "doc.on.doc"),
            cmd("/diff", "Diff", "plusminus"),
            after_divider(cmd("/model", "Model", "cpu")),
            cmd("/effort", "Effort", "gauge.with.dots.needle.50percent"),
            cmd("/permissions", "Permissions", "checkmark.shield"),
            cmd("/config", "Settings", "gearshape"),
            cmd("/mcp", "MCP servers", "server.rack"),
            cmd("/agents", "Agents", "person.2"),
            cmd("/hooks", "Hooks", "link"),
            cmd("/plugin", "Plugins", "puzzlepiece.extension"),
            after_divider(cmd("/credits", "Credits", "dollarsign.circle")),
            cmd("/changelog", "Changelog", "doc.text"),
            cmd("/help", "Help", "questionmark.circle"),
        ],
        hook_name: "antigravity".into(),
        resume_args: vec!["--continue".into()],
        models_id: "antigravity".into(),
        five_hour_estimate: false,
    }
}

/// Codex: its session is read (rollout files); its screen is not parsed yet, so the dialect is `Unknown`
/// and nothing of its keys or commands is assumed until they are captured live (spec 60, phase 5).
fn codex() -> AgentDescriptor {
    AgentDescriptor {
        id: "codex".into(),
        kind: Some(AgentKind::Codex),
        display_name: "Codex".into(),
        process_names: vec!["codex".into()],
        dialect: ScreenDialect::Codex,
        session_store: SessionStore::File,
        // Verified live (Codex 0.164.0, private tmux, 2026-10-26): Enter during a turn sends the message
        // at once (it steers the turn) and Tab queues it, so there is no "send now" for a queued entry;
        // Shift+Left (ESC [ 1 ; 2 D) brings the last queued message back into the box; Up recalls history.
        keys: AgentKeys {
            queue_keys: Some(vec![TAB]),
            send_now_keys: None,
            pull_queue_keys: SHIFT_LEFT.to_vec(),
            clear_line_keys: vec![CTRL_U],
            accept_suggestion_keys: None,
            image_reference_prefix: String::new(),
        },
        commands: vec![
            destructive(cmd("/new", "New chat", "plus.bubble")),
            cmd("/compact", "Compact", "arrow.down.right.and.arrow.up.left"),
            cmd("/resume", "Resume", "clock.arrow.circlepath"),
            cmd("/fork", "Fork", "arrow.triangle.branch"),
            cmd("/diff", "Git diff", "plus.forwardslash.minus"),
            cmd("/review", "Review changes", "checkmark.seal"),
            cmd("/plan", "Plan mode", "list.bullet.clipboard"),
            cmd("/goal", "Goal", "target"),
            after_divider(cmd("/model", "Model", "cpu")),
            cmd("/permissions", "Permissions", "lock.shield"),
            cmd("/config", "Config", "gearshape"),
            cmd("/hooks", "Hooks", "link"),
            cmd("/skills", "Skills", "sparkles"),
            cmd("/agents", "Agents", "person.2"),
            after_divider(cmd("/status", "Status", "info.circle")),
            cmd("/ps", "Background terminals", "terminal"),
            cmd("/stop", "Stop background terminals", "stop.circle"),
            cmd("/init", "Create AGENTS.md", "doc.badge.plus"),
        ],
        hook_name: "codex".into(),
        resume_args: Vec::new(),
        models_id: "".into(),
        five_hour_estimate: false,
    }
}

fn grok() -> AgentDescriptor {
    AgentDescriptor {
        id: "grok".into(),
        kind: Some(AgentKind::Grok),
        display_name: "Grok".into(),
        process_names: vec!["grok".into()],
        dialect: ScreenDialect::Grok,
        session_store: SessionStore::File,
        // Verified live (Grok Build 1.0.70, private tmux, 2026-10-26): Enter during a turn queues the message
        // (`#1 text` under the spinner); Ctrl+Enter (CSI u, ESC [ 13 ; 5 u) sends the queue now; Up focuses the
        // queue and `e` brings the entry back into the box ("editing queued #1").
        keys: AgentKeys {
            queue_keys: None,
            send_now_keys: Some(CTRL_ENTER.to_vec()),
            pull_queue_keys: vec![0x1b, 0x5b, 0x41, b'e'],
            clear_line_keys: vec![CTRL_U],
            accept_suggestion_keys: None,
            image_reference_prefix: String::new(),
        },
        commands: vec![
            destructive(cmd("/new", "New session", "plus.bubble")),
            cmd("/compact", "Compact", "arrow.down.right.and.arrow.up.left"),
            cmd("/context", "Context", "chart.pie"),
            cmd("/resume", "Resume", "clock.arrow.circlepath"),
            cmd("/rewind", "Rewind", "arrow.uturn.backward"),
            cmd("/fork", "Fork", "arrow.triangle.branch"),
            cmd("/plan", "Plan mode", "list.bullet.clipboard"),
            cmd("/goal", "Goal", "target"),
            cmd("/recap", "Recap", "text.append"),
            after_divider(cmd("/model", "Model", "cpu")),
            cmd("/effort", "Effort", "gauge.with.dots.needle.67percent"),
            cmd("/auto", "Auto-review", "checkmark.shield"),
            cmd("/always-approve", "Always approve", "lock.open"),
            cmd("/settings", "Settings", "gearshape"),
            cmd("/hooks", "Hooks", "link"),
            cmd("/mcps", "MCP servers", "server.rack"),
            cmd("/skills", "Skills", "sparkles"),
            after_divider(cmd("/tasks", "Background tasks", "terminal")),
            cmd("/queue", "Queued prompts", "list.number"),
            cmd("/usage", "Usage", "chart.bar"),
            cmd("/doctor", "Doctor", "stethoscope"),
            cmd("/help", "Help", "questionmark.circle"),
        ],
        hook_name: "grok".into(),
        resume_args: Vec::new(),
        models_id: "".into(),
        five_hour_estimate: false,
    }
}

/// An agent not checked: nothing is assumed beyond the keys every terminal editor has.
fn generic() -> AgentDescriptor {
    AgentDescriptor {
        id: "generic".into(),
        kind: None,
        display_name: "Agent".into(),
        process_names: vec![],
        dialect: ScreenDialect::Unknown,
        session_store: SessionStore::File,
        keys: AgentKeys {
            queue_keys: None,
            send_now_keys: None,
            pull_queue_keys: UP.to_vec(),
            clear_line_keys: vec![CTRL_U],
            accept_suggestion_keys: None,
            image_reference_prefix: String::new(),
        },
        commands: vec![],
        hook_name: "".into(),
        resume_args: Vec::new(),
        models_id: "".into(),
        five_hour_estimate: false,
    }
}

/// Every agent the feed knows, the generic entry last.
#[uniffi::export]
pub fn agent_descriptors() -> Vec<AgentDescriptor> {
    vec![claude(), agy(), codex(), grok(), generic()]
}

/// The descriptor with this id (`claude`, `agy`, `generic`, ...).
#[uniffi::export]
pub fn agent_descriptor(id: String) -> Option<AgentDescriptor> {
    agent_descriptors().into_iter().find(|d| d.id == id)
}

/// The entry for an agent nothing is known about (never missing, whatever rows are added).
#[uniffi::export]
pub fn generic_agent_descriptor() -> AgentDescriptor {
    generic()
}

/// The descriptor of an agent kind; kinds without a row of their own get the generic one.
#[uniffi::export]
pub fn agent_descriptor_for_kind(kind: AgentKind) -> AgentDescriptor {
    agent_descriptors()
        .into_iter()
        .find(|d| d.kind == Some(kind))
        .unwrap_or_else(generic)
}

/// The descriptor of a program in a process list (`claude`, `agy`, ...), `None` when no agent has that
/// name. The name is the last path component, case-insensitive.
#[uniffi::export]
pub fn agent_descriptor_for_process(program: String) -> Option<AgentDescriptor> {
    let name = program.rsplit('/').next().unwrap_or("").to_lowercase();
    agent_descriptors()
        .into_iter()
        .find(|d| d.process_names.contains(&name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn claude_agy_and_generic_keep_the_keys_the_app_has_always_used() {
        let c = agent_descriptor("claude".into()).unwrap().keys;
        assert_eq!(c.send_now_keys, Some(vec![0x18, 0x13]));
        assert_eq!(c.pull_queue_keys, vec![0x1b, 0x5b, 0x41]);
        assert_eq!(c.clear_line_keys, vec![0x15]);
        assert_eq!(c.accept_suggestion_keys, Some(vec![0x09]));
        assert_eq!(c.image_reference_prefix, "@");
        let a = agent_descriptor("agy".into()).unwrap().keys;
        assert_eq!(a.send_now_keys, Some(vec![0x1b, 0x6d]));
        assert_eq!(a.accept_suggestion_keys, None);
        assert_eq!(a.image_reference_prefix, "");
        let g = agent_descriptor("generic".into()).unwrap().keys;
        assert_eq!(g.send_now_keys, None);
        assert_eq!(g.pull_queue_keys, vec![0x1b, 0x5b, 0x41]);
    }

    #[test]
    fn ids_kinds_and_process_names_are_unique() {
        let all = agent_descriptors();
        let ids: HashSet<_> = all.iter().map(|d| d.id.clone()).collect();
        assert_eq!(ids.len(), all.len());
        let procs: Vec<_> = all.iter().flat_map(|d| d.process_names.clone()).collect();
        let unique: HashSet<_> = procs.iter().collect();
        assert_eq!(unique.len(), procs.len());
        let kinds: Vec<_> = all.iter().filter_map(|d| d.kind).collect();
        let unique: HashSet<_> = kinds.iter().map(|k| format!("{k:?}")).collect();
        assert_eq!(unique.len(), kinds.len());
        assert_eq!(all.last().unwrap().id, "generic");
    }

    #[test]
    fn process_names_are_lower_case_program_names() {
        for d in agent_descriptors() {
            for p in &d.process_names {
                assert_eq!(*p, p.to_lowercase());
                assert!(crate::session_source::is_safe_program_name(p), "{p}");
            }
        }
    }

    #[test]
    fn commands_are_slash_commands_without_control_characters() {
        for d in agent_descriptors() {
            let names: HashSet<_> = d.commands.iter().map(|c| c.name.clone()).collect();
            assert_eq!(names.len(), d.commands.len(), "{}", d.id);
            for c in &d.commands {
                assert!(c.name.starts_with('/') && c.name.len() > 1, "{}", c.name);
                for s in [&c.name, &c.label, &c.icon] {
                    assert!(!s.chars().any(char::is_control), "{s:?}");
                }
            }
            assert!(!d.commands.first().is_some_and(|c| c.divider_before));
        }
    }

    #[test]
    fn lookup_by_kind_and_by_program() {
        assert_eq!(
            agent_descriptor_for_kind(AgentKind::ClaudeCode).id,
            "claude"
        );
        assert_eq!(agent_descriptor_for_kind(AgentKind::Antigravity).id, "agy");
        // No row of its own yet: the generic entry, not an error.
        assert_eq!(agent_descriptor_for_kind(AgentKind::Aider).id, "generic");
        assert_eq!(
            agent_descriptor_for_process("/usr/local/bin/Claude".into()).map(|d| d.id),
            Some("claude".into())
        );
        assert_eq!(
            agent_descriptor_for_process("antigravity".into()).map(|d| d.id),
            Some("agy".into())
        );
        assert!(agent_descriptor_for_process("vim".into()).is_none());
        assert!(agent_descriptor("nope".into()).is_none());
        assert_eq!(generic_agent_descriptor().id, "generic");
    }

    #[test]
    fn only_antigravity_reads_a_database() {
        for d in agent_descriptors() {
            let want = if d.id == "agy" {
                SessionStore::Database
            } else {
                SessionStore::File
            };
            assert_eq!(d.session_store, want, "{}", d.id);
        }
    }

    #[test]
    fn the_dialect_of_each_agent_is_its_own() {
        assert_eq!(
            agent_descriptor("claude".into()).unwrap().dialect,
            ScreenDialect::Claude
        );
        assert_eq!(
            agent_descriptor("agy".into()).unwrap().dialect,
            ScreenDialect::Agy
        );
        assert_eq!(
            agent_descriptor("generic".into()).unwrap().dialect,
            ScreenDialect::Unknown
        );
    }
}

#[cfg(test)]
mod descriptor_fields_tests {
    use super::*;

    #[test]
    fn hook_names_are_unique_and_only_claude_and_agy_resume_and_list_models() {
        let all = agent_descriptors();
        let names: Vec<&str> = all
            .iter()
            .map(|d| d.hook_name.as_str())
            .filter(|n| !n.is_empty())
            .collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(names.len(), sorted.len(), "{names:?}");
        let by = |id: &str| all.iter().find(|d| d.id == id).unwrap();
        assert_eq!(by("claude").resume_args, ["--continue"]);
        assert_eq!(by("agy").resume_args, ["--continue"]);
        assert_eq!(by("agy").hook_name, "antigravity");
        assert_eq!(by("agy").models_id, "antigravity");
        assert!(by("claude").five_hour_estimate && !by("agy").five_hour_estimate);
        for id in ["codex", "grok"] {
            assert!(
                by(id).resume_args.is_empty() && by(id).models_id.is_empty(),
                "{id}"
            );
        }
        assert!(generic_agent_descriptor().hook_name.is_empty());
    }
}
