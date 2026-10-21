use super::*;

/// What is in effect for an open session: its folder, the rules from the profile's settings and
/// from the folder's `.claude/settings*.json`, and the suggestions computed from them.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SessionPermissionsReport {
    /// The session's working folder (tmux pane), if the session exists.
    pub directory: Option<String>,
    /// `CLAUDE_CONFIG_DIR` of the Claude process running in this session, when it has one.
    pub detected_config_dir: Option<String>,
    pub user: Option<AgentPermissions>,
    pub project_shared: Option<AgentPermissions>,
    pub project_local: Option<AgentPermissions>,
    pub suggestions: Vec<PermissionSuggestion>,
}

pub(super) const INSPECT_FILE_CAP: usize = 200_000;

/// One exec that prints the pane's folder and the three settings files between markers. The
/// folder is only used when it has no control characters; files are read, never written, and only
/// when they are regular files (a FIFO planted as a project's settings file would block `head`).
pub(crate) fn build_inspect_permissions_command(
    session: &str,
    user_file_expr: Option<&str>,
    tag: &str,
) -> String {
    let sess = crate::tmux::shell_quote(&format!("={session}:"));
    // Without an explicit profile folder, the running Claude's own CLAUDE_CONFIG_DIR is used
    // (read from the environment of its process in this pane), else the default ~/.claude.
    let user_file = match user_file_expr {
        Some(expr) => format!("uf={expr}"),
        None => "uf=\"${cfg:-$HOME/.claude}/settings.json\"".to_string(),
    };
    // The section markers carry a per-call random tag: the settings files (a project's file is
    // untrusted) cannot contain a marker they could not know in advance.
    format!(
        "PATH=\"/opt/homebrew/bin:/usr/local/bin:$PATH\"; export PATH; \
         p=$(tmux display-message -p -t {sess} '#{{pane_current_path}}' 2>/dev/null); \
         case \"$p\" in *[![:print:]]*) p='';; esac; \
         cfg=''; pp=$(tmux display-message -p -t {sess} '#{{pane_pid}}' 2>/dev/null); \
         if [ -n \"$pp\" ]; then \
           for pid in $(ps -Ao pid=,ppid= | awk -v root=\"$pp\" '{{par[$1]=$2}} END{{for(x in par){{q=x;n=0;while((q in par)&&q!=root&&n<50){{q=par[q];n++}} if(q==root)print x}}}}'); do \
             c=$(ps -p \"$pid\" -o comm= 2>/dev/null); \
             case \"${{c##*/}}\" in claude*) ;; *) continue;; esac; \
             full=$(ps eww -p \"$pid\" -o command= 2>/dev/null); av=$(ps -p \"$pid\" -o command= 2>/dev/null); \
             env_part=${{full#\"$av\"}}; \
             v=$(printf '%s\\n' \"$env_part\" | tr ' ' '\\n' | sed -n 's/^CLAUDE_CONFIG_DIR=//p' | head -n 1); \
             case \"$v\" in /*) [ -d \"$v\" ] && {{ cfg=$v; break; }};; esac; \
           done; \
         fi; \
         case \"$cfg\" in *[![:print:]]*) cfg='';; esac; \
         {user_file}; \
         printf '==={tag}:DIR===\\n%s\\n==={tag}:CFG===\\n%s\\n==={tag}:USER===\\n' \"$p\" \"$cfg\"; \
         if [ -f \"$uf\" ]; then head -c {INSPECT_FILE_CAP} \"$uf\" 2>/dev/null; fi; \
         printf '\\n==={tag}:SHARED===\\n'; \
         if [ -f \"$p/.claude/settings.json\" ]; then head -c {INSPECT_FILE_CAP} \"$p/.claude/settings.json\" 2>/dev/null; fi; \
         printf '\\n==={tag}:LOCAL===\\n'; \
         if [ -f \"$p/.claude/settings.local.json\" ]; then head -c {INSPECT_FILE_CAP} \"$p/.claude/settings.local.json\" 2>/dev/null; fi; \
         printf '\\n==={tag}:END===\\n'"
    )
}

/// A fresh unguessable marker tag for one inspect call.
pub(crate) fn new_inspect_tag() -> String {
    format!(
        "{:016x}{:016x}",
        rand::random::<u64>(),
        rand::random::<u64>()
    )
}

pub(crate) fn parse_inspect_permissions_output(out: &str, tag: &str) -> SessionPermissionsReport {
    let section = |name: &str, next: &str| -> String {
        let start = format!("==={tag}:{name}===\n");
        let Some(i) = out.find(&start) else {
            return String::new();
        };
        let rest = &out[i + start.len()..];
        let end = rest
            .find(&format!("==={tag}:{next}==="))
            .unwrap_or(rest.len());
        rest[..end].trim().to_string()
    };
    let dir = section("DIR", "CFG");
    let cfg = section("CFG", "USER");
    let user = lenient_permissions(&section("USER", "SHARED"));
    let shared = lenient_permissions(&section("SHARED", "LOCAL"));
    let local = lenient_permissions(&section("LOCAL", "END"));
    let current = user.clone().unwrap_or_default();
    let context: Vec<AgentPermissions> = shared.iter().chain(local.iter()).cloned().collect();
    SessionPermissionsReport {
        directory: Some(dir).filter(|d| !d.is_empty() && !d.chars().any(char::is_control)),
        detected_config_dir: Some(cfg).filter(|d| {
            !d.is_empty() && d.chars().count() <= 300 && !d.chars().any(char::is_control)
        }),
        suggestions: suggest_permission_changes(current, context),
        user,
        project_shared: shared,
        project_local: local,
    }
}
