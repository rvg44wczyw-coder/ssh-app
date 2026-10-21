use crate::error::SshCoreError;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

/// Risk level classification for an AI-generated shell command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
pub enum AiRiskLevel {
    /// Read-only or benign operations (ls, grep, cat, docker ps, git status).
    Safe,
    /// Requires elevated root/admin privileges (sudo, doas, chown, chmod).
    Elevated,
    /// Modifies files, services, or repository state (git checkout, kill, systemctl stop).
    Caution,
    /// High-risk or irreversible destructive operations (rm -rf, dd, mkfs, git reset --hard, dropdb).
    Destructive,
}

/// Request parameters for generating a shell command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct AiCommandRequest {
    /// Natural language prompt from the user (e.g. "find all files modified today").
    pub user_prompt: String,
    /// Operating system of the remote host ("macOS", "Linux", "Unknown").
    pub target_os: String,
    /// Remote login shell ("zsh", "bash", "fish", "sh").
    pub shell_name: String,
    /// Current working directory if known (e.g. "~/projects/backend").
    pub cwd: Option<String>,
    /// Recent terminal output lines to provide context (max 30 lines).
    pub terminal_context: Vec<String>,
}

/// Verified command suggestion returned to mobile UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct AiCommandSuggestion {
    /// Clean, ready-to-run shell command line string.
    pub command: String,
    /// Brief explanation of how the command works and what flags mean.
    pub explanation: String,
    /// Risk assessment level.
    pub risk_level: AiRiskLevel,
    /// Specific risk warnings if applicable (e.g. "Contains recursive deletion flag -r").
    pub warnings: Vec<String>,
}

/// Result of deterministic command risk analysis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct AiRiskAssessment {
    pub level: AiRiskLevel,
    pub warnings: Vec<String>,
}

/// Assembles the system prompt instructing the LLM to output a single tailored command.
#[uniffi::export]
pub fn assemble_ai_system_prompt(target_os: &str, shell_name: &str, cwd: Option<String>) -> String {
    let os_desc = match target_os.to_lowercase().as_str() {
        "macos" | "darwin" => "macOS (BSD coreutils)",
        "linux" => "Linux (GNU coreutils)",
        _ => target_os,
    };
    let cwd_line = match cwd {
        Some(c) if !c.trim().is_empty() => format!("\nCurrent working directory: {}", c.trim()),
        _ => String::new(),
    };

    format!(
        r#"You are an expert command-line assistant for {os_desc} running {shell_name}.{cwd_line}
The user will ask for a terminal command. Output EXACTLY ONE shell command to achieve the goal.

Format requirements:
COMMAND: <single executable shell command or pipeline>
EXPLANATION: <brief 1-2 sentence explanation of arguments and flags>

Rules:
- Do not output markdown fences or code blocks.
- Do not invent non-existent flags.
- Prefer non-interactive flags where safe.
- Tailor flags to {os_desc}."#
    )
}

/// Assembles the user prompt incorporating recent terminal context if available.
#[uniffi::export]
pub fn assemble_ai_user_prompt(request: AiCommandRequest) -> String {
    let mut prompt = String::new();
    if !request.terminal_context.is_empty() {
        prompt.push_str("Recent terminal context:\n```\n");
        // Take at most the last 20 lines to keep prompt concise
        let start_idx = request.terminal_context.len().saturating_sub(20);
        for line in &request.terminal_context[start_idx..] {
            prompt.push_str(line);
            prompt.push('\n');
        }
        prompt.push_str("```\n\n");
    }
    prompt.push_str(&format!("Task: {}", request.user_prompt.trim()));
    prompt
}

/// Strips markdown fences, backticks, prompt markers ($), and whitespace from a shell command string.
#[uniffi::export]
pub fn sanitize_shell_command(command: &str) -> String {
    let mut cleaned = command.trim().to_string();

    // Strip triple backticks block ```bash ... ```
    if cleaned.starts_with("```") {
        if let Some(first_nl) = cleaned.find('\n') {
            cleaned = cleaned[first_nl + 1..].to_string();
        }
        if let Some(last_ticks) = cleaned.rfind("```") {
            cleaned = cleaned[..last_ticks].to_string();
        }
        cleaned = cleaned.trim().to_string();
    }

    // Strip inline backticks: `command` -> command
    if cleaned.starts_with('`') && cleaned.ends_with('`') && cleaned.len() >= 2 {
        cleaned = cleaned[1..cleaned.len() - 1].trim().to_string();
    }

    // Strip leading '$ ' prompt symbol
    if cleaned.starts_with("$ ") {
        cleaned = cleaned[2..].trim().to_string();
    }

    // Remove carriage returns
    cleaned = cleaned.replace('\r', "");
    cleaned.trim().to_string()
}

type RiskTier = (AiRiskLevel, Vec<(regex::Regex, &'static str)>);

/// Rules per tier, strictest first, compiled once per process.
static RISK_TIERS: LazyLock<Vec<RiskTier>> = LazyLock::new(|| {
    let tier = |level, rules: &[(&'static str, &'static str)]| -> RiskTier {
        let compiled = rules
            .iter()
            .map(|(pattern, desc)| (regex::Regex::new(pattern).expect("static risk rule"), *desc))
            .collect();
        (level, compiled)
    };
    vec![
        tier(
            AiRiskLevel::Destructive,
            &[
                (
                    r"\brm\s+-[a-zA-Z]*[rR][a-zA-Z]*[fF]?\b|\brm\s+-[a-zA-Z]*[fF][a-zA-Z]*[rR]\b",
                    "Recursive file deletion (rm -rf)",
                ),
                (
                    r"\bmkfs\b|\bfdisk\b|\bparted\b",
                    "Disk or filesystem partitioning/formatting",
                ),
                (r"\bdd\s+if=", "Raw block copy (dd)"),
                (
                    r">\s*/dev/(sd[a-z]|nvme|disk)",
                    "Direct write to raw disk device",
                ),
                (
                    r"\bgit\s+reset\s+--hard\b",
                    "Hard git reset discards uncommitted changes",
                ),
                (
                    r"\bgit\s+clean\s+-[a-zA-Z]*f[a-zA-Z]*\b",
                    "Force git clean permanently removes untracked files",
                ),
                (
                    r"(?i)\bdrop\s+(database|table)\b|\btruncate\s+table\b",
                    "Destructive database DROP/TRUNCATE statement",
                ),
                (
                    r"\bkill\s+-9\s+-1\b|\bkillall\s+-9\b",
                    "Unconditional mass process termination",
                ),
                (r":\(\)\s*\{\s*:\|:&\s*\};:", "Fork bomb DoS pattern"),
            ],
        ),
        tier(
            AiRiskLevel::Elevated,
            &[
                (
                    r"\bsudo\s+|\bdoas\s+|\bsu\s+",
                    "Requires root/superuser privileges",
                ),
                (
                    r"\bchown\s+-[a-zA-Z]*R\b|\bchmod\s+777\b|\bchmod\s+-[a-zA-Z]*R\b",
                    "Recursive or permissive permission modification",
                ),
                (
                    r"\bsystemctl\s+(stop|disable|restart|mask)\b",
                    "Modifies system service state",
                ),
                (
                    r"\biptables\b|\bufw\s+disable\b|\bpfctl\b",
                    "Modifies firewall rules",
                ),
            ],
        ),
        tier(
            AiRiskLevel::Caution,
            &[
                (r"\bkill\s+|\bpkill\s+", "Terminates running processes"),
                (
                    r"\bdocker\s+(rm|rmi|stop|system\s+prune)\b",
                    "Removes or stops Docker containers/images",
                ),
                (
                    r"\bgit\s+(checkout|restore)\s+\.",
                    "Discards uncommitted working tree changes",
                ),
                (
                    r"\bgit\s+push\s+.*(-f|--force)\b",
                    "Force push overwrites remote git history",
                ),
                (
                    r"\b(npm|pnpm|yarn|pip|cargo)\s+uninstall\b",
                    "Uninstalls packages",
                ),
                (
                    r"[^0-9&]>\s*[^&>]",
                    "Overwrites file content with stdout redirect (>)",
                ),
            ],
        ),
    ]
});

/// Deterministically evaluates a shell command against safety rules and returns its risk level and warnings.
#[uniffi::export]
pub fn classify_command_risk(command: &str) -> AiRiskAssessment {
    let cmd = command.trim();
    for (level, rules) in RISK_TIERS.iter() {
        let warnings: Vec<String> = rules
            .iter()
            .filter(|(re, _)| re.is_match(cmd))
            .map(|(_, desc)| (*desc).to_string())
            .collect();
        if !warnings.is_empty() {
            return AiRiskAssessment {
                level: *level,
                warnings,
            };
        }
    }
    AiRiskAssessment {
        level: AiRiskLevel::Safe,
        warnings: Vec::new(),
    }
}

/// Parses the raw LLM output into an AiCommandSuggestion with safety classification.
#[uniffi::export]
pub fn parse_ai_response(raw_llm_response: &str) -> Result<AiCommandSuggestion, SshCoreError> {
    let text = raw_llm_response.trim();
    if text.is_empty() {
        return Err(SshCoreError::AiResponseParseError {
            reason: "LLM response is empty".to_string(),
        });
    }

    let mut parsed_command = String::new();
    let mut parsed_explanation = String::new();

    // Strategy 1: Check for COMMAND: and EXPLANATION: format
    let mut found_command_prefix = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(cmd_part) = trimmed.strip_prefix("COMMAND:") {
            parsed_command = cmd_part.trim().to_string();
            found_command_prefix = true;
        } else if let Some(exp_part) = trimmed.strip_prefix("EXPLANATION:") {
            parsed_explanation = exp_part.trim().to_string();
        } else if found_command_prefix && parsed_explanation.is_empty() && !trimmed.is_empty() {
            // Continuation of explanation if EXPLANATION: was omitted or on next line
            if parsed_explanation.is_empty() {
                parsed_explanation = trimmed.to_string();
            }
        }
    }

    // Strategy 2: If no COMMAND: prefix found, extract from markdown code fence or first line
    if parsed_command.is_empty() {
        if let Some(start_fence) = text.find("```") {
            let after_fence = &text[start_fence + 3..];
            let code_start = after_fence.find('\n').map(|i| i + 1).unwrap_or(0);
            let rest = &after_fence[code_start..];
            if let Some(end_fence) = rest.find("```") {
                parsed_command = rest[..end_fence].trim().to_string();
                let explanation_part = rest[end_fence + 3..].trim();
                if !explanation_part.is_empty() {
                    parsed_explanation = explanation_part.to_string();
                }
            }
        }
    }

    // Strategy 3: Inline backticks or plain text first line
    if parsed_command.is_empty() {
        let mut lines = text.lines().map(|l| l.trim()).filter(|l| !l.is_empty());
        if let Some(first_line) = lines.next() {
            parsed_command = first_line.to_string();
            parsed_explanation = lines.collect::<Vec<_>>().join(" ");
        }
    }

    parsed_command = sanitize_shell_command(&parsed_command);

    if parsed_command.is_empty() {
        return Err(SshCoreError::AiResponseParseError {
            reason: "Could not extract a valid shell command from LLM response".to_string(),
        });
    }

    if parsed_explanation.is_empty() {
        parsed_explanation = "Executes the requested shell operation.".to_string();
    }

    let assessment = classify_command_risk(&parsed_command);

    Ok(AiCommandSuggestion {
        command: parsed_command,
        explanation: parsed_explanation,
        risk_level: assessment.level,
        warnings: assessment.warnings,
    })
}

/// What the host has: whether Ollama is installed, whether its server answers on the default
/// loopback endpoint, and which models it lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct OllamaHostStatus {
    pub installed: bool,
    pub binary_path: Option<String>,
    pub version: Option<String>,
    pub running: bool,
    pub models: Vec<String>,
}

const OLLAMA_PROBE_SCRIPT: &str = r#"p=$(command -v ollama 2>/dev/null)
case "$p" in *[![:print:]]*) p="";; esac
if [ -z "$p" ]; then
  for c in /opt/homebrew/bin/ollama /usr/local/bin/ollama /Applications/Ollama.app/Contents/Resources/ollama; do
    if [ -x "$c" ]; then p="$c"; break; fi
  done
fi
if [ -n "$p" ]; then
  printf 'OLLAMA_PATH=%s\n' "$p"
  v=$("$p" --version 2>&1 | head -c 4096 | grep 'version is' | tail -n 1)
  if [ -n "$v" ]; then printf 'OLLAMA_VERSION=%s\n' "$v"; fi
fi
t=$(curl -s -m 2 http://127.0.0.1:11434/api/tags 2>/dev/null | head -c 262144 | tr -d '\n\r')
if [ -n "$t" ]; then printf 'OLLAMA_TAGS=%s\n' "$t"; fi
true"#;

/// Fixed, read-only host command that prints `OLLAMA_PATH=`, `OLLAMA_VERSION=` and `OLLAMA_TAGS=`
/// lines (each only when known). It runs through the login shell (so PATH from `.zprofile` is
/// set) and then through POSIX `sh` (the login shell may be fish), under an 8 s host watchdog.
#[uniffi::export]
pub fn build_ollama_probe_command() -> String {
    crate::hostprobe::login_sh_script(OLLAMA_PROBE_SCRIPT, &[], 8)
}

/// Fast first look: only asks the loopback endpoint, in plain `sh` (no login shell, no
/// `ollama --version`), which answers in well under a second when the server is up. When it is
/// not, the full probe above runs to tell "not installed" from "not running".
pub(crate) fn build_ollama_quick_probe_command() -> String {
    const QUICK: &str = r#"t=$(curl -s -m 2 http://127.0.0.1:11434/api/tags 2>/dev/null | head -c 262144 | tr -d '\n\r')
if [ -n "$t" ]; then printf 'OLLAMA_TAGS=%s\n' "$t"; fi
true"#;
    format!("sh -c {}", crate::tmux::shell_quote(QUICK))
}

pub(crate) fn is_model_name(name: &str) -> bool {
    (1..=128).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '-'))
}

/// Parses the probe output. Host-controlled, so every field is validated: the last well-formed
/// value per marker wins, noise lines are ignored, and nothing can panic.
#[uniffi::export]
pub fn parse_ollama_probe(output: String) -> OllamaHostStatus {
    let mut binary_path = None;
    let mut version = None;
    let mut tags = None;
    for line in output.lines() {
        let line = line.trim();
        if let Some(p) = line.strip_prefix("OLLAMA_PATH=") {
            let spoofy = |c: char| {
                c.is_control()
                    || matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
            };
            if p.starts_with('/') && p.len() <= 4096 && !p.chars().any(spoofy) {
                binary_path = Some(p.to_string());
            }
        } else if let Some(v) = line.strip_prefix("OLLAMA_VERSION=") {
            if let Some((_, rest)) = v.split_once("version is") {
                let rest = rest.trim();
                if !rest.is_empty()
                    && rest.len() <= 64
                    && rest
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
                {
                    version = Some(rest.to_string());
                }
            }
        } else if let Some(t) = line.strip_prefix("OLLAMA_TAGS=") {
            tags = Some(t.to_string());
        }
    }

    let mut running = false;
    let mut models: Vec<String> = vec![];
    if let Some(body) = tags {
        if let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(&body)
        {
            if let Some(serde_json::Value::Array(items)) = map.get("models") {
                running = true;
                models = items
                    .iter()
                    .filter_map(|m| m.get("name").and_then(serde_json::Value::as_str))
                    .filter(|n| is_model_name(n))
                    .map(str::to_string)
                    .collect();
                models.sort();
                models.dedup();
                models.truncate(100);
            }
        }
    }
    OllamaHostStatus {
        installed: binary_path.is_some() || running,
        binary_path,
        version,
        running,
        models,
    }
}

/// Parses the JSON response from Ollama's /api/generate endpoint.
#[uniffi::export]
pub fn parse_ollama_generate_response(json_body: &str) -> Result<String, SshCoreError> {
    #[derive(Deserialize)]
    struct OllamaResponse {
        response: Option<String>,
        error: Option<String>,
    }

    let parsed: OllamaResponse =
        serde_json::from_str(json_body).map_err(|e| SshCoreError::AiAssistantError {
            reason: format!("Failed to parse Ollama JSON: {e}"),
        })?;

    if let Some(err) = parsed.error {
        return Err(SshCoreError::AiAssistantError {
            reason: format!("Ollama returned error: {err}"),
        });
    }

    parsed
        .response
        .ok_or_else(|| SshCoreError::AiAssistantError {
            reason: "Ollama response missing 'response' field".to_string(),
        })
}

/// Who said a turn of the local-assistant chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    User,
    Assistant,
}

/// One turn of the local-assistant chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

const CHAT_MAX_MESSAGES: usize = 60;
/// The request travels as one argument of a remote command line, so the whole history is capped.
const CHAT_MAX_TOTAL_BYTES: usize = 96 * 1024;

/// Builds the `/api/chat` body (non-streaming). The system prompt is sent as the first message;
/// only `user`/`assistant` turns are accepted from the caller, and the history must end with a
/// user turn.
pub fn build_ollama_chat_payload(
    model: &str,
    system_prompt: &str,
    messages: &[ChatMessage],
) -> Result<String, SshCoreError> {
    let bad = |reason: &str| SshCoreError::AiAssistantError {
        reason: reason.to_string(),
    };
    if model.trim().is_empty() {
        return Err(bad("No model selected"));
    }
    if messages.is_empty() || messages.len() > CHAT_MAX_MESSAGES {
        return Err(bad("Chat history is empty or too long"));
    }
    if messages.last().map(|m| m.role) != Some(ChatRole::User) {
        return Err(bad("Chat must end with a user message"));
    }
    let total: usize =
        messages.iter().map(|m| m.content.len()).sum::<usize>() + system_prompt.len();
    if total > CHAT_MAX_TOTAL_BYTES {
        return Err(bad("Chat is too long; start a new chat"));
    }
    let mut turns = vec![serde_json::json!({"role": "system", "content": system_prompt})];
    for m in messages {
        let role = match m.role {
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
        };
        turns.push(serde_json::json!({"role": role, "content": m.content}));
    }
    Ok(serde_json::json!({"model": model, "messages": turns, "stream": false}).to_string())
}

/// Parses the JSON response from Ollama's /api/chat endpoint into the assistant's text.
pub fn parse_ollama_chat_response(json_body: &str) -> Result<String, SshCoreError> {
    #[derive(Deserialize)]
    struct Msg {
        content: Option<String>,
    }
    #[derive(Deserialize)]
    struct Resp {
        message: Option<Msg>,
        error: Option<String>,
    }
    let parsed: Resp =
        serde_json::from_str(json_body).map_err(|e| SshCoreError::AiAssistantError {
            reason: format!("Failed to parse Ollama JSON: {e}"),
        })?;
    if let Some(err) = parsed.error {
        return Err(SshCoreError::AiAssistantError {
            reason: format!("Ollama returned error: {err}"),
        });
    }
    parsed
        .message
        .and_then(|m| m.content)
        .ok_or_else(|| SshCoreError::AiAssistantError {
            reason: "Ollama response missing 'message' field".to_string(),
        })
}

#[cfg(test)]
mod tests {
    #[test]
    fn ollama_probe_parses_the_documented_example() {
        let out = "OLLAMA_PATH=/opt/homebrew/bin/ollama\nOLLAMA_VERSION=ollama version is 0.12.3\n\
                   OLLAMA_TAGS={\"models\":[{\"name\":\"qwen2.5-coder:7b\"},{\"name\":\"llama3.2:latest\"}]}\n";
        let s = parse_ollama_probe(out.into());
        assert_eq!(
            s,
            OllamaHostStatus {
                installed: true,
                binary_path: Some("/opt/homebrew/bin/ollama".into()),
                version: Some("0.12.3".into()),
                running: true,
                models: vec!["llama3.2:latest".into(), "qwen2.5-coder:7b".into()],
            }
        );
    }

    #[test]
    fn ollama_probe_states() {
        let none = parse_ollama_probe(String::new());
        assert!(!none.installed && !none.running && none.models.is_empty());
        let down = parse_ollama_probe(
            "OLLAMA_PATH=/usr/local/bin/ollama\nOLLAMA_VERSION=Warning: client version is 0.9.0\n"
                .into(),
        );
        assert!(down.installed && !down.running);
        assert_eq!(down.version.as_deref(), Some("0.9.0"));
        let empty = parse_ollama_probe("OLLAMA_TAGS={\"models\":[]}".into());
        assert!(empty.running && empty.installed && empty.models.is_empty());
    }

    #[test]
    fn ollama_probe_ignores_noise_and_takes_the_last_marker() {
        let s = parse_ollama_probe(
            "welcome!\nOLLAMA_PATH=/first\nOLLAMA_PATH=/second/ollama\nzsh: banner\n".into(),
        );
        assert_eq!(s.binary_path.as_deref(), Some("/second/ollama"));
    }

    #[test]
    fn ollama_probe_rejects_hostile_values() {
        let long = "a".repeat(200);
        let many: Vec<String> = (0..150).map(|i| format!("{{\"name\":\"z{i}\"}}")).collect();
        let tags = format!(
            "OLLAMA_TAGS={{\"models\":[{{\"name\":\"ok:1\"}},{{\"name\":\"a; rm -rf ~\"}},{{\"name\":\"a b\"}},{{\"name\":\"{long}\"}},{{\"name\":\"ok:1\"}},{{\"name\":5}},{{}},{}]}}",
            many.join(",")
        );
        let s = parse_ollama_probe(tags);
        assert!(s.models.contains(&"ok:1".to_string()));
        assert!(s.models.iter().all(|m| is_model_name(m)));
        assert_eq!(s.models.iter().filter(|m| *m == "ok:1").count(), 1);
        assert_eq!(s.models.len(), 100);

        assert_eq!(
            parse_ollama_probe("OLLAMA_PATH=/usr/bin/\u{202e}gnp".into()).binary_path,
            None
        );
        assert_eq!(
            parse_ollama_probe("OLLAMA_PATH=relative/ollama".into()).binary_path,
            None
        );
        assert_eq!(
            parse_ollama_probe("OLLAMA_VERSION=version is $(id)".into()).version,
            None
        );
        assert!(!parse_ollama_probe("OLLAMA_TAGS={\"models\":[".into()).running);
        assert!(!parse_ollama_probe("OLLAMA_TAGS=[1,2]".into()).running);
        assert!(!parse_ollama_probe("OLLAMA_TAGS={\"models\":5}".into()).running);
    }

    #[test]
    fn quick_probe_is_a_constant_plain_sh_that_the_normal_parser_reads() {
        let c = build_ollama_quick_probe_command();
        assert!(c.starts_with("sh -c '"));
        assert!(c.contains("127.0.0.1:11434") && c.contains("curl -s -m 2"));
        assert!(!c.contains("-l -c"), "must not start a login shell");
        let st = parse_ollama_probe("OLLAMA_TAGS={\"models\":[{\"name\":\"llama3:8b\"}]}".into());
        assert!(st.running && st.installed && st.models == ["llama3:8b"]);
    }

    #[test]
    fn ollama_probe_command_is_a_constant_watchdogged_sh() {
        let c = build_ollama_probe_command();
        assert!(c.starts_with("sh -c '"));
        assert!(c.contains("kill -9") && c.contains("kt()"));
        for needle in [
            "OLLAMA_PATH",
            "OLLAMA_VERSION",
            "OLLAMA_TAGS",
            "/opt/homebrew/bin/ollama",
            "127.0.0.1:11434",
            "-l -c",
        ] {
            assert!(c.contains(needle), "missing {needle}");
        }
    }

    use super::*;

    #[test]
    fn test_assemble_ai_system_prompt() {
        let prompt_macos =
            assemble_ai_system_prompt("macOS", "zsh", Some("/Users/test".to_string()));
        assert!(prompt_macos.contains("macOS (BSD coreutils)"));
        assert!(prompt_macos.contains("zsh"));
        assert!(prompt_macos.contains("/Users/test"));

        let prompt_linux = assemble_ai_system_prompt("Linux", "bash", None);
        assert!(prompt_linux.contains("Linux (GNU coreutils)"));
        assert!(prompt_linux.contains("bash"));
        assert!(!prompt_linux.contains("Current working directory"));
    }

    #[test]
    fn test_assemble_ai_user_prompt_with_context() {
        let req = AiCommandRequest {
            user_prompt: "find large files".to_string(),
            target_os: "macOS".to_string(),
            shell_name: "zsh".to_string(),
            cwd: None,
            terminal_context: vec!["line 1".to_string(), "line 2".to_string()],
        };
        let user_prompt = assemble_ai_user_prompt(req);
        assert!(user_prompt.contains("Recent terminal context:"));
        assert!(user_prompt.contains("line 1"));
        assert!(user_prompt.contains("Task: find large files"));
    }

    #[test]
    fn test_sanitize_shell_command() {
        assert_eq!(sanitize_shell_command("  ls -la  "), "ls -la");
        assert_eq!(sanitize_shell_command("`ls -la`"), "ls -la");
        assert_eq!(sanitize_shell_command("$ ls -la"), "ls -la");
        assert_eq!(
            sanitize_shell_command("```bash\nfind . -name '*.rs'\n```"),
            "find . -name '*.rs'"
        );
    }

    #[test]
    fn test_risk_classification_safe() {
        let assessment = classify_command_risk("ls -la");
        assert_eq!(assessment.level, AiRiskLevel::Safe);
        assert!(assessment.warnings.is_empty());

        let assessment = classify_command_risk("git status");
        assert_eq!(assessment.level, AiRiskLevel::Safe);

        let assessment = classify_command_risk("docker ps -a");
        assert_eq!(assessment.level, AiRiskLevel::Safe);
    }

    #[test]
    fn test_risk_classification_destructive() {
        let assessment = classify_command_risk("rm -rf /tmp/data");
        assert_eq!(assessment.level, AiRiskLevel::Destructive);
        assert!(!assessment.warnings.is_empty());

        let assessment = classify_command_risk("git reset --hard HEAD~1");
        assert_eq!(assessment.level, AiRiskLevel::Destructive);
        assert!(!assessment.warnings.is_empty());

        let assessment = classify_command_risk("git clean -fd");
        assert_eq!(assessment.level, AiRiskLevel::Destructive);

        let assessment = classify_command_risk("dd if=/dev/zero of=/dev/sda");
        assert_eq!(assessment.level, AiRiskLevel::Destructive);
    }

    #[test]
    fn test_risk_classification_elevated() {
        let assessment = classify_command_risk("sudo systemctl restart nginx");
        assert_eq!(assessment.level, AiRiskLevel::Elevated);
        assert!(!assessment.warnings.is_empty());

        let assessment = classify_command_risk("chmod 777 /var/www");
        assert_eq!(assessment.level, AiRiskLevel::Elevated);
    }

    #[test]
    fn test_risk_classification_caution() {
        let assessment = classify_command_risk("pkill -f node");
        assert_eq!(assessment.level, AiRiskLevel::Caution);
        assert!(!assessment.warnings.is_empty());

        let assessment = classify_command_risk("docker rm -f web-server");
        assert_eq!(assessment.level, AiRiskLevel::Caution);

        let assessment = classify_command_risk("echo 'secret' > file.txt");
        assert_eq!(assessment.level, AiRiskLevel::Caution);
    }

    #[test]
    fn test_parse_structured_ai_response() {
        let raw = "COMMAND: find . -type f -name '*.ts'\nEXPLANATION: Recursively finds all TypeScript files in current directory.";
        let suggestion = parse_ai_response(raw).unwrap();
        assert_eq!(suggestion.command, "find . -type f -name '*.ts'");
        assert_eq!(
            suggestion.explanation,
            "Recursively finds all TypeScript files in current directory."
        );
        assert_eq!(suggestion.risk_level, AiRiskLevel::Safe);
    }

    #[test]
    fn test_parse_markdown_ai_response() {
        let raw = "Here is the command:\n```bash\ngit checkout .\n```\nThis discards all unstaged changes in working tree.";
        let suggestion = parse_ai_response(raw).unwrap();
        assert_eq!(suggestion.command, "git checkout .");
        assert_eq!(suggestion.risk_level, AiRiskLevel::Caution);
    }

    #[test]
    fn test_parse_ollama_generate_response() {
        let json = r#"{"model":"qwen2.5-coder","response":"COMMAND: ls\nEXPLANATION: lists files","done":true}"#;
        let response_text = parse_ollama_generate_response(json).unwrap();
        assert!(response_text.contains("COMMAND: ls"));
    }
    fn turn(role: ChatRole, content: &str) -> ChatMessage {
        ChatMessage {
            role,
            content: content.into(),
        }
    }

    #[test]
    fn chat_payload_carries_history_and_rejects_bad_input() {
        let msgs = vec![
            turn(ChatRole::User, "hi"),
            turn(ChatRole::Assistant, "hello"),
            turn(ChatRole::User, "why?"),
        ];
        let v: serde_json::Value =
            serde_json::from_str(&build_ollama_chat_payload("m", "sys", &msgs).unwrap()).unwrap();
        assert_eq!(v["stream"], false);
        assert_eq!(v["messages"].as_array().unwrap().len(), 4);
        assert_eq!(v["messages"][0]["role"], "system");
        assert_eq!(v["messages"][3]["content"], "why?");
        assert!(build_ollama_chat_payload("", "s", &msgs).is_err());
        assert!(build_ollama_chat_payload("m", "s", &[]).is_err());
        assert!(build_ollama_chat_payload("m", "s", &[turn(ChatRole::Assistant, "x")]).is_err());

        let huge = turn(ChatRole::User, &"a".repeat(CHAT_MAX_TOTAL_BYTES + 1));
        assert!(build_ollama_chat_payload("m", "s", &[huge]).is_err());
    }

    #[test]
    fn chat_response_is_parsed_and_errors_surface() {
        let ok = r#"{"message":{"role":"assistant","content":"42"},"done":true}"#;
        assert_eq!(parse_ollama_chat_response(ok).unwrap(), "42");
        assert!(parse_ollama_chat_response(r#"{"error":"model not found"}"#).is_err());
        assert!(parse_ollama_chat_response("not json").is_err());
    }
}
