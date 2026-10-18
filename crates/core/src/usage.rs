//! Usage and limits of the models running in tmux sessions on the host.
//!
//! One read-only probe (Python 3 standard library, run on the host under a watchdog) collects:
//! * per tmux session: which agent runs in it (found through the pane's process tree), and for
//!   Claude Code the context size of the latest turn from the session transcript, plus a
//!   "limit reached" notice if the transcript carries one;
//! * for Ollama sessions: what the server reports as loaded (`/api/ps`);
//! * rolling token totals for the last 5 hours and 7 days from Claude Code transcripts.
//!
//! Nothing here is the subscription's real remaining quota: that number is not exposed to a
//! non-interactive reader. The totals are what the transcripts on this host record.

use crate::error::SshCoreError;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OllamaLoad {
    pub model: String,
    pub size_bytes: u64,
    pub vram_bytes: u64,
    pub context_length: Option<u64>,
    /// When the server unloads the model if it stays idle (epoch seconds).
    pub expires_epoch: Option<u64>,
}

/// One plan-limit window as the agent itself reports it ("5h" 42 %).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LimitWindow {
    pub label: String,
    pub percent: u32,
    pub resets_epoch: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SessionUsage {
    pub session: String,
    /// "claude", "codex", "agy", "grok" or "ollama"; `None` when no model runs in the session.
    pub agent: Option<String>,
    pub model: Option<String>,
    pub context_used: Option<u64>,
    pub context_limit: Option<u64>,
    /// Text of a "limit reached" notice found in the recent transcript.
    pub limit_note: Option<String>,
    pub limit_reset_epoch: Option<u64>,
    pub ollama: Option<OllamaLoad>,
    /// Real plan limits (percent used, reset time); empty when the agent does not expose them.
    pub limits: Vec<LimitWindow>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct UsageWindow {
    pub input: u64,
    pub output: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub messages: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HostUsageReport {
    pub sessions: Vec<SessionUsage>,
    pub five_hour: UsageWindow,
    pub week: UsageWindow,
    /// The scan hit its time budget: the totals are a lower bound.
    pub partial: bool,
}

#[derive(Deserialize, Default)]
struct RawWindow {
    #[serde(default)]
    input: u64,
    #[serde(default)]
    output: u64,
    #[serde(default)]
    cache_write: u64,
    #[serde(default)]
    cache_read: u64,
    #[serde(default)]
    messages: u64,
}

#[derive(Deserialize)]
struct RawOllama {
    #[serde(default)]
    name: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    size_vram: u64,
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default)]
    expires_epoch: Option<u64>,
}

#[derive(Deserialize)]
struct RawSession {
    session: String,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    context_used: Option<u64>,
    #[serde(default)]
    context_limit: Option<u64>,
    #[serde(default)]
    limit_note: Option<String>,
    #[serde(default)]
    limit_reset_epoch: Option<u64>,
    #[serde(default)]
    ollama_model: Option<String>,
    #[serde(default)]
    limits: Vec<RawLimit>,
}

#[derive(Deserialize)]
struct RawLimit {
    #[serde(default)]
    label: String,
    #[serde(default)]
    percent: u32,
    #[serde(default)]
    resets_epoch: Option<u64>,
}

#[derive(Deserialize, Default)]
struct RawReport {
    #[serde(default)]
    sessions: Vec<RawSession>,
    #[serde(default)]
    ollama: Vec<RawOllama>,
    #[serde(default)]
    five_hour: RawWindow,
    #[serde(default)]
    week: RawWindow,
    #[serde(default)]
    partial: bool,
}

impl From<RawWindow> for UsageWindow {
    fn from(w: RawWindow) -> Self {
        Self {
            input: w.input,
            output: w.output,
            cache_write: w.cache_write,
            cache_read: w.cache_read,
            messages: w.messages,
        }
    }
}

/// Text outside the JSON object (a login shell may print banners) is ignored: the probe's result
/// is the last line that starts with `{`.
pub(crate) fn parse_usage_report(output: &str) -> Result<HostUsageReport, SshCoreError> {
    let line = output
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.starts_with('{'))
        .ok_or_else(|| SshCoreError::IoError {
            reason: if output.contains("python3") && output.contains("not found") {
                "python3 is not installed on the host".to_string()
            } else {
                "The host did not return a usage report".to_string()
            },
        })?;
    let raw: RawReport = serde_json::from_str(line).map_err(|e| SshCoreError::IoError {
        reason: format!("Unreadable usage report: {e}"),
    })?;
    let sessions = raw
        .sessions
        .into_iter()
        .filter(|s| crate::tmux::validate_session_name(&s.session).is_ok())
        .map(|s| {
            let ollama = s.ollama_model.as_deref().and_then(|m| {
                raw.ollama
                    .iter()
                    .find(|o| o.name == m || o.name.split(':').next() == Some(m))
                    .map(|o| OllamaLoad {
                        model: o.name.clone(),
                        size_bytes: o.size,
                        vram_bytes: o.size_vram,
                        context_length: o.context_length,
                        expires_epoch: o.expires_epoch,
                    })
            });
            SessionUsage {
                session: s.session,
                agent: s.agent,
                model: s.model.or(s.ollama_model),
                context_used: s.context_used,
                context_limit: s.context_limit,
                limit_note: s.limit_note.map(|n| n.chars().take(160).collect()),
                limit_reset_epoch: s.limit_reset_epoch,
                ollama,
                limits: s
                    .limits
                    .into_iter()
                    .filter(|l| !l.label.is_empty() && l.label.len() <= 8)
                    .take(4)
                    .map(|l| LimitWindow {
                        label: l.label,
                        percent: l.percent.min(100),
                        resets_epoch: l.resets_epoch,
                    })
                    .collect(),
            }
        })
        .collect();
    Ok(HostUsageReport {
        sessions,
        five_hour: raw.five_hour.into(),
        week: raw.week.into(),
        partial: raw.partial,
    })
}

/// Config directories whose `projects/` hold Claude Code transcripts (profile `CLAUDE_CONFIG_DIR`
/// values); empty means `~/.claude`.
pub(crate) fn validate_config_dirs(dirs: &[String]) -> Result<(), SshCoreError> {
    if dirs.len() > 8 {
        return Err(SshCoreError::InvalidInput {
            reason: "Too many config directories".into(),
        });
    }
    for d in dirs {
        if d.trim().is_empty() || d.len() > 200 || d.chars().any(char::is_control) {
            return Err(SshCoreError::InvalidInput {
                reason: format!("Invalid config directory '{d}'"),
            });
        }
    }
    Ok(())
}

/// Shell command for the probe: `python3 -c <script> <dirs...>` under the login shell and a
/// 30 s watchdog (the script keeps its own 15 s scan budget).
pub(crate) fn build_usage_command(dirs: &[String]) -> String {
    let mut args = vec![crate::tmux::shell_quote(USAGE_PROBE_PY)];
    for d in dirs {
        args.push(crate::agents::remote_path_expr(d.trim()));
    }
    let script = "exec python3 -c \"$@\"";
    crate::hostprobe::login_sh_script(script, &args, 30)
}

pub(crate) const USAGE_PROBE_PY: &str = include_str!("usage_probe.py");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_report_and_ignores_banner_noise() {
        let out = "welcome\n{\"sessions\":[{\"session\":\"claude-1\",\"agent\":\"claude\",\"model\":\"claude-x\",\"context_used\":142000,\"context_limit\":200000},{\"session\":\"llm-1\",\"agent\":\"ollama\",\"ollama_model\":\"qwen2.5-coder\"},{\"session\":\"bad name\",\"agent\":null}],\"ollama\":[{\"name\":\"qwen2.5-coder:7b\",\"size\":5000,\"size_vram\":4000,\"context_length\":8192,\"expires_epoch\":99}],\"five_hour\":{\"input\":1,\"output\":2,\"cache_write\":3,\"cache_read\":4,\"messages\":5},\"week\":{},\"partial\":true}\n";
        let r = parse_usage_report(out).unwrap();
        assert_eq!(r.sessions.len(), 2, "invalid session name is dropped");
        assert_eq!(r.sessions[0].context_used, Some(142000));
        let o = r.sessions[1].ollama.as_ref().unwrap();
        assert_eq!((o.model.as_str(), o.vram_bytes), ("qwen2.5-coder:7b", 4000));
        assert_eq!(r.five_hour.cache_read, 4);
        assert_eq!(r.week, UsageWindow::default());
        assert!(r.partial);
    }

    #[test]
    fn parses_plan_limits_and_clamps_them() {
        let out = r#"{"sessions":[{"session":"claude-1","agent":"claude","limits":[{"label":"5h","percent":42,"resets_epoch":1900000000},{"label":"7d","percent":250},{"label":"waytoolonglabel","percent":1}]}]}"#;
        let r = parse_usage_report(out).unwrap();
        let l = &r.sessions[0].limits;
        assert_eq!(l.len(), 2);
        assert_eq!(
            (l[0].label.as_str(), l[0].percent, l[0].resets_epoch),
            ("5h", 42, Some(1900000000))
        );
        assert_eq!(l[1].percent, 100);
    }

    #[test]
    fn missing_output_is_a_typed_error() {
        assert!(parse_usage_report("").is_err());
        let e = parse_usage_report("sh: python3: command not found").unwrap_err();
        assert!(e.to_string().contains("python3"));
    }

    #[test]
    fn config_dirs_are_validated() {
        assert!(validate_config_dirs(&["~/.claude-personal".into()]).is_ok());
        assert!(validate_config_dirs(&["a\nb".into()]).is_err());
        assert!(validate_config_dirs(&vec!["x".to_string(); 9]).is_err());
    }
}
