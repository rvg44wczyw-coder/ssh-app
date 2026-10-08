use crate::error::SshCoreError;
use regex::Regex;
use std::sync::LazyLock;

static SESSION_NAME_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9_\-\.]+$").expect("Static regex is valid"));

/// Validates that a tmux session name contains only safe alphanumeric and hyphen/underscore characters.
/// Rejects empty names, names exceeding 64 characters, and any characters that could allow shell injection.
pub fn validate_session_name(name: &str) -> Result<(), SshCoreError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(SshCoreError::InvalidSessionName {
            name: name.to_string(),
            reason: "Session name cannot be empty".to_string(),
        });
    }

    if trimmed.len() > 64 {
        return Err(SshCoreError::InvalidSessionName {
            name: name.to_string(),
            reason: "Session name cannot exceed 64 characters".to_string(),
        });
    }

    if !SESSION_NAME_REGEX.is_match(trimmed) {
        return Err(SshCoreError::InvalidSessionName {
            name: name.to_string(),
            reason: "Session name contains invalid characters; only [a-zA-Z0-9_.-] are permitted"
                .to_string(),
        });
    }

    Ok(())
}

/// Quotes a string safely for POSIX shell single-quoted parameter expansion.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Builds the remote command to attach to an existing tmux session or create a new one.
///
/// Execution details:
/// 1. Uses dynamic login shell (`${SHELL:-/bin/zsh} -l -c ...`) to ensure ~/.zshrc, PATH, and API keys are loaded.
/// 2. Ensures TrueColor override (`terminal-overrides ",xterm-256color:Tc"`).
/// 3. Invokes `tmux new-session -A -s <session_name>`.
pub fn build_tmux_attach_or_create_command(
    session_name: &str,
    initial_command: Option<&str>,
) -> Result<String, SshCoreError> {
    validate_session_name(session_name)?;

    let mut tmux_cmd = format!("tmux new-session -A -s {session_name}");
    if let Some(cmd) = initial_command {
        let trimmed_cmd = cmd.trim();
        if !trimmed_cmd.is_empty() {
            // Escape single quotes for POSIX shell single-quoted string
            let escaped_cmd = trimmed_cmd.replace('\'', "'\\''");
            tmux_cmd.push_str(&format!(" '{escaped_cmd}'"));
        }
    }

    // Escape single quotes inside the inner tmux command for the outer shell wrapper
    let escaped_inner = tmux_cmd.replace('\'', "'\\''");

    // Execute via login shell to source remote environment, PATH, and agent keys.
    // If tmux is installed, configure TrueColor override, capture last 1000 lines of history if available, and attach/create session.
    // If tmux is absent, warn the user in yellow and fallback to a plain login shell.
    let full_command = format!(
        "exec ${{SHELL:-/bin/zsh}} -l -c 'if command -v tmux >/dev/null 2>&1; then tmux set -g history-limit 10000 2>/dev/null; tmux set -ga terminal-overrides \"xterm*:Tc,xterm*:smcup@:rmcup@\" 2>/dev/null; tmux capture-pane -t {session_name} -e -p -J -S -1000 -E -1 2>/dev/null || true; {escaped_inner}; else printf \"\\033[1;33m[Warning] tmux is not installed on remote host. Falling back to plain login shell.\\033[0m\\r\\n\"; exec ${{SHELL:-/bin/zsh}} -l; fi'"
    );

    Ok(full_command)
}

/// Builds command to detach existing clients from a session.
pub fn build_tmux_detach_command(session_name: &str) -> Result<String, SshCoreError> {
    validate_session_name(session_name)?;
    Ok(format!(
        "exec ${{SHELL:-/bin/zsh}} -l -c 'export PATH=\"/opt/homebrew/bin:/usr/local/bin:$PATH\"; tmux detach-client -s {session_name}'"
    ))
}

/// Builds command to terminate a tmux session.
pub fn build_tmux_kill_session_command(session_name: &str) -> Result<String, SshCoreError> {
    validate_session_name(session_name)?;
    Ok(format!(
        "exec ${{SHELL:-/bin/zsh}} -l -c 'export PATH=\"/opt/homebrew/bin:/usr/local/bin:$PATH\"; tmux kill-session -t {session_name}'"
    ))
}

/// Structured information about an active remote tmux session.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TmuxSessionInfo {
    pub name: String,
    pub windows: u32,
    pub attached: bool,
    pub created_timestamp: u64,
}

/// Builds command to list active tmux session names.
pub fn build_tmux_list_sessions_command() -> String {
    "${SHELL:-/bin/zsh} -l -c 'export PATH=\"/opt/homebrew/bin:/usr/local/bin:$PATH\"; tmux list-sessions -F \"#{session_name}\" 2>/dev/null || true'".to_string()
}

/// Builds command to list active tmux sessions in a machine-readable format:
/// name|windows|attached(1/0)|created_epoch
pub fn build_tmux_list_sessions_detailed_command() -> String {
    "${SHELL:-/bin/zsh} -l -c 'export PATH=\"/opt/homebrew/bin:/usr/local/bin:$PATH\"; tmux list-sessions -F \"#{session_name}|#{session_windows}|#{?session_attached,1,0}|#{session_created}\" 2>/dev/null || true'".to_string()
}

/// Parses machine-readable output from `build_tmux_list_sessions_detailed_command`.
pub fn parse_tmux_sessions_output(output: &str) -> Vec<TmuxSessionInfo> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() < 4 {
                return None;
            }
            let name = parts[0].trim().to_string();
            if name.is_empty() || validate_session_name(&name).is_err() {
                return None;
            }
            let windows = parts[1].trim().parse::<u32>().unwrap_or(1);
            let attached = parts[2].trim() == "1";
            let created_timestamp = parts[3].trim().parse::<u64>().unwrap_or(0);

            Some(TmuxSessionInfo {
                name,
                windows,
                attached,
                created_timestamp,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_session_names() {
        assert!(validate_session_name("claude").is_ok());
        assert!(validate_session_name("agent-1").is_ok());
        assert!(validate_session_name("antigravity_01").is_ok());
        assert!(validate_session_name("local.llm-3").is_ok());
    }

    #[test]
    fn test_invalid_session_names() {
        assert!(validate_session_name("").is_err());
        assert!(validate_session_name("   ").is_err());
        assert!(validate_session_name("claude; rm -rf /").is_err());
        assert!(validate_session_name("agent$(whoami)").is_err());
        assert!(validate_session_name("agent`id`").is_err());
        assert!(validate_session_name("agent&bg").is_err());
        assert!(validate_session_name("name with spaces").is_err());
        assert!(validate_session_name(&"a".repeat(65)).is_err());
    }

    #[test]
    fn test_build_attach_or_create_command() {
        let cmd =
            build_tmux_attach_or_create_command("claude-code", None).expect("Valid session name");
        assert!(cmd.contains("tmux new-session -A -s claude-code"));
        assert!(cmd.contains("${SHELL:-/bin/zsh} -l -c"));
        assert!(cmd.contains("terminal-overrides \"xterm*:Tc,xterm*:smcup@:rmcup@\""));
        assert!(cmd.contains("capture-pane -t claude-code -e -p -J -S -1000 -E -1"));
    }

    #[test]
    fn test_build_command_with_initial_program() {
        let cmd = build_tmux_attach_or_create_command("local-llm", Some("ollama run llama3"))
            .expect("Valid session name");
        assert!(cmd.contains("'ollama run llama3'"));
    }

    #[test]
    fn test_parse_tmux_sessions_output() {
        let raw = "\
term-1|2|1|1728000000
term-2|1|0|1728000500
invalid_line_without_enough_pipes
bad name;|1|0|1728000600

term-3|3|0|1728001000
";
        let sessions = parse_tmux_sessions_output(raw);
        assert_eq!(sessions.len(), 3);

        assert_eq!(sessions[0].name, "term-1");
        assert_eq!(sessions[0].windows, 2);
        assert!(sessions[0].attached);
        assert_eq!(sessions[0].created_timestamp, 1728000000);

        assert_eq!(sessions[1].name, "term-2");
        assert_eq!(sessions[1].windows, 1);
        assert!(!sessions[1].attached);
        assert_eq!(sessions[1].created_timestamp, 1728000500);

        assert_eq!(sessions[2].name, "term-3");
        assert_eq!(sessions[2].windows, 3);
        assert!(!sessions[2].attached);
        assert_eq!(sessions[2].created_timestamp, 1728001000);
    }

    #[test]
    fn test_build_kill_session_command() {
        assert!(build_tmux_kill_session_command("term-1").is_ok());
        assert!(build_tmux_kill_session_command("term; rm -rf /").is_err());
    }
}
