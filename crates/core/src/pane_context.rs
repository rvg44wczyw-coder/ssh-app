//! Is the terminal tab sitting in an AI CLI (Claude Code, Antigravity, Codex, ...) or in a plain
//! shell? Inside an AI CLI a line is a prompt for the model; a shell command has to start with
//! `!` to be run as one. The answer comes from the processes under the tmux pane, not from how
//! the tab was started: the user may start `claude` by hand, or leave it and be back in the shell.

use crate::error::SshCoreError;
use crate::tmux::{shell_quote, validate_session_name};

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PaneKind {
    /// A shell (or any program that is not an AI CLI): commands are typed as they are.
    Shell,
    /// An AI CLI is running in the pane: shell commands need a leading `!`.
    AiCli,
    /// The pane could not be inspected (no tmux session, host error): treated like a shell.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PaneContext {
    pub kind: PaneKind,
    /// Name of the AI CLI that was recognised (`claude`, `agy`, `codex`, ...), empty otherwise.
    pub agent: String,
}

impl PaneContext {
    pub fn unknown() -> Self {
        PaneContext {
            kind: PaneKind::Unknown,
            agent: String::new(),
        }
    }
}

const PANE_SCRIPT: &str = r#"PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"; export PATH
pid=$(tmux display-message -p -t "=$1:" '#{pane_pid}' 2>/dev/null)
case "$pid" in ""|*[!0-9]*) printf 'PC_STATUS=no_session\n'; exit 0;; esac
printf 'PC_STATUS=ok\n'
ps -Ao pid=,ppid=,args= 2>/dev/null | awk -v root="$pid" '
  { id=$1; par=$2; $1=""; $2=""; sub(/^[ \t]+/, ""); pp[id]=par; cmd[id]=substr($0, 1, 300) }
  END { for (p in pp) { q=p; d=0; while (q != "" && q != 0 && d < 40) { if (q == root) { print "PC_PROC=" cmd[p]; break } q=pp[q]; d++ } } }' | head -n 80
exit 0"#;

pub fn build_pane_context_command(session_name: &str) -> Result<String, SshCoreError> {
    validate_session_name(session_name)?;
    Ok(crate::hostprobe::login_sh_script(
        PANE_SCRIPT,
        &[shell_quote(session_name.trim())],
        8,
    ))
}

const AI_PROGRAMS: &[&str] = &[
    "claude",
    "agy",
    "antigravity",
    "codex",
    "grok",
    "gemini",
    "aider",
    "opencode",
    "crush",
    "cursor-agent",
    "amp",
    "goose",
];
/// Fragments of the command line of an agent started through node/python wrappers.
const AI_FRAGMENTS: &[&str] = &[
    "@anthropic-ai/claude-code",
    "/claude-code/",
    "@openai/codex",
    "@google/gemini-cli",
    "/antigravity/",
    "aider-chat",
];

/// The AI CLI a process command line belongs to, if any.
pub fn ai_cli_in(command: &str) -> Option<String> {
    let lower = command.to_lowercase();
    let mut tokens = lower.split_whitespace();
    let first = tokens.next().unwrap_or("");
    let base = |t: &str| t.rsplit('/').next().unwrap_or(t).to_string();
    let program = base(first);
    if AI_PROGRAMS.contains(&program.as_str()) {
        return Some(program);
    }
    // `node /path/to/claude`, `python3 -m aider`, `bun x codex`: the script / module name.
    if matches!(
        program.as_str(),
        "node" | "bun" | "deno" | "python" | "python3" | "npx" | "uv" | "uvx"
    ) {
        for t in tokens.take(4) {
            let name = base(t);
            let name = name
                .trim_end_matches(".js")
                .trim_end_matches(".mjs")
                .trim_end_matches(".py");
            if AI_PROGRAMS.contains(&name) {
                return Some(name.to_string());
            }
        }
    }
    AI_FRAGMENTS.iter().find(|f| lower.contains(*f)).map(|f| {
        f.trim_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(f)
            .to_string()
    })
}

pub fn parse_pane_context(output: &str) -> PaneContext {
    if !output.lines().any(|l| l == "PC_STATUS=ok") {
        return PaneContext::unknown();
    }
    for line in output.lines().filter_map(|l| l.strip_prefix("PC_PROC=")) {
        if let Some(agent) = ai_cli_in(line) {
            return PaneContext {
                kind: PaneKind::AiCli,
                agent,
            };
        }
    }
    PaneContext {
        kind: PaneKind::Shell,
        agent: String::new(),
    }
}

/// What to type to run `command` as a **shell** command in this pane: with a leading `!` inside
/// an AI CLI, unchanged in a shell. Already prefixed commands are left alone.
#[uniffi::export]
pub fn shell_command_for_pane(context: PaneContext, command: String) -> String {
    let trimmed = command.trim_start();
    if context.kind == PaneKind::AiCli && !trimmed.starts_with('!') {
        format!("!{trimmed}")
    } else {
        command
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(out: &str) -> PaneContext {
        parse_pane_context(out)
    }

    #[test]
    fn plain_shell_and_ai_cli() {
        let shell = ctx("PC_STATUS=ok\nPC_PROC=-zsh\nPC_PROC=vim notes.md\nPC_PROC=tmux: client\n");
        assert_eq!(shell.kind, PaneKind::Shell);
        let claude = ctx("PC_STATUS=ok\nPC_PROC=-zsh\nPC_PROC=claude --resume\n");
        assert_eq!(
            (claude.kind, claude.agent.as_str()),
            (PaneKind::AiCli, "claude")
        );
        let agy = ctx("PC_STATUS=ok\nPC_PROC=/Users/u/.local/bin/agy\n");
        assert_eq!(agy.agent, "agy");
    }

    #[test]
    fn wrappers_and_installed_packages_are_recognised() {
        for (line, want) in [
            (
                "node /opt/homebrew/lib/node_modules/@anthropic-ai/claude-code/cli.js",
                "claude-code",
            ),
            ("node /usr/local/bin/codex", "codex"),
            ("python3 -m aider --model x", "aider"),
            ("/opt/homebrew/bin/claude", "claude"),
        ] {
            assert_eq!(ai_cli_in(line).as_deref(), Some(want), "{line}");
        }
    }

    #[test]
    fn lookalikes_are_not_ai_clis() {
        for line in [
            "vim legacy.txt",
            "cat agy-notes.md",
            "node server.js",
            "-zsh",
            "python3 script.py",
            "tail -f claude.log",
            "grep claude file",
        ] {
            assert_eq!(ai_cli_in(line), None, "{line}");
        }
    }

    #[test]
    fn unknown_when_the_pane_cannot_be_read() {
        assert_eq!(ctx("PC_STATUS=no_session\n").kind, PaneKind::Unknown);
        assert_eq!(ctx("").kind, PaneKind::Unknown);
        assert_eq!(ctx("Traceback").kind, PaneKind::Unknown);
        // A process line cannot fake the status line.
        assert_eq!(ctx("PC_PROC=PC_STATUS=ok\n").kind, PaneKind::Unknown);
    }

    #[test]
    fn the_bang_prefix_is_added_only_in_an_ai_cli() {
        let ai = PaneContext {
            kind: PaneKind::AiCli,
            agent: "claude".into(),
        };
        let sh = PaneContext {
            kind: PaneKind::Shell,
            agent: String::new(),
        };
        assert_eq!(
            shell_command_for_pane(ai.clone(), "git diff".into()),
            "!git diff"
        );
        assert_eq!(
            shell_command_for_pane(ai.clone(), "!git diff".into()),
            "!git diff"
        );
        assert_eq!(shell_command_for_pane(ai, "  ls".into()), "!ls");
        assert_eq!(shell_command_for_pane(sh, "git diff".into()), "git diff");
        assert_eq!(
            shell_command_for_pane(PaneContext::unknown(), "ls".into()),
            "ls"
        );
    }

    #[test]
    fn command_is_built_for_valid_names_only() {
        assert!(build_pane_context_command("term-1").is_ok());
        assert!(build_pane_context_command("a b; rm").is_err());
    }
}
