//! Agent launch profiles (auto-start of Claude Code / Antigravity / Codex / Grok in a new tmux
//! session) and host-side binary probes. Hook installation lives in `agent_hooks`.
//!
//! Profiles never rely on shell aliases: the app starts the remote command through a login,
//! non-interactive shell (`$SHELL -l -c`), which does not read `~/.zshrc`. A personal/work split
//! such as `alias claude-work='CLAUDE_CONFIG_DIR=~/.claude claude'` is therefore expressed as an
//! explicit `env CLAUDE_CONFIG_DIR=... claude` command.

use crate::error::SshCoreError;
use crate::tmux::shell_quote;

/// Env vars that can hijack the agent process or the shell; never allowed in a profile.
const FORBIDDEN_ENV_KEYS: [&str; 6] = [
    "BASH_ENV",
    "ENV",
    "SHELLOPTS",
    "PS4",
    "IFS",
    "PROMPT_COMMAND",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AgentKind {
    ClaudeCode,
    Antigravity,
    Codex,
    Grok,
    /// Local-model agents (docs/features/23-local-models.md). OpenCode, Qwen Code and Crush have
    /// phone-approval hooks; the others are launch-only.
    OpenCode,
    QwenCode,
    Crush,
    Cline,
    Aider,
    Goose,
    OllamaChat,
}

impl AgentKind {
    /// Whether ssh-app can install approval hooks for this agent.
    pub fn has_hooks(self) -> bool {
        !matches!(
            self,
            AgentKind::Cline | AgentKind::Aider | AgentKind::Goose | AgentKind::OllamaChat
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct EnvVar {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentLaunchProfile {
    pub id: String,
    pub name: String,
    pub agent: AgentKind,
    pub binary: String,
    pub env: Vec<EnvVar>,
    pub args: Vec<String>,
    pub working_dir: Option<String>,
    /// Git branch to switch to (in the start directory) before the agent starts. A failed
    /// switch (no such branch, dirty tree) is reported in the tab and the agent still starts.
    pub git_branch: Option<String>,
    /// Permission mode the agent starts in: Claude `default|acceptEdits|plan|bypassPermissions`
    /// (`--permission-mode`), Antigravity `accept-edits|auto|plan` (`--mode`). `None` = the
    /// agent's own default. Other agents reject a mode.
    pub permission_mode: Option<String>,
    /// Start the agent through an interactive shell (`$SHELL -i -c`) so `~/.zshrc` (PATH from
    /// nvm/npm/asdf, exports) is loaded. Login non-interactive shells do not read it.
    pub load_shell_rc: bool,
}

/// Where a binary resolves on the host in a login shell vs. an interactive shell (rc loaded).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentEnvCheck {
    pub login_path: Option<String>,
    pub interactive_path: Option<String>,
    /// The interactive probe did not finish in time (a `~/.zshrc` that blocks or is very slow).
    pub interactive_timed_out: bool,
}

pub(crate) fn invalid(reason: impl Into<String>) -> SshCoreError {
    SshCoreError::InvalidInput {
        reason: reason.into(),
    }
}

pub(crate) fn has_control(s: &str) -> bool {
    s.chars().any(char::is_control)
}

/// Default presets. Personal/work variants are user data (see `config_dir` env), not presets.
#[uniffi::export]
pub fn default_launch_profiles() -> Vec<AgentLaunchProfile> {
    let p = |id: &str, name: &str, agent, binary: &str| AgentLaunchProfile {
        id: id.into(),
        name: name.into(),
        agent,
        binary: binary.into(),
        env: vec![],
        args: vec![],
        working_dir: None,
        git_branch: None,
        permission_mode: None,
        load_shell_rc: true,
    };
    vec![
        p("claude", "Claude Code", AgentKind::ClaudeCode, "claude"),
        p(
            "antigravity",
            "Google Antigravity",
            AgentKind::Antigravity,
            "agy",
        ),
        p("codex", "OpenAI Codex", AgentKind::Codex, "codex"),
        p("grok", "Grok", AgentKind::Grok, "grok"),
    ]
    .into_iter()
    .chain(
        LOCAL_TOOLS
            .iter()
            .filter_map(|t| build_local_agent_profile(t.id.into(), String::new()).ok()),
    )
    .collect()
}

/// One local-model tool the app can launch (see `local_agent_tools`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LocalAgentTool {
    pub id: String,
    pub name: String,
    pub agent: AgentKind,
    /// Phone approvals can be installed for it.
    pub has_approvals: bool,
    /// The model can be chosen from the host's Ollama models (otherwise the tool picks its own).
    pub takes_model: bool,
    /// One line for the editor: what to know before using it.
    pub note: String,
}

struct LocalTool {
    id: &'static str,
    name: &'static str,
    agent: AgentKind,
    takes_model: bool,
    note: &'static str,
}

const LOCAL_TOOLS: [LocalTool; 7] = [
    LocalTool {
        id: "opencode",
        name: "OpenCode (local model)",
        agent: AgentKind::OpenCode,
        takes_model: true,
        note: "Started with `ollama launch opencode`, which writes OpenCode's config on the host. Phone approvals cover shell commands.",
    },
    LocalTool {
        id: "qwen",
        name: "Qwen Code (local model)",
        agent: AgentKind::QwenCode,
        takes_model: true,
        note: "Started with `ollama launch qwen`, which writes Qwen Code's config on the host.",
    },
    LocalTool {
        id: "crush",
        name: "Crush",
        agent: AgentKind::Crush,
        takes_model: false,
        note: "Crush picks its model in its own config. Phone approvals cover tool calls only (no notifications).",
    },
    LocalTool {
        id: "cline",
        name: "Cline (local model)",
        agent: AgentKind::Cline,
        takes_model: true,
        note: "No approval hook: Cline's plugin hooks time out after 3 s and fail open. It asks in its own terminal.",
    },
    LocalTool {
        id: "aider",
        name: "Aider (local model)",
        agent: AgentKind::Aider,
        takes_model: true,
        note: "No approval hook; Aider asks y/n in its own terminal.",
    },
    LocalTool {
        id: "goose",
        name: "Goose (local model)",
        agent: AgentKind::Goose,
        takes_model: true,
        note: "No approval hook found; use Goose's own permission mode.",
    },
    LocalTool {
        id: "ollama",
        name: "Ollama chat",
        agent: AgentKind::OllamaChat,
        takes_model: true,
        note: "A model chat: it runs no tools, so there is nothing to approve.",
    },
];

#[uniffi::export]
pub fn local_agent_tools() -> Vec<LocalAgentTool> {
    LOCAL_TOOLS
        .iter()
        .map(|t| LocalAgentTool {
            id: t.id.into(),
            name: t.name.into(),
            agent: t.agent,
            has_approvals: t.agent.has_hooks(),
            takes_model: t.takes_model,
            note: t.note.into(),
        })
        .collect()
}

/// A model name that is safe as one command-line argument: the Ollama name charset and not an
/// option (the charset alone allows a leading `-`).
fn validate_local_model(model: &str) -> Result<(), SshCoreError> {
    if model.starts_with('-') || !crate::ai::is_model_name(model) {
        return Err(invalid(format!(
            "Invalid model name '{}'",
            model.chars().take(40).collect::<String>().escape_default()
        )));
    }
    Ok(())
}

/// Launch profile for a local-model tool. An empty `model` leaves the choice to the tool (or to
/// Ollama's own picker). Pure: nothing is sent to the host here.
#[uniffi::export]
pub fn build_local_agent_profile(
    tool: String,
    model: String,
) -> Result<AgentLaunchProfile, SshCoreError> {
    let t = LOCAL_TOOLS
        .iter()
        .find(|t| t.id == tool)
        .ok_or_else(|| invalid(format!("Unknown local tool '{}'", tool.escape_default())))?;
    let model = model.trim();
    if !model.is_empty() {
        if !t.takes_model {
            return Err(invalid(format!("{} chooses its model itself", t.name)));
        }
        validate_local_model(model)?;
    }
    let env = |k: &str, v: &str| EnvVar {
        key: k.into(),
        value: v.into(),
    };
    let strs = |v: &[&str]| v.iter().map(|x| (*x).to_string()).collect::<Vec<_>>();
    let with_model = |mut base: Vec<String>, flag: &str, value: String| {
        if !model.is_empty() {
            base.push(flag.into());
            base.push(value);
        }
        base
    };
    let (binary, args, env_vars) = match t.id {
        "opencode" | "qwen" | "cline" => (
            "ollama",
            with_model(strs(&["launch", t.id]), "--model", model.into()),
            vec![],
        ),
        "crush" => ("crush", vec![], vec![]),
        "aider" => (
            "aider",
            with_model(vec![], "--model", format!("ollama_chat/{model}")),
            vec![env("OLLAMA_API_BASE", "http://127.0.0.1:11434")],
        ),
        "goose" => {
            let mut e = vec![env("GOOSE_PROVIDER", "ollama")];
            if !model.is_empty() {
                e.push(env("GOOSE_MODEL", model));
            }
            ("goose", strs(&["session"]), e)
        }
        _ => (
            "ollama",
            if model.is_empty() {
                vec![]
            } else {
                strs(&["run", model])
            },
            vec![],
        ),
    };
    Ok(AgentLaunchProfile {
        id: format!("local-{}", t.id),
        name: t.name.into(),
        agent: t.agent,
        binary: binary.into(),
        env: env_vars,
        args,
        working_dir: None,
        git_branch: None,
        permission_mode: None,
        load_shell_rc: true,
    })
}

/// Name of the env var an agent uses to select its config directory, if it has one.
#[uniffi::export]
pub fn agent_config_dir_env(agent: AgentKind) -> Option<String> {
    match agent {
        AgentKind::ClaudeCode => Some("CLAUDE_CONFIG_DIR".into()),
        AgentKind::Codex => Some("CODEX_HOME".into()),
        _ => None,
    }
}

/// The config directory a profile points at (value of its config-dir env var), if set.
#[uniffi::export]
pub fn profile_config_dir(profile: AgentLaunchProfile) -> Option<String> {
    let key = agent_config_dir_env(profile.agent)?;
    profile
        .env
        .into_iter()
        .find(|e| e.key == key)
        .map(|e| e.value)
}

fn validate_env_key(key: &str) -> Result<(), SshCoreError> {
    let mut chars = key.chars();
    let ok_first = chars
        .next()
        .map(|c| c.is_ascii_alphabetic() || c == '_')
        .unwrap_or(false);
    if !ok_first || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(invalid(format!(
            "Invalid environment variable name '{key}'"
        )));
    }
    let upper = key.to_ascii_uppercase();
    if upper.starts_with("LD_")
        || upper.starts_with("DYLD_")
        || FORBIDDEN_ENV_KEYS.contains(&upper.as_str())
    {
        return Err(invalid(format!(
            "Environment variable '{key}' is not allowed"
        )));
    }
    // Profile env values end up in the remote process list (`ps` shows tmux's argv) and in
    // UserDefaults; secrets belong in the host's own environment/keychain, not here.
    const SECRETISH: [&str; 6] = ["KEY", "TOKEN", "SECRET", "PASSWORD", "PASSWD", "CREDENTIAL"];
    if SECRETISH.iter().any(|w| upper.contains(w)) {
        return Err(invalid(format!(
            "'{key}' looks like a secret; profile env is stored unencrypted and visible in the host's process list. Set it in the host's shell profile instead"
        )));
    }
    Ok(())
}

fn validate_binary(bin: &str) -> Result<(), SshCoreError> {
    if bin.is_empty()
        || bin.starts_with('-')
        || !bin
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | '~'))
    {
        return Err(invalid(format!("Invalid binary '{bin}'")));
    }
    Ok(())
}

#[uniffi::export]
pub fn validate_launch_profile(profile: AgentLaunchProfile) -> Result<(), SshCoreError> {
    if profile.name.trim().is_empty() || has_control(&profile.name) {
        return Err(invalid(
            "Profile name is empty or contains control characters",
        ));
    }
    validate_binary(&profile.binary)?;
    for e in &profile.env {
        validate_env_key(&e.key)?;
        if has_control(&e.value) {
            return Err(invalid(format!(
                "Value of '{}' contains control characters",
                e.key
            )));
        }
    }
    if profile.args.iter().any(|a| has_control(a)) {
        return Err(invalid("Argument contains control characters"));
    }
    if let Some(dir) = &profile.working_dir {
        if has_control(dir) {
            return Err(invalid("Working directory contains control characters"));
        }
    }
    if let Some(b) = &profile.git_branch {
        validate_git_branch(b)?;
    }
    permission_mode_args(&profile)?;
    Ok(())
}

/// The launch flags for the profile's permission mode (empty when none is set).
fn permission_mode_args(profile: &AgentLaunchProfile) -> Result<Vec<String>, SshCoreError> {
    let Some(mode) = profile.permission_mode.as_deref().map(str::trim) else {
        return Ok(vec![]);
    };
    if mode.is_empty() {
        return Ok(vec![]);
    }
    let (flag, allowed): (&str, &[&str]) = match profile.agent {
        AgentKind::ClaudeCode => (
            "--permission-mode",
            &["default", "acceptEdits", "plan", "bypassPermissions"],
        ),
        AgentKind::Antigravity => ("--mode", &["accept-edits", "auto", "plan"]),
        _ => {
            return Err(invalid(
                "This agent has no permission mode option; leave it unset",
            ))
        }
    };
    if !allowed.contains(&mode) {
        return Err(invalid(format!(
            "Permission mode must be one of: {}",
            allowed.join(", ")
        )));
    }
    Ok(vec![flag.to_string(), mode.to_string()])
}

/// A branch name that is safe to pass to `git switch` unquoted-by-meaning: no leading `-`, no
/// `..`, only the characters git branches normally use.
pub(crate) fn validate_git_branch(b: &str) -> Result<(), SshCoreError> {
    let b = b.trim();
    if b.is_empty() {
        return Ok(());
    }
    if b.len() > 200
        || b.starts_with('-')
        || b.starts_with('/')
        || b.ends_with('/')
        || b.ends_with(".lock")
        || b.contains("..")
        || b.contains("//")
        || !b
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | '+' | '@'))
    {
        return Err(invalid(format!("Invalid git branch '{b}'")));
    }
    Ok(())
}

/// Quotes a remote path/value for the shell, expanding a leading `~` to `$HOME` safely.
pub(crate) fn remote_path_expr(path: &str) -> String {
    if path == "~" {
        "\"$HOME\"".to_string()
    } else if let Some(rest) = path.strip_prefix("~/") {
        format!("\"$HOME\"{}", shell_quote(&format!("/{rest}")))
    } else {
        shell_quote(path)
    }
}

/// Builds the command run inside the new tmux session: `[cd dir &&] env K=V... bin args...`,
/// followed by a login shell so the tmux session survives the agent exiting.
#[uniffi::export]
pub fn build_agent_launch_command(profile: AgentLaunchProfile) -> Result<String, SshCoreError> {
    validate_launch_profile(profile.clone())?;
    let mut cmd = String::new();
    if let Some(dir) = profile
        .working_dir
        .as_deref()
        .filter(|d| !d.trim().is_empty())
    {
        cmd.push_str(&format!("cd -- {} && ", remote_path_expr(dir.trim())));
    }
    if let Some(branch) = profile
        .git_branch
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
    {
        // The name is validated (no leading `-`, restricted charset), so it cannot be an option.
        let q = shell_quote(branch);
        cmd.push_str(&format!(
            "{{ git switch {q} || echo \"ssh-app: could not switch to branch \"{q}; }} && "
        ));
    }
    let mut inner = String::from("env");
    for e in &profile.env {
        inner.push_str(&format!(" {}={}", e.key, remote_path_expr(&e.value)));
    }
    // The binary charset is validated (no quoting needed); only a leading `~` needs expansion.
    if profile.binary.starts_with('~') {
        inner.push_str(&format!(" {}", remote_path_expr(&profile.binary)));
    } else {
        inner.push_str(&format!(" {}", profile.binary));
    }
    for a in permission_mode_args(&profile)?.iter().chain(&profile.args) {
        inner.push_str(&format!(" {}", shell_quote(a)));
    }
    // Claude prints calls and results whole with --verbose; the feed reads them off the screen.
    if profile.agent == AgentKind::ClaudeCode && !profile.args.iter().any(|a| a == "--verbose") {
        inner.push_str(" '--verbose'");
    }
    if profile.load_shell_rc {
        // `+m` (no job control): an interactive shell that owns the terminal's foreground group
        // leaves it behind when it exits, and the login shell below then dies on "error on TTY
        // read" - the tab closed when the agent was quit.
        cmd.push_str(&format!(
            "case \"${{SHELL##*/}}\" in zsh|bash) m=+m;; *) m=;; esac; \"${{SHELL:-/bin/zsh}}\" -i $m -c {}",
            shell_quote(&inner)
        ));
    } else {
        cmd.push_str(&inner);
    }
    cmd.push_str("; exec \"${SHELL:-/bin/zsh}\" -l");
    Ok(cmd)
}

/// Command that prints where `binary` resolves in a login shell (`interactive == false`) or an
/// interactive shell that has read `~/.zshrc`. Run it on the host and feed the output to
/// `parse_resolved_binary`.
#[uniffi::export]
pub fn build_binary_probe_command(
    binary: String,
    interactive: bool,
) -> Result<String, SshCoreError> {
    validate_binary(&binary)?;
    let flag = if interactive { "-i" } else { "-l" };
    let probe = format!("command -v {}", remote_path_expr(&binary));
    // The user's rc may block; the watchdog kills the probe shell after 8 s so the host is not
    // left with a stuck process every time the check runs (the client gives up at 10 s).
    let inner = format!(
        "\"${{SHELL:-/bin/zsh}}\" {flag} -c {} </dev/null 2>/dev/null",
        shell_quote(&probe)
    );
    Ok(crate::hostprobe::watchdog_sh(&inner, 8))
}

/// Extracts the resolved absolute path from probe output. Interactive shells may print noise
/// (banners, prompt frameworks) and `command -v` prints `alias ...` for aliases, so only the
/// last line that is an absolute path without control characters counts.
#[uniffi::export]
pub fn parse_resolved_binary(output: String) -> Option<String> {
    // Bidi/zero-width characters could make a path read differently from what it is.
    let spoofy = |c: char| matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}');
    output
        .lines()
        .map(str::trim)
        .rev()
        .find(|l| {
            l.starts_with('/') && l.len() <= 4096 && !has_control(l) && !l.chars().any(spoofy)
        })
        .map(str::to_string)
}

pub use crate::agent_hooks::*;

#[cfg(test)]
mod tests {
    fn local(tool: &str, model: &str) -> Result<AgentLaunchProfile, SshCoreError> {
        build_local_agent_profile(tool.into(), model.into())
    }

    /// The command without the interactive-shell wrapper, so quoting is easy to read.
    fn command(p: &AgentLaunchProfile) -> String {
        let mut p = p.clone();
        p.load_shell_rc = false;
        build_agent_launch_command(p).unwrap()
    }

    #[test]
    fn local_tools_build_the_documented_commands() {
        let c = command(&local("opencode", "gemma4:26b").unwrap());
        assert!(
            c.contains("env ollama 'launch' 'opencode' '--model' 'gemma4:26b'"),
            "{c}"
        );
        assert!(command(&local("qwen", "m").unwrap()).contains("'launch' 'qwen' '--model' 'm'"));
        assert!(command(&local("cline", "m").unwrap()).contains("'launch' 'cline' '--model' 'm'"));
        let aider = local("aider", "qwen2.5-coder:7b").unwrap();
        assert_eq!(aider.agent, AgentKind::Aider);
        let c = command(&aider);
        assert!(
            c.contains("OLLAMA_API_BASE='http://127.0.0.1:11434'"),
            "{c}"
        );
        assert!(
            c.contains("aider '--model' 'ollama_chat/qwen2.5-coder:7b'"),
            "{c}"
        );
        let c = command(&local("goose", "m").unwrap());
        assert!(
            c.contains("GOOSE_PROVIDER='ollama'") && c.contains("GOOSE_MODEL='m'"),
            "{c}"
        );
        assert!(c.contains("goose 'session'"), "{c}");
        assert!(command(&local("ollama", "m").unwrap()).contains("ollama 'run' 'm'"));
        assert!(command(&local("crush", "").unwrap()).contains("env crush;"));
    }

    #[test]
    fn local_tools_without_a_model_leave_the_choice_to_the_tool() {
        assert!(command(&local("opencode", "").unwrap())
            .ends_with("'launch' 'opencode'; exec \"${SHELL:-/bin/zsh}\" -l"));
        assert!(local("aider", " ").unwrap().args.is_empty());
        assert!(local("goose", "")
            .unwrap()
            .env
            .iter()
            .all(|e| e.key != "GOOSE_MODEL"));
        assert!(local("ollama", "").unwrap().args.is_empty());
    }

    #[test]
    fn local_tool_models_cannot_carry_shell_syntax_or_options() {
        for bad in [
            "; rm -rf ~",
            "$(touch x)",
            "`id`",
            "-h",
            "--version",
            "a b",
            "a\nb",
            "a'b",
            &"x".repeat(129),
        ] {
            for t in ["opencode", "aider", "goose", "ollama"] {
                assert!(local(t, bad).is_err(), "{t} accepted {bad:?}");
            }
        }
        assert!(local("crush", "m").is_err(), "Crush has no model flag");
        assert!(local("nope", "").is_err());
    }

    #[test]
    fn every_local_tool_is_a_valid_default_profile() {
        let defaults = default_launch_profiles();
        assert_eq!(defaults.len(), 4 + local_agent_tools().len());
        for p in &defaults {
            validate_launch_profile(p.clone()).unwrap();
            build_agent_launch_command(p.clone()).unwrap();
        }
        let tools = local_agent_tools();
        let with: Vec<_> = tools
            .iter()
            .filter(|t| t.has_approvals)
            .map(|t| t.id.as_str())
            .collect();
        assert_eq!(with, ["opencode", "qwen", "crush"]);
    }

    use super::*;
    fn profile(agent: AgentKind, bin: &str, env: &[(&str, &str)]) -> AgentLaunchProfile {
        AgentLaunchProfile {
            id: "x".into(),
            name: "X".into(),
            agent,
            binary: bin.into(),
            env: env
                .iter()
                .map(|(k, v)| EnvVar {
                    key: (*k).into(),
                    value: (*v).into(),
                })
                .collect(),
            args: vec![],
            working_dir: None,
            git_branch: None,
            permission_mode: None,
            load_shell_rc: false,
        }
    }

    #[test]
    fn launch_command_uses_explicit_env_not_aliases() {
        let p = profile(
            AgentKind::ClaudeCode,
            "claude",
            &[("CLAUDE_CONFIG_DIR", "~/.claude-personal")],
        );
        let c = build_agent_launch_command(p.clone()).unwrap();
        assert_eq!(
            c,
            "env CLAUDE_CONFIG_DIR=\"$HOME\"'/.claude-personal' claude '--verbose'; exec \"${SHELL:-/bin/zsh}\" -l"
        );
        assert_eq!(profile_config_dir(p).as_deref(), Some("~/.claude-personal"));
    }

    #[test]
    fn launch_command_quotes_hostile_input() {
        let mut p = profile(AgentKind::Codex, "codex", &[("FOO", "a'; rm -rf ~ #")]);
        p.args = vec!["--prompt".into(), "$(id) `id`".into()];
        p.working_dir = Some("~/my proj".into());
        let c = build_agent_launch_command(p).unwrap();
        assert!(c.starts_with("cd -- \"$HOME\"'/my proj' && env FOO='a'\\''; rm -rf ~ #'"));
        assert!(c.contains("'$(id) `id`'"));
    }

    #[test]
    fn launch_command_switches_branch_after_cd() {
        let mut p = profile(AgentKind::ClaudeCode, "claude", &[]);
        p.working_dir = Some("~/app".into());
        p.git_branch = Some("feature/x-1".into());
        let c = build_agent_launch_command(p).unwrap();
        assert!(c.starts_with(
            "cd -- \"$HOME\"'/app' && { git switch 'feature/x-1' || echo \"ssh-app: could not switch to branch \"'feature/x-1'; } && env claude '--verbose'"
        ));
        for bad in ["-c", "a b", "x;rm", "a..b", "$(id)", "/x", "x/", "a.lock"] {
            let mut p = profile(AgentKind::ClaudeCode, "claude", &[]);
            p.git_branch = Some(bad.into());
            assert!(validate_launch_profile(p).is_err(), "{bad}");
        }
    }

    #[test]
    fn permission_mode_becomes_the_agents_own_flag() {
        let mut p = profile(AgentKind::ClaudeCode, "claude", &[]);
        p.args = vec!["--model".into(), "x".into()];
        p.permission_mode = Some("acceptEdits".into());
        let c = build_agent_launch_command(p).unwrap();
        assert!(
            c.contains("claude '--permission-mode' 'acceptEdits' '--model' 'x'"),
            "{c}"
        );
        let mut p = profile(AgentKind::Antigravity, "agy", &[]);
        p.permission_mode = Some("accept-edits".into());
        let c = build_agent_launch_command(p).unwrap();
        assert!(c.contains("agy '--mode' 'accept-edits'"), "{c}");
        // Unset or blank adds nothing; wrong mode for the agent, unknown mode, or an agent
        // without the option is refused (never passed through to the shell).
        for (agent, mode) in [
            (AgentKind::ClaudeCode, "accept-edits"),
            (AgentKind::Antigravity, "acceptEdits"),
            (AgentKind::ClaudeCode, "x; rm -rf /"),
            (AgentKind::Codex, "auto"),
        ] {
            let mut p = profile(agent, "bin", &[]);
            p.permission_mode = Some(mode.into());
            assert!(validate_launch_profile(p).is_err(), "{mode}");
        }
        let mut p = profile(AgentKind::Codex, "codex", &[]);
        p.permission_mode = Some("  ".into());
        assert!(!build_agent_launch_command(p).unwrap().contains("--mode"));
    }

    #[test]
    fn profile_validation_rejects_bad_input() {
        for (bin, env) in [
            ("claude; rm", vec![]),
            ("-rf", vec![]),
            ("", vec![]),
            ("claude", vec![("1BAD", "x")]),
            ("claude", vec![("A B", "x")]),
            ("claude", vec![("LD_PRELOAD", "/x")]),
            ("claude", vec![("DYLD_INSERT_LIBRARIES", "/x")]),
            ("claude", vec![("BASH_ENV", "/x")]),
            ("claude", vec![("ANTHROPIC_API_KEY", "sk-x")]),
            ("claude", vec![("github_token", "x")]),
            ("claude", vec![("MY_SECRET", "x")]),
            ("claude", vec![("DB_PASSWORD", "x")]),
            ("claude", vec![("OK", "a\nb")]),
        ] {
            assert!(
                build_agent_launch_command(profile(AgentKind::ClaudeCode, bin, &env)).is_err(),
                "{bin} {env:?}"
            );
        }
        let mut p = profile(AgentKind::ClaudeCode, "claude", &[]);
        p.args = vec!["a\rb".into()];
        assert!(validate_launch_profile(p).is_err());
        let mut p = profile(AgentKind::ClaudeCode, "claude", &[]);
        p.name = "  ".into();
        assert!(validate_launch_profile(p).is_err());
    }

    #[test]
    fn non_secret_config_env_is_allowed() {
        for k in [
            "CLAUDE_CONFIG_DIR",
            "ANTHROPIC_BASE_URL",
            "NODE_ENV",
            "TERM",
        ] {
            assert!(
                build_agent_launch_command(profile(AgentKind::ClaudeCode, "claude", &[(k, "v")]))
                    .is_ok(),
                "{k}"
            );
        }
    }

    #[test]
    fn default_profiles_are_valid() {
        let d = default_launch_profiles();
        assert_eq!(d.len(), 4 + local_agent_tools().len());
        for p in d {
            assert!(build_agent_launch_command(p).is_ok());
        }
    }

    #[test]
    fn rc_profile_runs_through_interactive_shell_with_quoted_inner_command() {
        let mut p = profile(
            AgentKind::ClaudeCode,
            "claude",
            &[("CLAUDE_CONFIG_DIR", "~/.c p")],
        );
        p.args = vec!["it's".into()];
        p.load_shell_rc = true;
        let cmd = build_agent_launch_command(p).unwrap();
        assert!(
            cmd.contains("\"${SHELL:-/bin/zsh}\" -i $m -c '") && cmd.contains("m=+m"),
            "{cmd}"
        );
        assert!(cmd.contains("env CLAUDE_CONFIG_DIR="));
        assert!(cmd.ends_with("; exec \"${SHELL:-/bin/zsh}\" -l"));
    }

    #[test]
    fn probe_command_selects_shell_mode_and_rejects_bad_binary() {
        let login = build_binary_probe_command("claude".into(), false).unwrap();
        assert!(login.starts_with("sh -c '"), "{login}");
        assert!(login.contains(" -l -c "), "{login}");
        assert!(
            login.contains("claude") && login.contains("kill -9"),
            "{login}"
        );
        let inter = build_binary_probe_command("~/bin/agy".into(), true).unwrap();
        assert!(inter.contains(" -i -c "), "{inter}");
        assert!(build_binary_probe_command("a; rm -rf ~".into(), true).is_err());
        assert!(build_binary_probe_command("-x".into(), false).is_err());
    }

    #[test]
    fn probe_command_runs_and_kills_a_hanging_shell() {
        use std::process::{Command, Stdio};
        let run = |shell: &str, bin: &str| {
            let cmd = build_binary_probe_command(bin.into(), true).unwrap();
            let t = std::time::Instant::now();
            let out = Command::new("sh")
                .args(["-c", &cmd])
                .env("SHELL", shell)
                .stderr(Stdio::null())
                .output()
                .unwrap();
            (
                String::from_utf8_lossy(&out.stdout).to_string(),
                t.elapsed(),
            )
        };
        let (out, _) = run("/bin/sh", "sh");
        assert_eq!(
            parse_resolved_binary(out)
                .as_deref()
                .map(|p| p.ends_with("/sh")),
            Some(true)
        );
        // A shell that never exits is killed by the host-side watchdog (8 s), not left running.
        let dir = std::env::temp_dir().join(format!("ssh-app-hang-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("hangshell");
        let pidfile = dir.join("pid");
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\necho $$ > {}\nexec sleep 60\n",
                pidfile.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let (out, took) = run(fake.to_str().unwrap(), "sh");
        assert!(parse_resolved_binary(out).is_none());
        assert!(took.as_secs() < 15, "{took:?}");
        let pid = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .to_string();
        let alive = Command::new("kill")
            .args(["-0", &pid])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(!alive, "hung probe shell {pid} was left running");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_resolved_binary_ignores_noise_and_aliases() {
        assert_eq!(
            parse_resolved_binary("banner\n/usr/local/bin/claude\n".into()),
            Some("/usr/local/bin/claude".into())
        );
        assert_eq!(parse_resolved_binary("alias claude='x'\n".into()), None);
        assert_eq!(parse_resolved_binary("".into()), None);
        assert_eq!(
            parse_resolved_binary("/a/b\n\u{1b}[0m/evil\u{7}\n".into()),
            Some("/a/b".into())
        );
    }

    #[test]
    fn resolved_binary_rejects_spoofing_and_oversize() {
        assert_eq!(
            parse_resolved_binary("/usr/bin/\u{202e}gnp.exe\n".into()),
            None
        );
        assert_eq!(parse_resolved_binary("/a\u{200b}b\n".into()), None);
        assert_eq!(
            parse_resolved_binary(format!("/{}\n", "a".repeat(5000))),
            None
        );
        assert_eq!(
            parse_resolved_binary("/usr/bin/é\n".into()),
            Some("/usr/bin/é".into())
        );
    }
}
