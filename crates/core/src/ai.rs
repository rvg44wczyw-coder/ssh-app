use crate::error::SshCoreError;
use serde::{Deserialize, Serialize};

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

/// Deterministically evaluates a shell command against safety rules and returns its risk level and warnings.
#[uniffi::export]
pub fn classify_command_risk(command: &str) -> AiRiskAssessment {
    let cmd = command.trim();
    let mut warnings = Vec::new();

    // 1. Destructive check
    let destructive_patterns = [
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
    ];

    for (pattern, desc) in destructive_patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            if re.is_match(cmd) {
                warnings.push(desc.to_string());
            }
        }
    }

    if !warnings.is_empty() {
        return AiRiskAssessment {
            level: AiRiskLevel::Destructive,
            warnings,
        };
    }

    // 2. Elevated check
    let elevated_patterns = [
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
    ];

    for (pattern, desc) in elevated_patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            if re.is_match(cmd) {
                warnings.push(desc.to_string());
            }
        }
    }

    if !warnings.is_empty() {
        return AiRiskAssessment {
            level: AiRiskLevel::Elevated,
            warnings,
        };
    }

    // 3. Caution check
    let caution_patterns = [
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
    ];

    for (pattern, desc) in caution_patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            if re.is_match(cmd) {
                warnings.push(desc.to_string());
            }
        }
    }

    if !warnings.is_empty() {
        return AiRiskAssessment {
            level: AiRiskLevel::Caution,
            warnings,
        };
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

#[cfg(test)]
mod tests {
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
}
