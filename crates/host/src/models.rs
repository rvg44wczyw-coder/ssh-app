//! Model choices of the coding agents, for the quick-command menus: what `/model` can be given.
//!
//! - Antigravity: `agy models` (official; id and label per line).
//! - Claude Code: the aliases the CLI accepts, plus ids and names from the model catalog Claude Code
//!   keeps under `~/.claude/cache/model-catalog` when it has the expected shape (internal format,
//!   so it is only a bonus on top of the built-in list).
//! - Ollama: `ollama list`.
//!
//! Every id is checked against a strict alphabet because the app types it into a terminal.

use crate::proc::run_with_input;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

const MAX_MODELS: usize = 60;
const MAX_CATALOG_BYTES: u64 = 1 << 20;

pub struct Model {
    pub id: String,
    pub label: String,
}

/// An id the app may type into a terminal: starts alphanumeric, then letters, digits and `._:/@-`.
pub fn valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    id.len() <= 100
        && chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || "._:/@-".contains(c))
}

fn label(raw: &str, fallback: &str) -> String {
    let clean: String = raw.chars().filter(|c| !c.is_control()).take(80).collect();
    let clean = clean.trim();
    if clean.is_empty() { fallback } else { clean }.to_string()
}

fn push(out: &mut Vec<Model>, id: &str, name: &str) {
    if out.len() < MAX_MODELS && valid_id(id) && !out.iter().any(|m| m.id == id) {
        out.push(Model {
            id: id.to_string(),
            label: label(name, id),
        });
    }
}

/// `id<TAB>label` lines (`agy models`); lines without a tab (progress text) are skipped.
pub fn parse_tabbed(text: &str) -> Vec<Model> {
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some((id, name)) = line.split_once('\t') {
            push(&mut out, id.trim(), name);
        }
    }
    out
}

/// `ollama list`: a header line, then `NAME  ID  SIZE  MODIFIED` columns.
pub fn parse_ollama(text: &str) -> Vec<Model> {
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        if let Some(name) = line.split_whitespace().next() {
            push(&mut out, name, name);
        }
    }
    out
}

/// Models of a Claude Code catalog document (`catalog.config.models`), if it has that shape.
pub fn parse_claude_catalog(doc: &Value) -> Vec<Model> {
    let mut out = Vec::new();
    let models = doc["catalog"]["config"]["models"].as_array();
    for m in models.into_iter().flatten() {
        if let (Some(id), Some(name)) = (m["id"].as_str(), m["name"].as_str()) {
            push(&mut out, id, name);
        }
    }
    out
}

fn claude_models(home: &str) -> (Vec<Model>, &'static str) {
    let mut out = Vec::new();
    for (alias, name) in [
        ("fable", "Fable (latest)"),
        ("opus", "Opus (latest)"),
        ("sonnet", "Sonnet (latest)"),
        ("haiku", "Haiku (latest)"),
    ] {
        push(&mut out, alias, name);
    }
    let dir = PathBuf::from(home).join(".claude/cache/model-catalog");
    let mut source = "builtin";
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    files.sort();
    for path in files {
        let small =
            std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() <= MAX_CATALOG_BYTES);
        let doc = small
            .then(|| std::fs::read(&path).ok())
            .flatten()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
        if let Some(doc) = doc {
            for m in parse_claude_catalog(&doc) {
                push(&mut out, &m.id, &m.label);
                source = "cache";
            }
        }
    }
    (out, source)
}

fn run(program: &str, args: &[&str], timeout: Duration) -> Option<String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    let (code, out) = run_with_input(cmd, b"", timeout)?;
    (code == Some(0)).then(|| String::from_utf8_lossy(&out).into_owned())
}

fn find_tool(name: &str, home: &str) -> String {
    for dir in [".local/bin", ".antigravity/bin", "bin"] {
        let p = PathBuf::from(home).join(dir).join(name);
        if p.is_file() {
            return p.to_string_lossy().into_owned();
        }
    }
    name.to_string()
}

/// The reply for `models <agent>`.
pub fn reply(agent: &str, home: &str) -> Value {
    let (models, source) = match agent {
        "claude" => claude_models(home),
        "antigravity" | "agy" => (
            run(
                &find_tool("agy", home),
                &["models"],
                Duration::from_secs(25),
            )
            .map(|t| parse_tabbed(&t))
            .unwrap_or_default(),
            "cli",
        ),
        "ollama" => (
            run("ollama", &["list"], Duration::from_secs(8))
                .map(|t| parse_ollama(&t))
                .unwrap_or_default(),
            "cli",
        ),
        _ => return json!({"ok": false, "error": "unknown agent"}),
    };
    json!({
        "ok": true, "agent": agent, "source": source,
        "models": models.iter().map(|m| json!({"id": m.id, "label": m.label})).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_safe_ids_pass() {
        assert!(valid_id("claude-opus-5-5"));
        assert!(valid_id("gemini-3.8-flash-high"));
        assert!(valid_id("llama3:8b"));
        assert!(valid_id("library/qwen2.5-coder:7b"));
        assert!(!valid_id(""));
        assert!(!valid_id("-rf"));
        assert!(!valid_id("a b"));
        assert!(!valid_id("x;rm -rf ~"));
        assert!(!valid_id("a\nb"));
        assert!(!valid_id(&"a".repeat(101)));
    }

    #[test]
    fn parses_agy_models_output() {
        let text = "Fetching available models...\ngemini-4-argon-low\tGemini 4 Argon (Low)\nbad id\tx\nclaude-opus-5-5-high\tClaude Opus 5.5 (High)\n";
        let m = parse_tabbed(text);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].id, "gemini-4-argon-low");
        assert_eq!(m[1].label, "Claude Opus 5.5 (High)");
    }

    #[test]
    fn parses_ollama_list() {
        let text = "NAME            ID      SIZE   MODIFIED\nqwen2.5-coder:7b  abc  4.7 GB  2 days ago\nllama3:8b  def  4.7 GB  3 days ago\n";
        let m = parse_ollama(text);
        assert_eq!(
            m.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["qwen2.5-coder:7b", "llama3:8b"]
        );
    }

    #[test]
    fn reads_the_claude_catalog_shape_and_ignores_others() {
        let doc = json!({"catalog": {"config": {"models": [
            {"id": "claude-opus-5-5", "name": "Opus 5.5"},
            {"id": "bad id", "name": "x"},
            {"name": "no id"},
        ]}}});
        let m = parse_claude_catalog(&doc);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].label, "Opus 5.5");
        assert!(parse_claude_catalog(&json!({"other": 1})).is_empty());
    }

    #[test]
    fn claude_list_starts_with_the_aliases_and_adds_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let cat = dir.path().join(".claude/cache/model-catalog");
        std::fs::create_dir_all(&cat).unwrap();
        std::fs::write(
            cat.join("x.json"),
            json!({"catalog": {"config": {"models": [{"id": "claude-opus-5-5", "name": "Opus 5.5"}]}}})
                .to_string(),
        )
        .unwrap();
        std::fs::write(cat.join("junk.json"), "not json").unwrap();
        let (m, source) = claude_models(dir.path().to_str().unwrap());
        assert_eq!(source, "cache");
        assert_eq!(m[0].id, "fable");
        assert!(m.iter().any(|m| m.id == "claude-opus-5-5"));
        let (m, source) = claude_models("/nonexistent-home");
        assert_eq!((m.len(), source), (4, "builtin"));
    }

    #[test]
    fn unknown_agents_are_refused() {
        assert_eq!(reply("nope", "/tmp")["ok"], false);
    }
}
