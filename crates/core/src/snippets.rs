use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::OnceLock;

/// Represents a reusable shell command snippet or history entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct SnippetItem {
    pub id: String,
    pub title: String,
    pub command: String,
    pub category: String,
    pub tags: Vec<String>,
    pub use_count: u64,
}

/// Represents a ranked fuzzy search result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct FuzzyMatchResult {
    pub item: SnippetItem,
    pub score: i64,
    pub matched_indices: Vec<u32>,
}

static PLACEHOLDER_REGEX: OnceLock<Regex> = OnceLock::new();
static SECRET_PATTERN_REGEX: OnceLock<Regex> = OnceLock::new();

fn get_placeholder_regex() -> &'static Regex {
    PLACEHOLDER_REGEX
        .get_or_init(|| Regex::new(r"\{\{\s*([a-zA-Z0-9_-]+)\s*\}\}").expect("Valid regex"))
}

fn get_secret_regex() -> &'static Regex {
    SECRET_PATTERN_REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)((api[_-]?key|secret|token|password|passwd|private[_-]?key)\s*[=:]|authorization:\s*bearer\s+)",
        )
        .expect("Valid secret regex")
    })
}

/// Extracts unique `{{variable}}` placeholder names in order of appearance.
#[uniffi::export]
pub fn extract_snippet_placeholders(command: String) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for cap in get_placeholder_regex().captures_iter(&command) {
        if let Some(m) = cap.get(1) {
            let name = m.as_str().to_string();
            if seen.insert(name.clone()) {
                result.push(name);
            }
        }
    }
    result
}

/// Returns the built-in catalog of developer snippets for AI agents, Git, Cargo, Tmux, and System.
#[uniffi::export]
pub fn default_developer_snippets() -> Vec<SnippetItem> {
    vec![
        SnippetItem {
            id: "builtin-agy-continue".to_string(),
            title: "Resume Antigravity Session".to_string(),
            command: "agy --continue".to_string(),
            category: "AI Agents".to_string(),
            tags: vec![
                "agy".to_string(),
                "antigravity".to_string(),
                "resume".to_string(),
            ],
            use_count: 10,
        },
        SnippetItem {
            id: "builtin-claude-code".to_string(),
            title: "Launch Claude Code".to_string(),
            command: "claude".to_string(),
            category: "AI Agents".to_string(),
            tags: vec!["claude".to_string(), "agent".to_string()],
            use_count: 9,
        },
        SnippetItem {
            id: "builtin-ollama-list".to_string(),
            title: "List Local Ollama Models".to_string(),
            command: "ollama list".to_string(),
            category: "AI Agents".to_string(),
            tags: vec![
                "ollama".to_string(),
                "llm".to_string(),
                "models".to_string(),
            ],
            use_count: 6,
        },
        SnippetItem {
            id: "builtin-git-status".to_string(),
            title: "Git Short Status & Branch".to_string(),
            command: "git status -sb".to_string(),
            category: "Git".to_string(),
            tags: vec!["git".to_string(), "status".to_string()],
            use_count: 12,
        },
        SnippetItem {
            id: "builtin-git-log".to_string(),
            title: "Git Graph Log (Last 15)".to_string(),
            command: "git log --oneline --graph --decorate -n 15".to_string(),
            category: "Git".to_string(),
            tags: vec!["git".to_string(), "log".to_string(), "history".to_string()],
            use_count: 8,
        },
        SnippetItem {
            id: "builtin-git-checkout".to_string(),
            title: "Switch / Create Git Branch".to_string(),
            command: "git checkout -b {{branch}}".to_string(),
            category: "Git".to_string(),
            tags: vec!["git".to_string(), "branch".to_string()],
            use_count: 5,
        },
        SnippetItem {
            id: "builtin-cargo-test".to_string(),
            title: "Cargo Test Workspace".to_string(),
            command: "cargo test --workspace".to_string(),
            category: "Cargo".to_string(),
            tags: vec!["rust".to_string(), "cargo".to_string(), "test".to_string()],
            use_count: 11,
        },
        SnippetItem {
            id: "builtin-cargo-clippy".to_string(),
            title: "Cargo Clippy Strict".to_string(),
            command: "cargo clippy --workspace -- -D warnings".to_string(),
            category: "Cargo".to_string(),
            tags: vec!["rust".to_string(), "clippy".to_string(), "lint".to_string()],
            use_count: 7,
        },
        SnippetItem {
            id: "builtin-ports-listen".to_string(),
            title: "Show Listening TCP Ports".to_string(),
            command: "lsof -iTCP -sTCP:LISTEN -P -n".to_string(),
            category: "System".to_string(),
            tags: vec![
                "ports".to_string(),
                "network".to_string(),
                "lsof".to_string(),
            ],
            use_count: 8,
        },
        SnippetItem {
            id: "builtin-tmux-ls".to_string(),
            title: "List Active tmux Sessions".to_string(),
            command: "tmux list-sessions".to_string(),
            category: "System".to_string(),
            tags: vec!["tmux".to_string(), "sessions".to_string()],
            use_count: 6,
        },
    ]
}

/// Computes a fuzzy match score and matched character indices for `query` against `target`.
fn score_fuzzy(query: &str, target: &str) -> Option<(i64, Vec<u32>)> {
    if query.is_empty() {
        return Some((0, Vec::new()));
    }

    let q_chars: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).collect();
    let t_chars: Vec<char> = target.chars().collect();
    let t_lower: Vec<char> = target
        .chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect();

    if q_chars.len() > t_lower.len() {
        return None;
    }

    let mut q_idx = 0;
    let mut matched_indices = Vec::with_capacity(q_chars.len());
    let mut score: i64 = 0;
    let mut prev_match_idx: Option<usize> = None;

    for (t_idx, &tc) in t_lower.iter().enumerate() {
        if q_idx < q_chars.len() && tc == q_chars[q_idx] {
            matched_indices.push(t_idx as u32);
            score += 10;

            // Prefix bonus
            if t_idx == 0 {
                score += 25;
            }

            // Word boundary bonus
            if t_idx > 0 {
                let prev_char = t_chars[t_idx - 1];
                if prev_char == ' ' || prev_char == '-' || prev_char == '_' || prev_char == '/' {
                    score += 20;
                }
            }

            // Contiguous run bonus
            if let Some(prev) = prev_match_idx {
                if t_idx == prev + 1 {
                    score += 15;
                } else {
                    // Gap penalty
                    let gap = (t_idx - prev - 1) as i64;
                    score -= gap.min(10);
                }
            }

            prev_match_idx = Some(t_idx);
            q_idx += 1;
        }
    }

    if q_idx == q_chars.len() {
        Some((score, matched_indices))
    } else {
        None
    }
}

/// Searches and ranks snippets by fuzzy matching `query` against command, title, category, and tags.
#[uniffi::export]
pub fn fuzzy_search_snippets(
    query: String,
    items: Vec<SnippetItem>,
    limit: u32,
) -> Vec<FuzzyMatchResult> {
    let trimmed = query.trim();
    let max_items = if limit == 0 { 50 } else { limit as usize };

    let mut results: Vec<FuzzyMatchResult> = Vec::new();

    for item in items {
        if trimmed.is_empty() {
            results.push(FuzzyMatchResult {
                score: item.use_count as i64,
                item,
                matched_indices: Vec::new(),
            });
            continue;
        }

        let cmd_match = score_fuzzy(trimmed, &item.command);
        let title_match = score_fuzzy(trimmed, &item.title);
        let tags_joined = item.tags.join(" ");
        let tag_match = score_fuzzy(trimmed, &tags_joined);

        let best = match (cmd_match, title_match, tag_match) {
            (Some((cs, ci)), Some((ts, _)), _) if cs >= ts => Some((cs + 10, ci)),
            (_, Some((ts, _)), _) => Some((ts, Vec::new())),
            (Some((cs, ci)), None, _) => Some((cs + 10, ci)),
            (None, None, Some((tgs, _))) => Some((tgs - 5, Vec::new())),
            (None, None, None) => None,
        };

        if let Some((base_score, indices)) = best {
            let frequency_bonus = (item.use_count.min(50) as i64) * 2;
            results.push(FuzzyMatchResult {
                item,
                score: base_score + frequency_bonus,
                matched_indices: indices,
            });
        }
    }

    results.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.item.title.cmp(&b.item.title))
    });
    results.truncate(max_items);
    results
}

/// Parses raw `.zsh_history` / `.bash_history` lines, scrubs secrets, and returns unique recent commands.
#[uniffi::export]
pub fn extract_history_commands(raw_history: String, limit: u32) -> Vec<String> {
    let max_items = if limit == 0 { 100 } else { limit as usize };
    let mut seen = HashSet::new();
    let mut commands = Vec::new();
    let secret_re = get_secret_regex();

    for line in raw_history.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Handle zsh extended_history format: `: 1700000000:0;actual command`
        let cmd = if trimmed.starts_with(": ") {
            if let Some(semi_pos) = trimmed.find(';') {
                trimmed[semi_pos + 1..].trim()
            } else {
                trimmed
            }
        } else {
            trimmed
        };

        if cmd.is_empty() || cmd.len() < 2 || cmd.len() > 500 {
            continue;
        }

        // Scrub lines containing potential credentials or API keys
        if secret_re.is_match(cmd) {
            continue;
        }

        if seen.insert(cmd.to_string()) {
            commands.push(cmd.to_string());
            if commands.len() >= max_items {
                break;
            }
        }
    }

    commands
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_snippet_placeholders() {
        let cmd = "git checkout -b {{branch}} && git push origin {{branch}} --port {{port}}";
        let vars = extract_snippet_placeholders(cmd.to_string());
        assert_eq!(vars, vec!["branch".to_string(), "port".to_string()]);
    }

    #[test]
    fn test_fuzzy_search_ranking() {
        let items = default_developer_snippets();
        let results = fuzzy_search_snippets("ctest".to_string(), items, 10);
        assert!(!results.is_empty());
        assert_eq!(results[0].item.command, "cargo test --workspace");
    }

    #[test]
    fn test_extract_history_commands_zsh_and_secret_scrubbing() {
        let raw = r#"
: 1712000001:0;ls -la
: 1712000002:0;export OPENAI_API_KEY=sk-1234567890
: 1712000003:0;cargo test -p ssh-core
curl -H "Authorization: Bearer secret_token" https://example.com
: 1712000004:0;git status -sb
: 1712000005:0;cargo test -p ssh-core
"#;
        let cmds = extract_history_commands(raw.to_string(), 10);
        assert_eq!(
            cmds,
            vec![
                "cargo test -p ssh-core".to_string(),
                "git status -sb".to_string(),
                "ls -la".to_string(),
            ]
        );
    }
}
