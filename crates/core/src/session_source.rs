//! The agents' side of finding a session on the host (docs/features/60-universal-agent-feed.md). One
//! `SessionSource` per agent says how its process is told from others by the command line, which name the
//! hooks use for it, and the shell arm that prints the path of its session. The locate script that runs
//! on the host is built from the registry (`agent_registry`: program names) and these sources, so a new
//! agent is one more source and one more registry row, not an edit of a shared script.

use crate::agent_registry::agent_descriptor;

/// What the host-side locate script needs to know about one agent.
pub(crate) trait SessionSource: Sync {
    /// The id of the agent's row in the registry (`claude`, `agy`, ...); it is also what the detection
    /// part of the script prints as the agent's kind.
    fn id(&self) -> &'static str;
    /// The name the agent's hooks write into `~/.ssh-app/run/sessions/*.json` (`"agent":"..."`).
    fn hook_agent(&self) -> &'static str;
    /// awk regex fragments (without the slashes) that mean this agent when found in a command line
    /// whose program name did not (a node script under the agent's package folder).
    fn arg_markers(&self) -> &'static [&'static str];
    /// The body of the `case "$kind"` arm: sets nothing, prints the session path or nothing. It may use
    /// `$apid` (the agent's pid), `$cwd`, `$sid` (the id the hooks noted, or empty) and `$HOME`.
    fn locate_arm(&self) -> &'static str;
}

struct ClaudeSource;
struct AgySource;
struct CodexSource;
struct GrokSource;

impl SessionSource for ClaudeSource {
    fn id(&self) -> &'static str {
        "claude"
    }
    fn hook_agent(&self) -> &'static str {
        "claude"
    }
    fn arg_markers(&self) -> &'static [&'static str] {
        &[r"@anthropic-ai\/claude-code", r"\/claude-code\/"]
    }
    fn locate_arm(&self) -> &'static str {
        CLAUDE_ARM
    }
}

impl SessionSource for AgySource {
    fn id(&self) -> &'static str {
        "agy"
    }
    fn hook_agent(&self) -> &'static str {
        "antigravity"
    }
    fn arg_markers(&self) -> &'static [&'static str] {
        &[r"\/antigravity\/"]
    }
    fn locate_arm(&self) -> &'static str {
        AGY_ARM
    }
}

impl SessionSource for CodexSource {
    fn id(&self) -> &'static str {
        "codex"
    }
    fn hook_agent(&self) -> &'static str {
        "codex"
    }
    fn arg_markers(&self) -> &'static [&'static str] {
        &[r"@openai\/codex"]
    }
    fn locate_arm(&self) -> &'static str {
        CODEX_ARM
    }
}

impl SessionSource for GrokSource {
    fn id(&self) -> &'static str {
        "grok"
    }
    fn hook_agent(&self) -> &'static str {
        "grok"
    }
    fn arg_markers(&self) -> &'static [&'static str] {
        &[]
    }
    fn locate_arm(&self) -> &'static str {
        GROK_ARM
    }
}

/// Every agent whose session the host can locate, in the order the detection tries them.
fn sources() -> [&'static dyn SessionSource; 4] {
    [&ClaudeSource, &AgySource, &CodexSource, &GrokSource]
}

/// A program name that may be put between quotes in the awk part of the script: letters, digits, `.`, `_`
/// and `-` only (names come from the registry, but the script must not depend on every row being tidy).
pub(crate) fn is_safe_program_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

/// The locate script: finds the agent below the pane's process, then prints its session file. Expects
/// `$roots` (space separated pids of the pane, see `zellij_feed::roots_prefix`) and `$1` (a validated
/// session name). Prints one path or nothing.
pub(crate) fn locate_script() -> String {
    let mut s = String::from(PRE);
    let src = sources();
    for source in src {
        let names: Vec<String> = agent_descriptor(source.id().into())
            .map(|d| d.process_names)
            .unwrap_or_default()
            .iter()
            .filter(|n| is_safe_program_name(n))
            .map(|n| format!("b == \"{n}\""))
            .collect();
        if names.is_empty() {
            continue;
        }
        s.push_str(&format!(
            "        if ({}) {{ print \"{} \" p; exit }}\n",
            names.join(" || "),
            source.id()
        ));
    }
    s.push_str(AFTER_NAMES);
    for source in src {
        let markers = source.arg_markers();
        if markers.is_empty() {
            continue;
        }
        s.push_str(&format!(
            "      if (c ~ /{}/) {{ print \"{} \" p; exit }}\n",
            markers.join("|"),
            source.id()
        ));
    }
    s.push_str(MID_A);
    // The first source is the default name, the others are named in a case.
    s.push_str("hook=");
    s.push_str(src[0].hook_agent());
    s.push_str("; case \"$kind\" in");
    for source in &src[1..] {
        s.push_str(&format!(" {}) hook={};;", source.id(), source.hook_agent()));
    }
    s.push_str(" esac\n");
    s.push_str("sid=\"\"; bts=-1\n");
    s.push_str(MID_B);
    for source in src {
        s.push_str(&format!("  {})\n", source.id()));
        s.push_str(source.locate_arm());
    }
    s.push_str(TAIL);
    s
}

const PRE: &str = r#"found=$(ps -Ao pid=,ppid=,args= 2>/dev/null | awk -v roots=" $roots " '
  { id=$1; par=$2; $1=""; $2=""; sub(/^[ \t]+/, ""); pp[id]=par; cmd[id]=$0 }
  END { for (p in pp) { q=p; d=0; while (q != "" && q != 0 && d < 40) { if (index(roots, " " q " ")) {
      c=cmd[p]; n=split(c, w, " ")
      for (i=1; i<=n && i<=4; i++) {
        b=w[i]; sub(/.*\//, "", b); sub(/\.(js|mjs|py)$/, "", b); b=tolower(b)
        if (i > 1 && w[1] !~ /(node|bun|deno|python[0-9.]*|sh|bash|zsh)$/) break
"#;

const AFTER_NAMES: &str = r#"      }
"#;

const MID_A: &str = r#"      break } q=pp[q]; d++ } } }')
kind=${found%% *}; apid=${found##* }
case "$apid" in ""|*[!0-9]*) exit 0;; esac
# The agent's own session id, as the hooks noted it for this tmux session (~/.ssh-app/run/sessions):
# it names the file exactly, even when the same agent runs in the same folder on the laptop.
"#;

const MID_B: &str = r#"for r in "$HOME"/.ssh-app/run/sessions/*.json; do
  [ -f "$r" ] || continue
  grep -qF "\"tmux\":\"$1\"" "$r" 2>/dev/null || continue
  grep -qF "\"agent\":\"$hook\"" "$r" 2>/dev/null || continue
  ts=$(sed -n 's/.*"ts":\([0-9]*\).*/\1/p' "$r" | head -n 1)
  if [ "${ts:-0}" -gt "$bts" ]; then
    bts=${ts:-0}; sid=$(sed -n 's/.*"session":"\([^"]*\)".*/\1/p' "$r" | head -n 1)
  fi
done
case "$sid" in ""|*[!A-Za-z0-9_-]*) sid="";; esac
cwd=$(readlink "/proc/$apid/cwd" 2>/dev/null)
[ -n "$cwd" ] || cwd=$(lsof -a -p "$apid" -d cwd -Fn 2>/dev/null | sed -n 's/^n//p' | head -n 1)
case "$kind" in
"#;

const CLAUDE_ARM: &str = r#"    [ -n "$cwd" ] || exit 0
    slug=$(printf '%s' "$cwd" | sed 's/[^A-Za-z0-9]/-/g')
    # Claude keeps its files under $CLAUDE_CONFIG_DIR (the agent's own, read from its environment)
    # or ~/.claude; accounts set up as ~/.claude-<name> are looked at too. The newest file wins.
    cdir=$(ps eww -p "$apid" 2>/dev/null | tr ' ' '\n' | sed -n 's/^CLAUDE_CONFIG_DIR=//p' | head -n 1)
    f=""
    if [ -n "$sid" ]; then
      for d in "$cdir" "$HOME/.claude" "$HOME"/.claude-*; do
        [ -n "$d" ] || continue
        c=$(ls "$d"/projects/*/"$sid".jsonl 2>/dev/null | head -n 1)
        [ -f "$c" ] && f=$c && break
      done
      [ -f "$f" ] && { echo "$f"; exit 0; }
    fi
    for d in "$cdir" "$HOME/.claude" "$HOME"/.claude-*; do
      [ -n "$d" ] || continue
      c=$(ls -t "$d/projects/$slug"/*.jsonl 2>/dev/null | head -n 1)
      [ -f "$c" ] && { [ -z "$f" ] || [ "$c" -nt "$f" ]; } && f=$c
    done
    [ -f "$f" ] && echo "$f";;
"#;

const AGY_ARM: &str = r#"    # Only a folder of the agent's own brain/ counts: a project the agent has open may hold any path.
    B="$HOME/.gemini/antigravity-cli/brain"
    d=""
    [ -n "$sid" ] && [ -d "$B/$sid" ] && d="$B/$sid"
    [ -n "$d" ] || d=$(ls -l "/proc/$apid/fd" 2>/dev/null | awk -v b="$B/" '{ i=index($0, "-> " b); if (i > 0) { r=substr($0, i+3+length(b)); sub(/[\/ ].*/, "", r); if (r != "") { print b r; exit } } }')
    [ -n "$d" ] || d=$(lsof -p "$apid" -Fn 2>/dev/null | sed -n 's/^n//p' | awk -v b="$B/" 'index($0, b) == 1 { r=substr($0, length(b)+1); sub(/\/.*/, "", r); if (r != "") { print b r; exit } }')
    [ -n "$d" ] || d=$(ls -td "$B"/* 2>/dev/null | head -n 1)
    f="$d/.system_generated/logs/transcript.jsonl"
    [ -n "$d" ] && [ -f "$f" ] && echo "$f";;
"#;

const CODEX_ARM: &str = r#"    # Codex keeps one rollout file per session under $CODEX_HOME (the agent's own, read from its environment)
    # or ~/.codex: sessions/YYYY/MM/DD/rollout-<time>-<session id>.jsonl, whose first line names the folder.
    cdir=$(ps eww -p "$apid" 2>/dev/null | tr ' ' '\n' | sed -n 's/^CODEX_HOME=//p' | head -n 1)
    f=""
    for d in "$cdir" "$HOME/.codex"; do
      [ -n "$d" ] || continue
      if [ -n "$sid" ]; then
        c=$(ls "$d"/sessions/*/*/*/rollout-*-"$sid".jsonl 2>/dev/null | head -n 1)
        [ -f "$c" ] && f=$c && break
      fi
    done
    if [ ! -f "$f" ] && [ -n "$cwd" ]; then
      for d in "$cdir" "$HOME/.codex"; do
        [ -n "$d" ] || continue
        c=$(ls -t "$d"/sessions/*/*/*/rollout-*.jsonl 2>/dev/null | head -n 20 | while IFS= read -r c; do
          head -n 1 "$c" | grep -qF "\"cwd\":\"$cwd\"" && { echo "$c"; break; }
        done)
        [ -f "$c" ] && f=$c && break
      done
    fi
    [ -f "$f" ] && echo "$f";;
"#;

const GROK_ARM: &str = r#"    # Grok keeps one folder per session under $GROK_HOME (the agent's own, read from its environment) or ~/.grok:
    # sessions/<folder with / as %2F>/<session id>/chat_history.jsonl.
    gdir=$(ps eww -p "$apid" 2>/dev/null | tr ' ' '\n' | sed -n 's/^GROK_HOME=//p' | head -n 1)
    f=""
    enc=$(printf %s "$cwd" | sed 's#/#%2F#g; s# #%20#g')
    for d in "$gdir" "$HOME/.grok"; do
      [ -n "$d" ] || continue
      if [ -n "$sid" ]; then
        c=$(ls "$d"/sessions/*/"$sid"/chat_history.jsonl 2>/dev/null | head -n 1)
        [ -f "$c" ] && f=$c && break
      fi
    done
    if [ ! -f "$f" ] && [ -n "$cwd" ]; then
      for d in "$gdir" "$HOME/.grok"; do
        [ -n "$d" ] || continue
        c=$(ls -t "$d"/sessions/"$enc"/*/chat_history.jsonl 2>/dev/null | head -n 1)
        [ -f "$c" ] && f=$c && break
      done
    fi
    [ -f "$f" ] && echo "$f";;
"#;

const TAIL: &str = r#"esac
exit 0"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_source_has_a_registry_row_and_an_arm_in_the_script() {
        let script = locate_script();
        for source in sources() {
            let d = agent_descriptor(source.id().into()).expect("registry row");
            assert!(!d.process_names.is_empty(), "{}", source.id());
            assert!(
                script.contains(&format!("  {})\n", source.id())),
                "{}",
                source.id()
            );
            assert!(
                script.contains(&format!("print \"{} \" p", source.id())),
                "{}",
                source.id()
            );
            for name in &d.process_names {
                assert!(script.contains(&format!("b == \"{name}\"")), "{name}");
            }
        }
    }

    #[test]
    fn only_tidy_program_names_reach_the_script() {
        for ok in ["agy", "claude", "node.js", "a-b_c9"] {
            assert!(is_safe_program_name(ok), "{ok}");
        }
        for bad in ["", "A", "a b", "a\"b", "a/b", "a;b", "a$b", "a\nb", "a'b"] {
            assert!(!is_safe_program_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn source_ids_and_hook_names_are_unique_shell_words() {
        let mut ids = std::collections::HashSet::new();
        let mut hooks = std::collections::HashSet::new();
        for source in sources() {
            assert!(ids.insert(source.id()));
            assert!(hooks.insert(source.hook_agent()));
            for word in [source.id(), source.hook_agent()] {
                assert!(
                    word.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                    "{word}"
                );
            }
        }
    }
}
