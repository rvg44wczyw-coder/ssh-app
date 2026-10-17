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
    Ok(())
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
    for a in &profile.args {
        inner.push_str(&format!(" {}", shell_quote(a)));
    }
    if profile.load_shell_rc {
        cmd.push_str(&format!(
            "\"${{SHELL:-/bin/zsh}}\" -i -c {}",
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
            "env CLAUDE_CONFIG_DIR=\"$HOME\"'/.claude-personal' claude; exec \"${SHELL:-/bin/zsh}\" -l"
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
            "cd -- \"$HOME\"'/app' && { git switch 'feature/x-1' || echo \"ssh-app: could not switch to branch \"'feature/x-1'; } && env claude"
        ));
        for bad in ["-c", "a b", "x;rm", "a..b", "$(id)", "/x", "x/", "a.lock"] {
            let mut p = profile(AgentKind::ClaudeCode, "claude", &[]);
            p.git_branch = Some(bad.into());
            assert!(validate_launch_profile(p).is_err(), "{bad}");
        }
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
        assert_eq!(d.len(), 4);
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
        assert!(cmd.starts_with("\"${SHELL:-/bin/zsh}\" -i -c '"), "{cmd}");
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
