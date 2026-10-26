//! The Hooks screen's view of one server (spec 61): which agents exist there, the state of their
//! user-level hook files and of the projects they were used in. The pinned host helper finds
//! everything (`hooks-overview`); this module builds the command and reads the reply.

use super::*;

const MAX_FOLDERS: usize = 64;

/// One place hooks can live: a project folder (`folder` set) with its file, or the user level.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HookTarget {
    pub folder: Option<String>,
    /// The file the hooks go to; `None` when the agent has no project-level file yet.
    pub file: Option<String>,
    /// `none`, `ours`, `foreign` or `unavailable`.
    pub state: String,
    /// An agent of this kind runs in a tmux pane there right now.
    pub running: bool,
    /// Epoch seconds of the agent's last activity there (0 = unknown).
    pub last_used: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentHooksOverview {
    /// `claude`, `agy`, `codex`, `grok`, `qwen`, `crush` or `opencode` (the agents with hooks).
    pub agent: String,
    /// The binary is on the host, or the agent was seen there.
    pub found: bool,
    pub user: HookTarget,
    pub projects: Vec<HookTarget>,
}

/// The pinned helper's `hooks-overview`; every folder travels as its own quoted argument.
pub(crate) fn build_hooks_overview_command(
    sha256: &str,
    folders: &[String],
    claude_dir: Option<&str>,
) -> Result<String, SshCoreError> {
    crate::usage::validate_sha256(sha256)?;
    if folders.len() > MAX_FOLDERS {
        return Err(invalid("Too many folders"));
    }
    let mut args: Vec<String> = vec![];
    for f in folders {
        validate_dir(f)?;
        args.push(shell_quote("--folder"));
        args.push(shell_quote(f));
    }
    if let Some(d) = claude_dir.map(str::trim).filter(|d| !d.is_empty()) {
        validate_dir(d)?;
        args.push(shell_quote("--claude-dir"));
        args.push(shell_quote(d));
    }
    // `$@` carries the quoted arguments into the helper.
    let script = format!(
        "h={hp}; if [ ! -f \"$h\" ] || [ -L \"$h\" ] || \
         [ \"{sha}\" != '{sha256}' ]; then echo HELPER_ABSENT; \
         else \"$h\" hooks-overview \"$@\" 2>/dev/null; fi",
        hp = crate::usage::HELPER_PATH_EXPR,
        sha = crate::hostprobe::SHA256_OF_H,
    );
    Ok(crate::hostprobe::login_sh_script(&script, &args, 20))
}

fn target(v: &Value, with_folder: bool) -> HookTarget {
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    HookTarget {
        folder: if with_folder { s("folder") } else { None },
        file: s("file"),
        state: s("state").unwrap_or_else(|| "none".into()),
        running: v.get("running").and_then(Value::as_bool).unwrap_or(false),
        last_used: v.get("last_used").and_then(Value::as_i64).unwrap_or(0),
    }
}

/// The helper's reply; `Err` when the helper is absent or the reply is unusable.
pub(crate) fn parse_hooks_overview(out: &str) -> Result<Vec<AgentHooksOverview>, SshCoreError> {
    if out.lines().any(|l| l.trim() == "HELPER_ABSENT") {
        return Err(SshCoreError::IoError {
            reason: "The host helper is missing or was replaced; reinstall it first".to_string(),
        });
    }
    let v = out
        .lines()
        .rev()
        .map(str::trim)
        .filter(|l| l.starts_with('{'))
        .find_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v.get("ok") == Some(&Value::Bool(true)))
        .ok_or_else(|| SshCoreError::IoError {
            reason: "The host helper gave no hooks overview (update it)".to_string(),
        })?;
    let agents = v["agents"].as_array().cloned().unwrap_or_default();
    Ok(agents
        .iter()
        .filter_map(|a| {
            let agent = a.get("agent")?.as_str()?.to_string();
            if !matches!(
                agent.as_str(),
                "claude" | "agy" | "codex" | "grok" | "qwen" | "crush" | "opencode"
            ) {
                return None;
            }
            Some(AgentHooksOverview {
                agent,
                found: a.get("found").and_then(Value::as_bool).unwrap_or(false),
                user: target(&a["user"], false),
                projects: a["projects"]
                    .as_array()
                    .map(|p| {
                        p.iter()
                            .take(MAX_FOLDERS)
                            .map(|t| target(t, true))
                            .collect()
                    })
                    .unwrap_or_default(),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn the_command_pins_the_helper_and_quotes_every_folder() {
        let c = build_hooks_overview_command(
            SHA,
            &["~/my dir".into(), "/a'b".into()],
            Some("~/.claude-x"),
        )
        .unwrap();
        assert!(c.contains(SHA) && c.contains("hooks-overview"));
        assert!(c.contains("--folder") && c.contains("--claude-dir"));
        assert!(!c.contains("; rm"), "{c}");
        assert!(build_hooks_overview_command("zz", &[], None).is_err());
        assert!(build_hooks_overview_command(SHA, &["a\nb".into()], None).is_err());
        let many = vec!["/x".to_string(); 65];
        assert!(build_hooks_overview_command(SHA, &many, None).is_err());
    }

    #[test]
    fn the_reply_becomes_agents_with_targets() {
        let out = r#"{"ok":true,"agents":[
 {"agent":"claude","found":true,"user":{"file":"/h/.claude/settings.json","state":"ours"},
  "projects":[{"folder":"/p","file":"/p/.claude/settings.local.json","state":"none","running":true,"last_used":5}]},
 {"agent":"evil","found":true,"user":{},"projects":[]},
 {"agent":"codex","found":false,"user":{"file":"/h/.codex/hooks.json","state":"none"},
  "projects":[{"folder":"/p","file":null,"state":"unavailable","running":false,"last_used":0}]}]}"#
            .replace('\n', " ");
        let r = parse_hooks_overview(&format!("noise\n{out}")).unwrap();
        assert_eq!(r.len(), 2, "an unknown agent id is dropped");
        assert_eq!(r[0].user.state, "ours");
        assert_eq!(r[0].projects[0].folder.as_deref(), Some("/p"));
        assert!(r[0].projects[0].running);
        assert_eq!(r[1].projects[0].file, None);
        assert_eq!(r[1].projects[0].state, "unavailable");
    }

    #[test]
    fn a_missing_or_old_helper_is_an_error_not_an_empty_list() {
        assert!(parse_hooks_overview("HELPER_ABSENT\n").is_err());
        assert!(parse_hooks_overview("").is_err());
        assert!(parse_hooks_overview("{\"ok\":false}").is_err());
    }
}
