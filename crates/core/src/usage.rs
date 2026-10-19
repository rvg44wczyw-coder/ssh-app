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
use crate::hostprobe::{SHA256_OF_H, SHA256_OF_T};
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

/// State of the typed helper on the host, judged against the digest the app pins.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum HostHelperStatus {
    /// No helper file (or something that is not a regular file) at the install path.
    Absent,
    /// A file is there but its digest is not the pinned one (old version or replaced): never run.
    Mismatch,
    /// The pinned helper is installed and answered `hello`.
    Ready { version: String },
    /// The digest matches but the helper did not answer (wrong platform, damaged).
    Unusable,
}

/// SHA-256 of the embedded helper: the digest the app pins. `None` when no helper is embedded.
#[uniffi::export]
pub fn bundled_host_helper_sha256() -> Option<String> {
    bundled_host_helper_sha256_for(crate::hosttarget::HostTarget::DarwinArm64)
}

/// Size of the embedded helper in bytes (0 when none is embedded).
#[uniffi::export]
pub fn bundled_host_helper_size() -> u64 {
    bundled_host_helper_size_for(crate::hosttarget::HostTarget::DarwinArm64)
}

include!(concat!(env!("OUT_DIR"), "/helpers.rs"));

/// The embedded helper build for `target`, if the app was built with it.
pub(crate) fn bundled_helper_for(target: &crate::hosttarget::HostTarget) -> Option<&'static [u8]> {
    let triple = target.triple()?;
    HELPERS.iter().find(|(t, _)| *t == triple).map(|(_, b)| *b)
}

/// SHA-256 of the embedded helper build for `target`: the digest the app pins on that host.
/// `None` for an unsupported system or a build that is not embedded.
#[uniffi::export]
pub fn bundled_host_helper_sha256_for(target: crate::hosttarget::HostTarget) -> Option<String> {
    use sha2::{Digest, Sha256};
    bundled_helper_for(&target).map(|b| hex::encode(Sha256::digest(b)))
}

/// Size in bytes of the embedded helper build for `target` (0 when there is none).
#[uniffi::export]
pub fn bundled_host_helper_size_for(target: crate::hosttarget::HostTarget) -> u64 {
    bundled_helper_for(&target).map_or(0, |b| b.len() as u64)
}

pub(crate) fn bundled_host_helper() -> &'static [u8] {
    bundled_helper_for(&crate::hosttarget::HostTarget::DarwinArm64).unwrap_or(&[])
}

pub(crate) fn build_helper_status_command(sha256: &str) -> String {
    let script = format!(
        "h={HELPER_PATH_EXPR}; if [ ! -f \"$h\" ] || [ -L \"$h\" ]; then echo HELPER_ABSENT; \
         elif [ \"{SHA256_OF_H}\" != '{sha256}' ]; then echo HELPER_MISMATCH; \
         else \"$h\" hello 2>/dev/null || echo HELPER_UNUSABLE; fi"
    );
    crate::hostprobe::login_sh_script(&script, &[], 10)
}

pub(crate) fn parse_helper_status(out: &str) -> HostHelperStatus {
    for line in out.lines().rev() {
        let line = line.trim();
        match line {
            "HELPER_ABSENT" => return HostHelperStatus::Absent,
            "HELPER_MISMATCH" => return HostHelperStatus::Mismatch,
            "HELPER_UNUSABLE" => return HostHelperStatus::Unusable,
            _ => {}
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            if v.get("name").and_then(|n| n.as_str()) == Some("ssh-app-host") {
                let version = v
                    .get("version")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-')
                    .take(32)
                    .collect();
                return HostHelperStatus::Ready { version };
            }
        }
    }
    HostHelperStatus::Unusable
}

/// One agent session as the host agent sees it (feature 26).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HostAgentSession {
    pub agent: String,
    pub session: String,
    /// approval, finished, waiting or running.
    pub state: String,
    pub tmux: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HostAgentState {
    pub sessions: Vec<HostAgentSession>,
    pub pending_approvals: u32,
}

/// A change of one agent session's state, pushed live by the host agent.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HostAgentEvent {
    pub agent: String,
    pub session: String,
    pub from: String,
    pub to: String,
}

/// Asks the host agent for its state through `rpc`, which starts the agent if needed and renews
/// its lease (12 h). Runs only the pinned helper; `HELPER_ABSENT` otherwise.
pub(crate) fn build_agent_state_command(sha256: &str) -> String {
    let script = format!(
        "h={HELPER_PATH_EXPR}; if [ ! -f \"$h\" ] || [ -L \"$h\" ] || \
         [ \"{SHA256_OF_H}\" != '{sha256}' ]; then echo HELPER_ABSENT; \
         else printf '%s\\n' '{{\"id\":1,\"m\":\"state\"}}' | \"$h\" rpc --lease 43200 2>/dev/null; fi"
    );
    crate::hostprobe::login_sh_script(&script, &[], 15)
}

/// Long-lived `rpc` bridge for the live event stream: the channel's stdin carries requests and
/// the pinned helper answers on stdout until the channel closes. Same digest pin as above; the
/// reply is `HELPER_ABSENT` (and the channel ends) when the helper is missing or replaced.
pub(crate) fn build_agent_watch_command(sha256: &str) -> String {
    let script = format!(
        "h={HELPER_PATH_EXPR}; if [ ! -f \"$h\" ] || [ -L \"$h\" ] || \
         [ \"{SHA256_OF_H}\" != '{sha256}' ]; then echo HELPER_ABSENT; \
         else exec \"$h\" rpc --lease 43200 2>/dev/null; fi"
    );
    format!("sh -c {}", crate::tmux::shell_quote(&script))
}

/// The agent's reply to `subscribe` (request id 1).
pub(crate) fn is_subscribe_ack(line: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line.trim()).is_ok_and(|v| {
        v.get("id").and_then(|i| i.as_i64()) == Some(1) && v.get("ok") == Some(&true.into())
    })
}

/// A model the agent can be switched to (`/model <id>`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentModelChoice {
    pub id: String,
    pub label: String,
}

/// Models of one agent from the pinned helper (`models <agent>`); `HELPER_ABSENT` otherwise.
pub(crate) fn build_models_command(sha256: &str, agent: &str) -> Result<String, SshCoreError> {
    if !matches!(agent, "claude" | "antigravity" | "ollama") {
        return Err(SshCoreError::InvalidInput {
            reason: "Unknown agent for a model list".to_string(),
        });
    }
    let script = format!(
        "h={HELPER_PATH_EXPR}; if [ ! -f \"$h\" ] || [ -L \"$h\" ] || \
         [ \"{SHA256_OF_H}\" != '{sha256}' ]; then echo HELPER_ABSENT; \
         else \"$h\" models {agent} 2>/dev/null; fi"
    );
    Ok(crate::hostprobe::login_sh_script(&script, &[], 40))
}

/// The same lists without the helper (it may be missing or older than this app): the agent's own
/// command, run through the login shell with a fixed script.
pub(crate) fn build_models_direct_command(agent: &str) -> Option<String> {
    let script = match agent {
        "antigravity" => "a=$(command -v agy 2>/dev/null); [ -n \"$a\" ] || a=\"$HOME/.local/bin/agy\"; \"$a\" models",
        "ollama" => "ollama list",
        _ => return None,
    };
    Some(crate::hostprobe::login_sh_script(script, &[], 40))
}

/// Output of `agy models` (`id<TAB>label`) or `ollama list` (header, then `NAME ...`).
pub(crate) fn parse_models_direct(agent: &str, out: &str) -> Vec<AgentModelChoice> {
    let mut found: Vec<AgentModelChoice> = Vec::new();
    let mut add = |id: &str, label: &str| {
        if found.len() < 60 && is_typable_id(id) && !found.iter().any(|m| m.id == id) {
            found.push(AgentModelChoice {
                id: id.to_string(),
                label: label.chars().filter(|c| !c.is_control()).take(80).collect(),
            });
        }
    };
    if agent == "ollama" {
        for line in out.lines().skip(1) {
            if let Some(name) = line.split_whitespace().next() {
                add(name, name);
            }
        }
    } else {
        for line in out.lines() {
            if let Some((id, label)) = line.split_once('\t') {
                add(id.trim(), label.trim());
            }
        }
    }
    found
}

/// An id that may be typed into a terminal (same alphabet the helper enforces).
fn is_typable_id(id: &str) -> bool {
    let mut chars = id.chars();
    id.len() <= 100
        && chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || "._:/@-".contains(c))
}

/// `None` when the helper was not usable or answered nothing sensible. Ids the app could not type
/// safely are dropped here again, whatever the host said.
pub(crate) fn parse_models(out: &str) -> Option<Vec<AgentModelChoice>> {
    for line in out.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };
        if v.get("ok") != Some(&true.into()) {
            continue;
        }
        let models = v.get("models")?.as_array()?;
        return Some(
            models
                .iter()
                .take(60)
                .filter_map(|m| {
                    let id = m.get("id")?.as_str()?;
                    let label = m.get("label").and_then(|l| l.as_str()).unwrap_or(id);
                    is_typable_id(id).then(|| AgentModelChoice {
                        id: id.to_string(),
                        label: label.chars().filter(|c| !c.is_control()).take(80).collect(),
                    })
                })
                .collect(),
        );
    }
    None
}

/// One state change pushed by the agent (`None` for any other line).
pub(crate) fn parse_agent_event(line: &str) -> Option<HostAgentEvent> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    if v.get("event")?.as_str()? != "state" {
        return None;
    }
    let text =
        |k: &str| -> Option<String> { Some(v.get(k)?.as_str()?.chars().take(512).collect()) };
    Some(HostAgentEvent {
        agent: text("agent")?,
        session: text("session")?,
        from: text("from")?,
        to: text("to")?,
    })
}

/// `None` when the helper is not usable (the reply says so) or the agent did not answer.
pub(crate) fn parse_agent_state(out: &str) -> Option<HostAgentState> {
    for line in out.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };
        if v.get("id").and_then(|i| i.as_i64()) != Some(1) || v.get("ok") != Some(&true.into()) {
            continue;
        }
        let text = |x: &serde_json::Value, k: &str| {
            x.get(k)
                .and_then(|t| t.as_str())
                .map(|t| t.chars().take(512).collect::<String>())
        };
        let sessions = v
            .get("sessions")?
            .as_array()?
            .iter()
            .take(100)
            .filter_map(|s| {
                Some(HostAgentSession {
                    agent: text(s, "agent")?,
                    session: text(s, "session")?,
                    state: text(s, "state")?,
                    tmux: text(s, "tmux").filter(|t| !t.is_empty()),
                    cwd: text(s, "cwd").filter(|t| !t.is_empty()),
                })
            })
            .collect();
        let pending = v.get("pending").and_then(|p| p.as_u64()).unwrap_or(0);
        return Some(HostAgentState {
            sessions,
            pending_approvals: pending.min(1000) as u32,
        });
    }
    None
}

/// Commands that put `bytes` on the host as the helper: base64 chunks appended to a private part
/// file, then decoded, hashed against the pin and moved into place only when the digest matches.
pub(crate) fn build_helper_install_commands(
    bytes: &[u8],
    sha256: &str,
) -> Result<Vec<String>, SshCoreError> {
    use sha2::{Digest, Sha256};
    validate_sha256(sha256)?;
    if bytes.is_empty() || bytes.len() > 16 * 1024 * 1024 {
        return Err(SshCoreError::InvalidInput {
            reason: "The host helper is empty or too large".into(),
        });
    }
    if hex::encode(Sha256::digest(bytes)) != sha256 {
        return Err(SshCoreError::InvalidInput {
            reason: "The host helper does not match its digest".into(),
        });
    }
    let dir = "\"$HOME/.ssh-app/bin\"";
    let part = "\"$HOME/.ssh-app/bin/.ssh-app-host.b64\"";
    let mut cmds = vec![];
    for (i, chunk) in bytes.chunks(48 * 1024).enumerate() {
        let b64 = base64_encode(chunk);
        let redirect = if i == 0 { ">" } else { ">>" };
        let prepare = if i == 0 {
            format!(
                "umask 077; mkdir -p \"$HOME/.ssh-app/run\" {dir} && chmod 700 \"$HOME/.ssh-app\" \"$HOME/.ssh-app/run\" {dir}; "
            )
        } else {
            "umask 077; ".to_string()
        };
        cmds.push(format!("{prepare}printf %s '{b64}' {redirect} {part}"));
    }
    cmds.push(format!(
        "set -e; umask 077; cd {dir}; t=.ssh-app-host.tmp; rm -f \"$t\"; \
         (base64 -d < .ssh-app-host.b64 > \"$t\" 2>/dev/null || base64 -D < .ssh-app-host.b64 > \"$t\"); rm -f .ssh-app-host.b64; \
         [ \"{SHA256_OF_T}\" = '{sha256}' ] || {{ rm -f \"$t\"; echo HELPER_DIGEST_MISMATCH; exit 3; }}; \
         chmod 700 \"$t\"; mv -f \"$t\" ssh-app-host; echo HELPER_INSTALLED"
    ));
    Ok(cmds)
}

fn base64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Where the app installs the typed helper on the host (feature 24).
pub(crate) const HELPER_PATH_EXPR: &str = "\"$HOME/.ssh-app/bin/ssh-app-host\"";

/// A SHA-256 digest as the app pins it: 64 lowercase hex characters.
pub(crate) fn validate_sha256(s: &str) -> Result<(), SshCoreError> {
    if s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Ok(())
    } else {
        Err(SshCoreError::InvalidInput {
            reason: "Invalid SHA-256 digest".into(),
        })
    }
}

/// Shell command for the probe under the login shell and a 30 s watchdog (the probe keeps its own
/// 15 s scan budget). With `helper_sha256` the installed `ssh-app-host` is used when it is a
/// regular file (not a link) whose digest equals the pin and it answers; anything else falls
/// through to the embedded Python probe, so usage never depends on the helper.
pub(crate) fn build_usage_command(dirs: &[String], helper_sha256: Option<&str>) -> String {
    let mut args = vec![crate::tmux::shell_quote(USAGE_PROBE_PY)];
    for d in dirs {
        args.push(crate::agents::remote_path_expr(d.trim()));
    }
    let helper = match helper_sha256.filter(|h| validate_sha256(h).is_ok()) {
        Some(pin) => format!(
            "h={HELPER_PATH_EXPR}; if [ -f \"$h\" ] && [ ! -L \"$h\" ] && \
             [ \"{SHA256_OF_H}\" = '{pin}' ]; then \
             o=$(\"$h\" usage --agent \"$@\" 2>/dev/null) && [ -n \"$o\" ] && {{ printf '%s\\n' \"$o\"; exit 0; }}; fi; "
        ),
        None => String::new(),
    };
    // `$1` is the Python source, the rest are the config directories.
    let script = format!("src=$1; shift; {helper}exec python3 -c \"$src\" \"$@\"");
    crate::hostprobe::login_sh_script(&script, &args, 30)
}

pub(crate) const USAGE_PROBE_PY: &str = include_str!("usage_probe.py");

#[cfg(test)]
mod tests {
    #[test]
    fn helper_table_is_keyed_by_target() {
        use crate::hosttarget::HostTarget;
        let unsupported = HostTarget::Unsupported { reason: "x".into() };
        assert_eq!(bundled_host_helper_sha256_for(unsupported.clone()), None);
        assert_eq!(bundled_host_helper_size_for(unsupported), 0);
        // Whatever is embedded is found under its own triple and the sizes agree.
        for (triple, bytes) in HELPERS {
            let t = match *triple {
                "aarch64-apple-darwin" => HostTarget::DarwinArm64,
                "x86_64-apple-darwin" => HostTarget::DarwinX86_64,
                "x86_64-unknown-linux-musl" => HostTarget::LinuxX86_64,
                "x86_64-pc-windows-gnu" => HostTarget::WindowsX86_64,
                _ => HostTarget::LinuxAarch64,
            };
            assert_eq!(bundled_host_helper_size_for(t.clone()), bytes.len() as u64);
            assert_eq!(bundled_host_helper_sha256_for(t).map(|h| h.len()), Some(64));
        }
        // The legacy API is the arm64 macOS build.
        if let Some(legacy) = bundled_host_helper_sha256() {
            assert_eq!(
                bundled_host_helper_sha256_for(HostTarget::DarwinArm64),
                Some(legacy)
            );
        }
    }

    use super::*;

    #[test]
    fn parses_agent_state_and_ignores_noise() {
        let out = "banner\n{\"id\":0,\"ok\":true,\"lease_until\":5}\n{\"id\":1,\"ok\":true,\"pending\":2,\"sessions\":[{\"agent\":\"claude\",\"session\":\"s1\",\"state\":\"approval\",\"tmux\":\"\",\"cwd\":\"/w\"},{\"agent\":1}]}\n";
        let st = parse_agent_state(out).unwrap();
        assert_eq!(st.pending_approvals, 2);
        assert_eq!(st.sessions.len(), 1);
        assert_eq!(
            (st.sessions[0].tmux.clone(), st.sessions[0].cwd.clone()),
            (None, Some("/w".into()))
        );
        assert!(parse_agent_state("HELPER_ABSENT\n").is_none());
        assert!(build_agent_state_command(&"a".repeat(64)).contains("rpc --lease 43200"));
    }

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
    fn the_embedded_helper_is_either_absent_or_pinned_by_its_own_digest() {
        match bundled_host_helper_sha256() {
            None => assert_eq!(bundled_host_helper_size(), 0),
            Some(sha) => {
                assert!(validate_sha256(&sha).is_ok());
                assert!(build_helper_install_commands(bundled_host_helper(), &sha).is_ok());
            }
        }
    }

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn usage_command_uses_the_helper_only_with_a_valid_pin() {
        let dirs = vec!["~/.claude".to_string()];
        let pin = "a".repeat(64);
        let with = build_usage_command(&dirs, Some(&pin));
        assert!(with.contains("ssh-app-host") && with.contains(&pin) && with.contains("python3"));
        for bad in ["", "xyz", &"A".repeat(64), &"a".repeat(63)] {
            assert!(
                !build_usage_command(&dirs, Some(bad)).contains("ssh-app-host"),
                "{bad}"
            );
        }
        assert!(!build_usage_command(&dirs, None).contains("ssh-app-host"));
    }

    #[test]
    fn helper_status_parses_every_answer() {
        assert_eq!(
            parse_helper_status("HELPER_ABSENT\n"),
            HostHelperStatus::Absent
        );
        assert_eq!(
            parse_helper_status("noise\nHELPER_MISMATCH"),
            HostHelperStatus::Mismatch
        );
        assert_eq!(
            parse_helper_status("HELPER_UNUSABLE"),
            HostHelperStatus::Unusable
        );
        assert_eq!(
            parse_helper_status("banner\n{\"v\":1,\"name\":\"ssh-app-host\",\"version\":\"0.1.0\",\"caps\":[\"usage\"]}\n"),
            HostHelperStatus::Ready { version: "0.1.0".into() }
        );
        assert_eq!(
            parse_helper_status("{\"name\":\"other\"}"),
            HostHelperStatus::Unusable
        );
        assert_eq!(parse_helper_status(""), HostHelperStatus::Unusable);
    }

    #[test]
    fn install_commands_check_the_digest_and_chunk_the_upload() {
        use sha2::{Digest, Sha256};
        let bytes = vec![7u8; 100_000];
        let sha = hex::encode(Sha256::digest(&bytes));
        let cmds = build_helper_install_commands(&bytes, &sha).unwrap();
        assert_eq!(
            cmds.len(),
            3 + 1,
            "3 chunks of 48 KiB and the finishing command"
        );
        assert!(cmds[0].contains("> \"$HOME/.ssh-app/bin/.ssh-app-host.b64\""));
        assert!(cmds[1].contains(">> "));
        assert!(cmds.last().unwrap().contains(&sha));
        assert!(cmds.iter().all(|c| c.len() < 70_000));
        // Wrong digest, empty and oversized input are refused before anything is sent.
        assert!(build_helper_install_commands(&bytes, &"0".repeat(64)).is_err());
        assert!(build_helper_install_commands(&[], &sha).is_err());
        assert!(build_helper_install_commands(&bytes, "nothex").is_err());
    }

    #[test]
    fn direct_model_lists_without_the_helper() {
        let agy =
            "Fetching available models...\ngemini-4-argon-low\tGemini 4 Argon (Low)\nbad id\tx\n";
        let m = parse_models_direct("antigravity", agy);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].label, "Gemini 4 Argon (Low)");
        let oll = "NAME ID SIZE MODIFIED\nllama3:8b abc 4 GB now\n";
        assert_eq!(parse_models_direct("ollama", oll)[0].id, "llama3:8b");
        assert!(build_models_direct_command("claude").is_none());
        assert!(build_models_direct_command("antigravity").is_some());
    }

    #[test]
    fn model_lists_are_parsed_and_unsafe_ids_dropped() {
        let out = r#"{"ok":true,"agent":"claude","models":[{"id":"opus","label":"Opus"},{"id":"x; rm -rf ~","label":"evil"},{"id":"-x","label":"flag"},{"id":"claude-opus-5-5"}]}"#;
        let m = parse_models(out).unwrap();
        assert_eq!(
            m.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["opus", "claude-opus-5-5"]
        );
        assert_eq!(m[1].label, "claude-opus-5-5");
        assert!(parse_models("HELPER_ABSENT").is_none());
        assert!(build_models_command(&"a".repeat(64), "claude").is_ok());
        assert!(build_models_command(&"a".repeat(64), "claude; id").is_err());
    }

    #[test]
    fn agent_stream_lines_are_told_apart() {
        assert!(is_subscribe_ack(r#"{"id":1,"ok":true}"#));
        assert!(!is_subscribe_ack(r#"{"id":1,"ok":false}"#));
        assert!(!is_subscribe_ack("HELPER_ABSENT"));
        let ev = parse_agent_event(
            r#"{"agent":"claude","event":"state","from":"running","session":"s1","to":"approval"}"#,
        )
        .unwrap();
        assert_eq!((ev.session.as_str(), ev.to.as_str()), ("s1", "approval"));
        assert!(parse_agent_event(r#"{"id":0,"lease_until":1,"ok":true}"#).is_none());
        let cmd = build_agent_watch_command(&"a".repeat(64));
        assert!(cmd.contains("exec ") && cmd.contains("rpc --lease"));
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
