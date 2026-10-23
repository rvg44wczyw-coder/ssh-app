//! Subagents of a Claude Code session (feature 55 §9). A subagent writes its own file next to the
//! session: `<session>/subagents/agent-<id>.jsonl` with `agent-<id>.meta.json` beside it. The file
//! has the line format of the session file, so it is read by cursor into a `FeedState` made with
//! `FeedState::new_subagent()`; this module finds the agents and builds the safe path of a file.

use crate::error::SshCoreError;
use crate::tmux::shell_quote;

/// Agents listed per session.
pub const MAX_SUBAGENTS: usize = 20;
/// Deepest nesting shown (the session's own agents are depth 1).
pub const MAX_DEPTH: u32 = 3;
/// A subagent whose file has not grown for this long is no longer running.
pub const RUNNING_WINDOW_SECS: u64 = 60;
const MAX_TEXT: usize = 300;

/// One subagent the host knows of.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SubagentInfo {
    pub agent_id: String,
    /// `Explore`, `general-purpose`, ... (display text).
    pub agent_type: String,
    pub description: String,
    /// Id of the parent's `Agent` tool call that started it (empty when unknown).
    pub tool_use_id: String,
    pub depth: u32,
    pub model: String,
    pub background: bool,
    /// Seconds since its file last grew, by the host's clock.
    pub idle_seconds: u64,
    /// Its file grew within the last minute.
    pub running: bool,
}

/// `^a[0-9a-f]{8,40}$`
pub fn valid_agent_id(id: &str) -> bool {
    (9..=41).contains(&id.len())
        && id.starts_with('a')
        && id[1..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The file of a subagent next to the session file `main_path`. Both paths are checked; the id
/// cannot carry a path.
pub fn subagent_file(main_path: &str, agent_id: &str) -> Result<String, SshCoreError> {
    let bad = |reason: &str| SshCoreError::InvalidTranscriptPath {
        path: main_path.to_string(),
        reason: reason.to_string(),
    };
    crate::transcript::validate_transcript_path(main_path)?;
    if !valid_agent_id(agent_id) {
        return Err(bad("Invalid subagent id"));
    }
    let stem = main_path
        .trim()
        .strip_suffix(".jsonl")
        .ok_or_else(|| bad("Not a session file"))?;
    let file = format!("{stem}/subagents/agent-{agent_id}.jsonl");
    crate::transcript::validate_transcript_path(&file)?;
    Ok(file)
}

/// `$1` the session file. Prints `NOW <epoch>` and, for each agent (newest first, at most 40
/// looked at), a line `SUB <id> <mtime of its file>` followed by its meta file on one line (first
/// 4 KB). The phone keeps at most 20.
pub(crate) fn list_script_with(guard: bool) -> String {
    let check = if guard {
        format!(
            "case \"$r\" in {}) echo NOFILE; exit 0;; esac\n",
            crate::feed_read::BLOCKED_PATTERNS
        )
    } else {
        String::new()
    };
    format!(
        r#"r=$(realpath -- "$1" 2>/dev/null || readlink -f -- "$1" 2>/dev/null) && [ -f "$r" ] || {{ echo NOFILE; exit 0; }}
{check}case "$r" in *.jsonl) ;; *) echo NOFILE; exit 0;; esac
d="${{r%.jsonl}}/subagents"
printf 'NOW %s\n' "$(date +%s)"
[ -d "$d" ] && [ ! -L "$d" ] || exit 0
ls -t "$d" 2>/dev/null | grep '\.meta\.json$' | head -n 40 | while IFS= read -r b; do
  id=${{b#agent-}}; id=${{id%.meta.json}}
  case "$id" in a*) ;; *) continue;; esac
  case "${{id#a}}" in ""|*[!0-9a-f]*) continue;; esac
  m="$d/$b"; f="$d/agent-$id.jsonl"
  [ -f "$m" ] && [ ! -L "$m" ] || continue
  mt=$(stat -c %Y "$f" 2>/dev/null || stat -f %m "$f" 2>/dev/null || echo 0)
  printf 'SUB %s %s\n' "$id" "$mt"
  head -c 4096 "$m" | tr -d '\n\r'
  printf '\n'
done
exit 0"#
    )
}

/// The command that lists the subagents of `main_path` (plain `sh`, arguments quoted).
pub fn build_list_command(main_path: &str) -> Result<String, SshCoreError> {
    crate::transcript::validate_transcript_path(main_path)?;
    Ok(format!(
        "exec sh -c {} sh {}",
        shell_quote(&list_script_with(true)),
        shell_quote(main_path)
    ))
}

fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(MAX_TEXT)
        .collect()
}

/// Reads the answer of the list script. Unknown ids, deeper agents and everything past the cap are
/// left out; text is display data only.
pub fn parse_subagent_list(out: &str) -> Vec<SubagentInfo> {
    let mut now = 0u64;
    let mut list: Vec<SubagentInfo> = Vec::new();
    let mut pending: Option<(String, u64)> = None;
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("NOW ") {
            now = rest.trim().parse().unwrap_or(0);
            continue;
        }
        if let Some(rest) = line.strip_prefix("SUB ") {
            let mut it = rest.split(' ');
            pending = match (it.next(), it.next().and_then(|m| m.parse::<u64>().ok())) {
                (Some(id), Some(mt)) if valid_agent_id(id) => Some((id.to_string(), mt)),
                _ => None,
            };
            continue;
        }
        let Some((id, mtime)) = pending.take() else {
            continue;
        };
        let Ok(meta) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let text = |k: &str| {
            meta.get(k)
                .and_then(|v| v.as_str())
                .map(clean)
                .unwrap_or_default()
        };
        let depth = meta
            .get("spawnDepth")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .min(u64::from(u32::MAX)) as u32;
        if depth > MAX_DEPTH || list.len() >= MAX_SUBAGENTS || list.iter().any(|s| s.agent_id == id)
        {
            continue;
        }
        let tool_use_id = text("toolUseId");
        let tool_use_id = if tool_use_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            tool_use_id
        } else {
            String::new()
        };
        let idle_seconds = now.saturating_sub(mtime);
        list.push(SubagentInfo {
            agent_id: id,
            agent_type: text("agentType"),
            description: text("description"),
            tool_use_id,
            depth,
            model: text("model"),
            background: meta.get("requestShape").and_then(|v| v.as_str()) == Some("background"),
            idle_seconds,
            running: mtime > 0 && idle_seconds < RUNNING_WINDOW_SECS,
        });
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::FeedState;
    use std::process::Command;

    const ID: &str = "a949df962524b4622";

    fn sub_lines() -> String {
        [
            r#"{"type":"user","isSidechain":true,"agentId":"a949df962524b4622","uuid":"u1","message":{"role":"user","content":"find the parser"}}"#,
            r#"{"type":"attachment","isSidechain":true,"agentId":"a949df962524b4622","uuid":"x1","attachment":{"type":"fork-context-ref"}}"#,
            r#"{"type":"assistant","isSidechain":true,"uuid":"u2","message":{"id":"m1","role":"assistant","content":[{"type":"text","text":"Looking"},{"type":"tool_use","id":"t1","name":"Grep","input":{"pattern":"parse"}}]}}"#,
            r#"{"type":"user","isSidechain":true,"uuid":"u3","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"src/lib.rs:1"}]}}"#,
            r#"{"type":"assistant","isSidechain":true,"uuid":"u4","message":{"id":"m2","role":"assistant","content":[{"type":"text","text":"Found it"}]}}"#,
        ]
        .join("\n")
            + "\n"
    }

    #[test]
    fn a_subagent_file_replayed_in_chunks_equals_its_whole_parse() {
        let text = sub_lines();
        let whole = FeedState::new_subagent();
        whole.apply_chunk(text.clone());
        let turns = whole.turns();
        assert!(turns.len() >= 3, "{turns:?}");
        assert!(turns.iter().any(|t| t
            .tool_calls
            .iter()
            .any(|c| c.tool_name == "Grep" && c.result.is_some())));
        for size in [1usize, 5, 17, 64, 1000] {
            let s = FeedState::new_subagent();
            for chunk in text.as_bytes().chunks(size) {
                s.apply_chunk(String::from_utf8_lossy(chunk).to_string());
            }
            assert_eq!(s.turns(), turns, "chunk size {size}");
        }
        // The same file read as a session's own is left out (side chain), and a reset keeps the kind.
        let main = FeedState::new();
        main.apply_chunk(text.clone());
        assert!(main.turns().is_empty());
        whole.reset();
        whole.apply_chunk(text);
        assert_eq!(whole.turns(), turns);
    }

    #[test]
    fn ids_and_paths_are_checked() {
        assert!(valid_agent_id(ID));
        for bad in [
            "",
            "a",
            "a1234567",
            "b949df962524b4622",
            "a../etc",
            "aXYZ12345",
            "a949df962524b4622a949df962524b4622a949df962524b4622",
            "a949df96/..",
            "a949df96\n",
        ] {
            assert!(!valid_agent_id(bad), "{bad:?}");
        }
        let main = "/home/u/.claude/projects/p/7e046360-70ef-46d1-af54-a6571d55f174.jsonl";
        assert_eq!(
            subagent_file(main, ID).expect("ok"),
            format!("/home/u/.claude/projects/p/7e046360-70ef-46d1-af54-a6571d55f174/subagents/agent-{ID}.jsonl")
        );
        assert!(subagent_file(main, "a/../../x1234567").is_err());
        assert!(subagent_file("/home/u/notes.txt", ID).is_err());
        assert!(subagent_file("/home/u/../etc/x.jsonl", ID).is_err());
        assert!(subagent_file("/etc/x.jsonl", ID).is_err());
        assert!(build_list_command("/home/u/a'b.jsonl").is_err());
    }

    #[test]
    fn the_list_is_read_with_caps_and_a_clock_from_the_host() {
        let meta = |t: &str, depth: u32| {
            format!(
                r#"{{"agentType":"{t}","description":"Find\u0007 things","toolUseId":"toolu_01AB","spawnDepth":{depth},"model":"opus","requestShape":"background"}}"#
            )
        };
        let mut out = String::from("NOW 1000\n");
        out += &format!("SUB {ID} 990\n{}\n", meta("Explore", 1));
        out += &format!("SUB a1111111111 100\n{}\n", meta("Plan", 2));
        out += &format!("SUB a2222222222 100\n{}\n", meta("Deep", 4)); // too deep
        out += "SUB ../evil 100\n{}\n"; // refused id
        out += &format!("SUB {ID} 100\n{}\n", meta("Dup", 1)); // duplicate
        out += "SUB a3333333333 100\nnot json\n";
        let list = parse_subagent_list(&out);
        assert_eq!(list.len(), 2, "{list:?}");
        assert_eq!(list[0].agent_id, ID);
        assert_eq!(list[0].description, "Find things");
        assert_eq!(list[0].tool_use_id, "toolu_01AB");
        assert!(list[0].running && list[0].background);
        assert_eq!(list[0].idle_seconds, 10);
        assert!(!list[1].running, "idle 900 s");
        // The 60 s rule at its edge.
        let edge = |mt: u64| {
            parse_subagent_list(&format!("NOW 1000\nSUB {ID} {mt}\n{}\n", meta("E", 1)))[0].running
        };
        assert!(edge(941) && !edge(940) && !edge(0));
        // The cap.
        let many: String = (0..30)
            .map(|i| format!("SUB a{i:010x} 999\n{}\n", meta("X", 1)))
            .collect();
        assert_eq!(
            parse_subagent_list(&format!("NOW 1000\n{many}")).len(),
            MAX_SUBAGENTS
        );
        // Hostile ids in tool_use_id are dropped, not trusted.
        let hostile = format!("NOW 1\nSUB {ID} 1\n{{\"toolUseId\":\"a b;rm\"}}\n");
        assert_eq!(parse_subagent_list(&hostile)[0].tool_use_id, "");
    }

    #[test]
    fn the_list_script_finds_the_agents_next_to_the_session_and_ignores_the_rest() {
        let dir = tempfile::tempdir().expect("dir");
        let main = dir.path().join("sess.jsonl");
        std::fs::write(&main, "{}\n").expect("w");
        let sub = dir.path().join("sess/subagents");
        std::fs::create_dir_all(&sub).expect("mk");
        let meta =
            r#"{"agentType":"Explore","description":"d","toolUseId":"toolu_1","spawnDepth":1}"#;
        std::fs::write(sub.join(format!("agent-{ID}.meta.json")), meta).expect("m");
        std::fs::write(sub.join(format!("agent-{ID}.jsonl")), "{}\n").expect("f");
        std::fs::write(sub.join("agent-aZZ.meta.json"), meta).expect("m");
        std::fs::write(sub.join("agent-b123456789.meta.json"), meta).expect("m");
        std::fs::write(sub.join("notes.txt"), "x").expect("m");
        let secret = dir.path().join("secret.json");
        std::fs::write(&secret, r#"{"agentType":"STOLEN"}"#).expect("m");
        std::os::unix::fs::symlink(&secret, sub.join("agent-a999999999.meta.json")).expect("ln");
        let run = |path: &std::path::Path| {
            let o = Command::new("sh")
                .args(["-c", &list_script_with(false), "sh"])
                .arg(path)
                .output()
                .expect("sh");
            String::from_utf8_lossy(&o.stdout).to_string()
        };
        let list = parse_subagent_list(&run(&main));
        assert_eq!(list.len(), 1, "{list:?}");
        assert_eq!(list[0].agent_id, ID);
        assert!(list[0].running, "just written");
        assert!(run(&dir.path().join("missing.jsonl")).starts_with("NOFILE"));
        std::fs::write(dir.path().join("plain.txt"), "x").expect("w");
        assert!(run(&dir.path().join("plain.txt")).starts_with("NOFILE"));
        // A session without a subagents folder: a clock and nothing else.
        let lone = dir.path().join("lone.jsonl");
        std::fs::write(&lone, "{}\n").expect("w");
        assert!(parse_subagent_list(&run(&lone)).is_empty());
    }
}
