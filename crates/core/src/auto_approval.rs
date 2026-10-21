//! Auto-approval by command category (feature 50): categories of commands with a level per agent
//! and per project, presets, and the compile step that turns a policy into the rules the hook
//! enforces on the host.
//!
//! The policy lives in `HostGuard.auto` and in `~/.ssh-app/guard.json` (`"auto"`, for the app).
//! The hook never reads it: `compile_auto_rules` produces `auto_rules`, a list written next to
//! the user's own rules on every save, so nothing generated is ever read back as a user rule.
//! Everything here is typed by the user in the app; nothing an agent or a repository writes can
//! become a category level, a project or a rule.

use crate::agents::invalid;
use crate::error::SshCoreError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const MAX_PROJECTS: usize = 30;
const MAX_ROOT: usize = 400;
const MAX_LABEL: usize = 60;
pub const DEFAULT_TRUST_MAX_SECS: u32 = 86_400;
const MIN_TRUST_MAX_SECS: u32 = 60;

/// What happens to a command of a category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
pub enum AutoLevel {
    /// The default: asked on the phone like any other call.
    Ask,
    /// Allowed without a question (inside a project, past the boundary check).
    Auto,
    /// Refused on the host.
    Block,
}

/// How much harm a command can do.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, uniffi::Enum,
)]
#[serde(rename_all = "lowercase")]
pub enum AutoRisk {
    ReadOnly,
    LocalChange,
    /// Runs code from the repository (`build.rs`, Makefile, package scripts, tests).
    RunsProjectCode,
    Network,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AutoCommand {
    /// A guard rule, e.g. `Bash(cargo test:*)` or the tool name `Read`.
    pub rule: String,
    pub risk: AutoRisk,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AutoCategory {
    pub id: String,
    pub title: String,
    pub description: String,
    /// The highest risk among its commands.
    pub risk: AutoRisk,
    pub commands: Vec<AutoCommand>,
    /// The category covers the file-edit tools (`Edit`, `Write`, …) inside the project.
    pub edits_files: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct AutoCategoryLevel {
    pub category: String,
    pub level: AutoLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct AutoAgentOverride {
    /// The hook's name of the agent: `claude`, `antigravity`, `codex`, `qwen`, `crush`, `opencode`, `grok`.
    pub agent: String,
    pub levels: Vec<AutoCategoryLevel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct AutoProject {
    /// Absolute folder on the host.
    pub root: String,
    pub label: String,
    /// Only `Ask` or `Auto` here: a project cannot lift a Block.
    pub overrides: Vec<AutoCategoryLevel>,
    /// `Edit`/`Write` inside the root may be allowed (needs the `files` category on `Auto`).
    pub allow_file_edits: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct AutoPolicy {
    pub preset_id: Option<String>,
    /// Levels used inside every project; a category not listed is `Ask`.
    pub levels: Vec<AutoCategoryLevel>,
    pub agent_overrides: Vec<AutoAgentOverride>,
    pub projects: Vec<AutoProject>,
    /// The read-only `inspect` category without a project (still behind the floor).
    pub inspect_anywhere: bool,
    /// Longest temporary trust the host honours; `0` = the default (24 h).
    pub trust_max_secs: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AutoPreset {
    pub id: String,
    pub title: String,
    pub description: String,
    pub levels: Vec<AutoCategoryLevel>,
}

/// Agents the hook can allow for (it can answer "allow" to them), by the hook's agent name.
pub const ALLOW_AGENTS: [&str; 6] = [
    "claude",
    "antigravity",
    "codex",
    "qwen",
    "crush",
    "opencode",
];
const BLOCK_AGENTS: [&str; 7] = [
    "claude",
    "antigravity",
    "codex",
    "qwen",
    "crush",
    "opencode",
    "grok",
];

fn cmd(rule: &str, risk: AutoRisk, note: &str) -> AutoCommand {
    AutoCommand {
        rule: rule.to_string(),
        risk,
        note: note.to_string(),
    }
}

fn bash(prefix: &str, risk: AutoRisk, note: &str) -> AutoCommand {
    cmd(&format!("Bash({prefix}:*)"), risk, note)
}

fn category(
    id: &str,
    title: &str,
    description: &str,
    commands: Vec<AutoCommand>,
    edits_files: bool,
) -> AutoCategory {
    let risk = commands
        .iter()
        .map(|c| c.risk)
        .max()
        .unwrap_or(AutoRisk::ReadOnly);
    AutoCategory {
        id: id.into(),
        title: title.into(),
        description: description.into(),
        risk,
        commands,
        edits_files,
    }
}

/// The categories, narrow subcommands only: a program is never listed whole (`cargo test`, not
/// `cargo`), so `cargo run` and `cargo install` are not allowed by a build category.
#[uniffi::export]
pub fn auto_categories() -> Vec<AutoCategory> {
    use AutoRisk::{LocalChange as L, Network as N, ReadOnly as R, RunsProjectCode as P};
    let reads =
        |names: &[&str]| -> Vec<AutoCommand> { names.iter().map(|n| bash(n, R, "")).collect() };
    let mut inspect = vec![
        cmd("Read", R, "the Read tool"),
        cmd("Glob", R, "the Glob tool"),
        cmd("Grep", R, "the Grep tool"),
    ];
    inspect.extend(reads(&[
        "ls",
        "pwd",
        "cat",
        "head",
        "tail",
        "wc",
        "which",
        "file",
        "stat",
        "echo",
        "tree",
        "du",
        "grep",
        "git status",
        "git diff",
        "git log",
        "git show",
        "git branch --show-current",
        "git remote get-url",
        "git rev-parse",
    ]));
    vec![
        category(
            "inspect",
            "Read-only inspection",
            "Looking at files and git state: Read/Glob/Grep, ls, cat, git status/diff/log.",
            inspect,
            false,
        ),
        category(
            "git-local",
            "Git: local changes",
            "Staging, committing and switching branches. A commit runs the repository's own git hooks.",
            vec![
                bash("git add", L, ""),
                bash("git commit", P, "runs the repository's git hooks"),
                bash("git checkout", L, "can discard uncommitted changes"),
                bash("git switch", L, ""),
                bash("git stash", L, ""),
                bash("git restore", L, "can discard uncommitted changes"),
            ],
            false,
        ),
        category(
            "git-remote",
            "Git: remote",
            "Talking to a remote. Force pushes stay blocked either way.",
            vec![
                bash("git fetch", N, ""),
                bash("git pull", N, "merges remote changes into the working tree"),
                bash("git push", N, "publishes commits"),
            ],
            false,
        ),
        category(
            "rust",
            "Rust (cargo)",
            "Build, test and lint. These run the project's build scripts and tests.",
            vec![
                bash("cargo check", P, "runs build.rs and proc macros"),
                bash("cargo build", P, "runs build.rs and proc macros"),
                bash("cargo test", P, "runs the project's tests"),
                bash("cargo clippy", P, "runs build.rs and proc macros"),
                bash("cargo fmt", L, ""),
                bash("cargo doc", P, "runs build.rs and proc macros"),
                bash("rustc --version", R, ""),
            ],
            false,
        ),
        category(
            "python",
            "Python",
            "Tests and linters. They run the project's code.",
            vec![
                bash("python3 -m pytest", P, "runs the project's tests"),
                bash("python -m pytest", P, "runs the project's tests"),
                bash("pytest", P, "runs the project's tests"),
                bash("python3 -m unittest", P, "runs the project's tests"),
                bash("ruff check", R, ""),
                bash("ruff format", L, ""),
                bash("mypy", P, "can run plugins from the project"),
                bash("python3 --version", R, ""),
            ],
            false,
        ),
        category(
            "go",
            "Go",
            "Build, test and vet. These run the project's code.",
            vec![
                bash("go build", P, ""),
                bash("go test", P, "runs the project's tests"),
                bash("go vet", P, ""),
                bash("go fmt", L, ""),
                bash("gofmt", L, ""),
                bash("go list", R, ""),
                bash("go version", R, ""),
            ],
            false,
        ),
        category(
            "node",
            "Node",
            "Tests, lint and build scripts of the package (they run the package's scripts).",
            vec![
                bash("npm test", P, "runs the package's test script"),
                bash("npm run lint", P, "runs the package's script"),
                bash("npm run build", P, "runs the package's script"),
                bash("npm run test", P, "runs the package's script"),
                bash("pnpm test", P, "runs the package's test script"),
                bash("pnpm run lint", P, "runs the package's script"),
                bash("pnpm run build", P, "runs the package's script"),
                bash("yarn test", P, "runs the package's test script"),
                bash("yarn lint", P, "runs the package's script"),
                bash("yarn build", P, "runs the package's script"),
            ],
            false,
        ),
        category(
            "swift",
            "Swift / Xcode",
            "SwiftPM build and test. For `xcodebuild` add a rule by hand: its flags come before the action.",
            vec![
                bash("swift build", P, "runs package plugins"),
                bash("swift test", P, "runs the project's tests"),
                bash("xcodebuild -version", R, ""),
            ],
            false,
        ),
        category(
            "make",
            "Make / CMake",
            "Runs the repository's own recipes: anything the Makefile says.",
            vec![
                bash("make", P, "runs whatever the Makefile says"),
                bash("cmake --build", P, "runs the project's build"),
            ],
            false,
        ),
        category(
            "files",
            "Files: create, move, edit",
            "Creating, moving and editing files inside the project folder (never outside it).",
            vec![
                bash("mkdir", L, ""),
                bash("touch", L, ""),
                bash("cp", L, ""),
                bash("mv", L, ""),
                cmd("Edit", L, "the edit tools, inside the project"),
            ],
            true,
        ),
        category(
            "docker",
            "Docker (read-only)",
            "Listing containers and reading their logs.",
            reads(&[
                "docker ps",
                "docker logs",
                "docker images",
                "docker inspect",
                "docker version",
                "docker compose ps",
                "docker compose logs",
            ]),
            false,
        ),
        category(
            "network",
            "Web tools",
            "The agent's web search and fetch tools.",
            vec![
                cmd("WebFetch", N, "fetches a page"),
                cmd("WebSearch", N, "searches the web"),
            ],
            false,
        ),
        category(
            "install",
            "Package installs",
            "Installing dependencies downloads code and may run install scripts.",
            vec![
                bash("npm install", N, "downloads and may run install scripts"),
                bash("npm ci", N, "downloads and may run install scripts"),
                bash("pip install", N, "downloads and may run setup scripts"),
                bash("pip3 install", N, "downloads and may run setup scripts"),
                bash("python3 -m pip install", N, "downloads and may run setup scripts"),
                bash("cargo fetch", N, ""),
                bash("go mod download", N, ""),
                bash("swift package resolve", N, ""),
            ],
            false,
        ),
    ]
}

fn level(category: &str, level: AutoLevel) -> AutoCategoryLevel {
    AutoCategoryLevel {
        category: category.into(),
        level,
    }
}

fn autos(ids: &[&str]) -> Vec<AutoCategoryLevel> {
    ids.iter().map(|c| level(c, AutoLevel::Auto)).collect()
}

#[uniffi::export]
pub fn auto_presets() -> Vec<AutoPreset> {
    let cautious: Vec<AutoCategoryLevel> = autos(&["inspect", "docker"]);
    let mut developer = cautious.clone();
    developer.extend(autos(&["rust", "python", "go", "node", "swift"]));
    let mut trusted = developer.clone();
    trusted.extend(autos(&["git-local", "files", "make"]));
    vec![
        AutoPreset {
            id: "cautious".into(),
            title: "Cautious".into(),
            description: "Only looking is automatic: reading files, git status, listing containers.".into(),
            levels: cautious,
        },
        AutoPreset {
            id: "developer".into(),
            title: "Developer".into(),
            description: "Cautious, plus build, test and lint for Rust, Python, Go, Node and Swift inside your projects.".into(),
            levels: developer,
        },
        AutoPreset {
            id: "trusted-project".into(),
            title: "Trusted project".into(),
            description: "Developer, plus git commits, make and file edits inside your projects. For repositories you wrote or fully trust.".into(),
            levels: trusted,
        },
        AutoPreset {
            id: "lockdown".into(),
            title: "Lockdown".into(),
            description: "Nothing is automatic: every call is asked, the always-blocked list stays.".into(),
            levels: vec![],
        },
    ]
}

fn find_category<'a>(cats: &'a [AutoCategory], id: &str) -> Option<&'a AutoCategory> {
    cats.iter().find(|c| c.id == id)
}

fn check_levels(
    what: &str,
    cats: &[AutoCategory],
    list: &[AutoCategoryLevel],
    allow_block: bool,
) -> Result<Vec<AutoCategoryLevel>, SshCoreError> {
    let mut out: Vec<AutoCategoryLevel> = Vec::new();
    for l in list {
        if find_category(cats, &l.category).is_none() {
            return Err(invalid(format!(
                "{what}: unknown category '{}'",
                l.category
            )));
        }
        if !allow_block && l.level == AutoLevel::Block {
            return Err(invalid(format!(
                "{what}: a project cannot Block a category (it can only choose Ask or Auto)"
            )));
        }
        out.retain(|x| x.category != l.category);
        out.push(l.clone());
    }
    Ok(out)
}

fn clean_text(what: &str, s: &str, max: usize) -> Result<String, SshCoreError> {
    let s = s.trim();
    if s.chars().count() > max || s.chars().any(char::is_control) {
        return Err(invalid(format!(
            "{what} is too long or has control characters"
        )));
    }
    Ok(s.to_string())
}

/// Trims, de-duplicates and validates a policy; the result is what gets saved.
pub fn validate_auto_policy(policy: &AutoPolicy) -> Result<AutoPolicy, SshCoreError> {
    let cats = auto_categories();
    if let Some(id) = &policy.preset_id {
        if !auto_presets().iter().any(|p| &p.id == id) {
            return Err(invalid(format!("Unknown preset '{id}'")));
        }
    }
    let levels = check_levels("Levels", &cats, &policy.levels, true)?;
    let mut agent_overrides: Vec<AutoAgentOverride> = Vec::new();
    for o in &policy.agent_overrides {
        if !BLOCK_AGENTS.contains(&o.agent.as_str()) {
            return Err(invalid(format!("Unknown agent '{}'", o.agent)));
        }
        if o.levels.iter().any(|l| l.level == AutoLevel::Auto)
            && !ALLOW_AGENTS.contains(&o.agent.as_str())
        {
            return Err(invalid(format!(
                "{} cannot be allowed automatically",
                o.agent
            )));
        }
        agent_overrides.retain(|x| x.agent != o.agent);
        agent_overrides.push(AutoAgentOverride {
            agent: o.agent.clone(),
            levels: check_levels("Agent levels", &cats, &o.levels, true)?,
        });
    }
    if policy.projects.len() > MAX_PROJECTS {
        return Err(invalid(format!("At most {MAX_PROJECTS} projects")));
    }
    let mut projects: Vec<AutoProject> = Vec::new();
    for p in &policy.projects {
        let root = clean_text("Project folder", &p.root, MAX_ROOT)?;
        if !root.starts_with('/') || root.split('/').any(|s| s == "..") || root == "/" {
            return Err(invalid(
                "A project folder is an absolute path below the root, without '..'",
            ));
        }
        let root = root.trim_end_matches('/').to_string();
        let label = clean_text("Project name", &p.label, MAX_LABEL)?;
        projects.retain(|x| x.root != root);
        projects.push(AutoProject {
            label: if label.is_empty() {
                root.rsplit('/').next().unwrap_or("project").to_string()
            } else {
                label
            },
            root,
            overrides: check_levels("Project levels", &cats, &p.overrides, false)?,
            allow_file_edits: p.allow_file_edits,
        });
    }
    let trust = match policy.trust_max_secs {
        0 => 0,
        n @ MIN_TRUST_MAX_SECS..=DEFAULT_TRUST_MAX_SECS => n,
        _ => {
            return Err(invalid(
                "The longest temporary trust is between 1 minute and 24 hours",
            ))
        }
    };
    Ok(AutoPolicy {
        preset_id: policy.preset_id.clone(),
        levels,
        agent_overrides,
        projects,
        inspect_anywhere: policy.inspect_anywhere,
        trust_max_secs: trust,
    })
}

/// Checks and tidies a policy the app has edited (trims labels, drops duplicates, rejects a bad
/// folder or an unknown category).
#[uniffi::export]
pub fn auto_policy_validate(policy: AutoPolicy) -> Result<AutoPolicy, SshCoreError> {
    validate_auto_policy(&policy)
}

/// Starting policy of a preset (projects are kept as they are).
#[uniffi::export]
pub fn auto_policy_with_preset(
    current: AutoPolicy,
    preset_id: String,
) -> Result<AutoPolicy, SshCoreError> {
    let preset = auto_presets()
        .into_iter()
        .find(|p| p.id == preset_id)
        .ok_or_else(|| invalid(format!("Unknown preset '{preset_id}'")))?;
    let mut policy = current;
    policy.levels = preset.levels;
    policy.agent_overrides.clear();
    for p in &mut policy.projects {
        p.overrides.clear();
    }
    policy.preset_id = Some(preset.id);
    validate_auto_policy(&policy)
}

fn effective(
    policy: &AutoPolicy,
    agent: &str,
    project: Option<&AutoProject>,
    category: &str,
) -> AutoLevel {
    let pick = |list: &[AutoCategoryLevel]| {
        list.iter()
            .find(|l| l.category == category)
            .map(|l| l.level)
    };
    let default = pick(&policy.levels).unwrap_or(AutoLevel::Ask);
    let agent_level = policy
        .agent_overrides
        .iter()
        .find(|o| o.agent == agent)
        .and_then(|o| pick(&o.levels));
    let mut level = agent_level.unwrap_or(default);
    if level == AutoLevel::Block {
        return level;
    }
    if let Some(p) = project {
        if let Some(l) = pick(&p.overrides) {
            level = l;
        }
    }
    // An agent the hook cannot allow for never gets `Auto`.
    if level == AutoLevel::Auto && !ALLOW_AGENTS.contains(&agent) {
        return AutoLevel::Ask;
    }
    level
}

/// Rules that are always refused while anything is automatic: the existing "no destructive",
/// "no secrets", "no sudo" and "protect agent config" guard presets.
pub fn auto_floor() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in crate::host_guard::host_guard_presets() {
        if matches!(
            p.id.as_str(),
            "no-destructive" | "no-secrets" | "no-sudo" | "no-agent-config-edits"
        ) {
            for r in p.guard.deny {
                if !out.contains(&r) {
                    out.push(r);
                }
            }
        }
    }
    for r in [
        "Bash(chmod -R:*)",
        "Bash(chown:*)",
        "Edit(./.git/**)",
        "Edit(~/.ssh-app/**)",
    ] {
        if !out.iter().any(|x| x == r) {
            out.push(r.to_string());
        }
    }
    out
}

fn rules_of(cats: &[AutoCategory], ids: &[String], tools_only_edits: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for id in ids {
        if let Some(c) = find_category(cats, id) {
            for command in &c.commands {
                // The edit tools of `files` are emitted through the entry's `edits` flag (they
                // need a path check), not as a plain rule.
                if c.edits_files && command.rule == "Edit" && !tools_only_edits {
                    continue;
                }
                if !out.contains(&command.rule) {
                    out.push(command.rule.clone());
                }
            }
        }
    }
    out
}

fn group_by_agents(per_agent: Vec<(String, Value)>) -> Vec<(Vec<String>, Value)> {
    let mut groups: Vec<(Vec<String>, Value)> = Vec::new();
    for (agent, v) in per_agent {
        if let Some(g) = groups.iter_mut().find(|g| g.1 == v) {
            g.0.push(agent);
        } else {
            groups.push((vec![agent], v));
        }
    }
    groups
}

/// The `auto_rules` the hook reads: `[{agents?, root?, allow, deny, edits}]`.
pub fn compile_auto_rules(policy: &AutoPolicy) -> Result<Vec<Value>, SshCoreError> {
    let policy = validate_auto_policy(policy)?;
    let cats = auto_categories();
    let ids: Vec<String> = cats.iter().map(|c| c.id.clone()).collect();
    let mut entries: Vec<Value> = Vec::new();
    let mut any_auto = false;

    // Block: global, per agent, no project.
    let blocks = group_by_agents(
        BLOCK_AGENTS
            .iter()
            .map(|a| {
                let blocked: Vec<String> = ids
                    .iter()
                    .filter(|c| effective(&policy, a, None, c) == AutoLevel::Block)
                    .cloned()
                    .collect();
                let mut deny = rules_of(&cats, &blocked, true);
                if blocked
                    .iter()
                    .any(|c| find_category(&cats, c).is_some_and(|c| c.edits_files))
                {
                    for t in ["Write", "MultiEdit", "NotebookEdit"] {
                        deny.push(t.to_string());
                    }
                }
                ((*a).to_string(), json!(deny))
            })
            .filter(|(_, v)| v.as_array().is_some_and(|a| !a.is_empty()))
            .collect(),
    );
    for (agents, deny) in blocks {
        entries.push(json!({"agents": agents, "deny": deny, "allow": [], "edits": false}));
    }

    // Auto inside each project, per agent group.
    for project in &policy.projects {
        let per_agent: Vec<(String, Value)> = ALLOW_AGENTS
            .iter()
            .map(|a| {
                let autos: Vec<String> = ids
                    .iter()
                    .filter(|c| effective(&policy, a, Some(project), c) == AutoLevel::Auto)
                    .cloned()
                    .collect();
                let edits = project.allow_file_edits
                    && autos
                        .iter()
                        .any(|c| find_category(&cats, c).is_some_and(|c| c.edits_files));
                (
                    (*a).to_string(),
                    json!({"allow": rules_of(&cats, &autos, false), "edits": edits}),
                )
            })
            .filter(|(_, v)| {
                v["allow"].as_array().is_some_and(|a| !a.is_empty()) || v["edits"] == json!(true)
            })
            .collect();
        for (agents, v) in group_by_agents(per_agent) {
            any_auto = true;
            entries.push(json!({
                "agents": agents,
                "root": project.root,
                "allow": v["allow"],
                "deny": [],
                "edits": v["edits"],
            }));
        }
    }

    // Read-only anywhere.
    if policy.inspect_anywhere {
        let inspect = vec!["inspect".to_string()];
        let agents: Vec<String> = ALLOW_AGENTS
            .iter()
            .filter(|a| effective(&policy, a, None, "inspect") != AutoLevel::Block)
            .map(|a| (*a).to_string())
            .collect();
        if !agents.is_empty() {
            any_auto = true;
            entries.push(json!({
                "agents": agents,
                "allow": rules_of(&cats, &inspect, false),
                "deny": [],
                "edits": false,
            }));
        }
    }

    if any_auto || !policy.projects.is_empty() {
        entries.insert(
            0,
            json!({"deny": auto_floor(), "allow": [], "edits": false}),
        );
    }
    Ok(entries)
}

/// Things to read twice before applying a policy.
#[uniffi::export]
pub fn auto_policy_warnings(policy: AutoPolicy) -> Vec<String> {
    let mut out = Vec::new();
    let cats = auto_categories();
    let auto_somewhere = policy.levels.iter().any(|l| l.level == AutoLevel::Auto)
        || policy
            .agent_overrides
            .iter()
            .any(|o| o.levels.iter().any(|l| l.level == AutoLevel::Auto))
        || policy
            .projects
            .iter()
            .any(|p| p.overrides.iter().any(|l| l.level == AutoLevel::Auto));
    if auto_somewhere && policy.projects.is_empty() && !policy.inspect_anywhere {
        out.push("Automatic categories only work inside a project: add one, or nothing is allowed automatically.".into());
    }
    let level = |c: &str| {
        policy
            .levels
            .iter()
            .find(|l| l.category == c)
            .map(|l| l.level)
    };
    for c in &cats {
        if level(&c.id) == Some(AutoLevel::Auto) && c.risk >= AutoRisk::Network && c.id != "inspect"
        {
            out.push(format!(
                "\"{}\" is automatic: it reaches the network or downloads code, so a prompt-injected agent can too.",
                c.title
            ));
        }
    }
    if level("files") == Some(AutoLevel::Auto) && level("make") == Some(AutoLevel::Auto) {
        out.push("File edits and Make are both automatic: an agent can edit the Makefile and run it without a question. Use this only for repositories you trust completely.".into());
    }
    if policy.inspect_anywhere {
        out.push("Read-only inspection anywhere lets the agent read any file the always-blocked list does not name.".into());
    }
    out.push("Rules match the command as typed; they are not a sandbox. An allowed program can still run arbitrary code (cargo build runs build.rs, make runs the Makefile).".into());
    out
}

/// The `auto` object of guard.json (for the app; the hook ignores it).
pub fn policy_to_json(policy: &AutoPolicy) -> Value {
    serde_json::to_value(policy).unwrap_or(Value::Null)
}

pub fn policy_from_json(v: Option<&Value>) -> AutoPolicy {
    v.and_then(|v| serde_json::from_value::<AutoPolicy>(v.clone()).ok())
        .and_then(|p| validate_auto_policy(&p).ok())
        .unwrap_or_default()
}

/// Bytes the phone signs for a temporary trust: `SSH_APP_TRUST_V1:` and the fields, each as
/// `<utf8 length>:<bytes>\n`. The host rebuilds them from the stored blob.
#[allow(clippy::too_many_arguments)]
#[uniffi::export]
pub fn create_trust_signing_bytes(
    id: String,
    nonce: String,
    ts: u64,
    expires: u64,
    agent: String,
    session: String,
    root: String,
    edits: bool,
    rules: Vec<String>,
) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let rules_hash = hex::encode(Sha256::digest(rules.join("\n").as_bytes()));
    let fields = [
        id,
        nonce,
        ts.to_string(),
        expires.to_string(),
        agent,
        session,
        root,
        if edits { "true" } else { "false" }.to_string(),
        rules_hash,
    ];
    let mut out = b"SSH_APP_TRUST_V1:".to_vec();
    for f in fields {
        out.extend_from_slice(format!("{}:", f.len()).as_bytes());
        out.extend_from_slice(f.as_bytes());
        out.push(b'\n');
    }
    out
}

/// Categories a tool call belongs to (every part of a shell command must be covered by some
/// category, else none): the ones a temporary trust could cover. A command with a redirection or
/// a substitution has none.
#[uniffi::export]
pub fn auto_categories_for_call(tool: String, command: String) -> Vec<String> {
    let cats = auto_categories();
    let mut found: Vec<String> = Vec::new();
    let is_shell = matches!(
        tool.as_str(),
        "Bash" | "run_command" | "bash" | "shell" | "run_shell_command"
    );
    if !is_shell {
        for c in &cats {
            if c.commands.iter().any(|k| k.rule == tool) || (c.edits_files && is_edit_tool(&tool)) {
                found.push(c.id.clone());
            }
        }
        return found;
    }
    let plain = command
        .replace("2>&1", " ")
        .replace(">/dev/null", " ")
        .replace("&>/dev/null", " ");
    if plain.contains('>') || plain.contains("<(") || plain.contains("$(") || plain.contains('`') {
        return found;
    }
    for seg in plain.split(['\n', ';', '&', '|']) {
        let seg = seg.trim();
        if seg.is_empty() {
            continue;
        }
        let words = seg.split_whitespace().collect::<Vec<_>>().join(" ");
        let Some(cat) = cats.iter().find(|c| {
            c.commands.iter().any(|k| {
                k.rule
                    .strip_prefix("Bash(")
                    .and_then(|r| r.strip_suffix(":*)"))
                    .is_some_and(|prefix| {
                        words == prefix || words.starts_with(&format!("{prefix} "))
                    })
            })
        }) else {
            return vec![];
        };
        if !found.contains(&cat.id) {
            found.push(cat.id.clone());
        }
    }
    found
}

fn is_edit_tool(tool: &str) -> bool {
    matches!(tool, "Edit" | "Write" | "MultiEdit" | "NotebookEdit")
}

/// The rules a trust for these categories carries (signed with it), and whether it covers file
/// edits inside the project. Unknown ids are an error; `Destructive`-free by construction.
#[uniffi::export]
pub fn auto_trust_rules(category_ids: Vec<String>) -> Result<AutoTrustRules, SshCoreError> {
    let cats = auto_categories();
    for id in &category_ids {
        if find_category(&cats, id).is_none() {
            return Err(invalid(format!("Unknown category '{id}'")));
        }
    }
    let edits = category_ids
        .iter()
        .any(|id| find_category(&cats, id).is_some_and(|c| c.edits_files));
    Ok(AutoTrustRules {
        rules: rules_of(&cats, &category_ids, false),
        edits,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AutoTrustRules {
    pub rules: Vec<String>,
    pub edits: bool,
}

/// What the approval card can offer as "Approve, and allow … for …".
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AutoTrustOffer {
    pub categories: Vec<String>,
    pub project_root: String,
    pub project_label: String,
    /// Longest duration the host honours.
    pub max_secs: u32,
}

/// `None`: nothing can be trusted for this call (no project of the policy contains `cwd`, or the
/// command is in no category, or part of it is not).
#[uniffi::export]
pub fn auto_trust_offer(
    policy: AutoPolicy,
    tool: String,
    command: String,
    cwd: String,
) -> Option<AutoTrustOffer> {
    let policy = validate_auto_policy(&policy).ok()?;
    let categories = auto_categories_for_call(tool, command);
    if categories.is_empty() {
        return None;
    }
    let cwd = cwd.trim_end_matches('/');
    let project = policy
        .projects
        .iter()
        .filter(|p| cwd == p.root || cwd.starts_with(&format!("{}/", p.root)))
        .max_by_key(|p| p.root.len())?;
    Some(AutoTrustOffer {
        categories,
        project_root: project.root.clone(),
        project_label: project.label.clone(),
        max_secs: trust_cap(&policy),
    })
}

/// An active temporary trust on the host (`ctl trusts`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AutoTrust {
    pub id: String,
    pub agent: String,
    pub session: String,
    pub root: String,
    pub expires_sec: u64,
    pub edits: bool,
    pub rules: Vec<String>,
}

/// One automatic decision of the hook (`ctl auto-log`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AutoLogEntry {
    pub timestamp_sec: u64,
    pub agent: String,
    pub tool: String,
    pub summary: String,
    /// `allow`, `trusted` or `deny`.
    pub decision: String,
    pub rule: String,
    pub root: String,
}

pub(crate) fn parse_trusts(output: &str) -> Vec<AutoTrust> {
    use crate::approval_queue::display_safe;
    let Ok(v) = serde_json::from_str::<Value>(output.trim()) else {
        return vec![];
    };
    let text = |i: &Value, k: &str, n: usize| {
        display_safe(
            &i.get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .chars()
                .take(n)
                .collect::<String>(),
        )
    };
    v.get("items")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .take(50)
                .filter_map(|i| {
                    let id = text(i, "id", 32);
                    (crate::approval_queue::is_valid_id(&id)).then(|| AutoTrust {
                        id,
                        agent: text(i, "agent", 40),
                        session: text(i, "session", 80),
                        root: text(i, "root", 300),
                        expires_sec: i.get("expires").and_then(Value::as_u64).unwrap_or(0),
                        edits: i.get("edits").and_then(Value::as_bool).unwrap_or(false),
                        rules: i
                            .get("rules")
                            .and_then(Value::as_array)
                            .map(|r| {
                                r.iter()
                                    .filter_map(Value::as_str)
                                    .take(12)
                                    .map(|x| display_safe(&x.chars().take(120).collect::<String>()))
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn parse_auto_log(output: &str) -> Vec<AutoLogEntry> {
    use crate::approval_queue::display_safe;
    let Ok(v) = serde_json::from_str::<Value>(output.trim()) else {
        return vec![];
    };
    let text = |i: &Value, k: &str, n: usize| {
        display_safe(
            &i.get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .chars()
                .take(n)
                .collect::<String>(),
        )
    };
    v.get("items")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .take(50)
                .map(|i| AutoLogEntry {
                    timestamp_sec: i.get("ts").and_then(Value::as_u64).unwrap_or(0),
                    agent: text(i, "agent", 40),
                    tool: text(i, "tool", 60),
                    summary: text(i, "summary", 200),
                    decision: text(i, "decision", 12),
                    rule: text(i, "rule", 120),
                    root: text(i, "root", 300),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The temporary-trust cap the host enforces.
pub fn trust_cap(policy: &AutoPolicy) -> u32 {
    if policy.trust_max_secs == 0 {
        DEFAULT_TRUST_MAX_SECS
    } else {
        policy.trust_max_secs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(preset: &str, root: &str) -> AutoPolicy {
        let p = AutoPolicy {
            projects: vec![AutoProject {
                root: root.into(),
                label: "p".into(),
                overrides: vec![],
                allow_file_edits: true,
            }],
            ..Default::default()
        };
        auto_policy_with_preset(p, preset.into()).unwrap()
    }

    #[test]
    fn no_category_rule_is_a_whole_program_except_make_and_tools() {
        for c in auto_categories() {
            for k in &c.commands {
                let spec = k
                    .rule
                    .strip_prefix("Bash(")
                    .and_then(|r| r.strip_suffix(":*)"));
                match spec {
                    None => {} // a tool name
                    Some(s) => {
                        let one_word = !s.contains(' ');
                        // single words are read-only inspectors, formatters or the explicit make
                        let ok = !one_word
                            || matches!(
                                s,
                                "ls" | "pwd"
                                    | "cat"
                                    | "head"
                                    | "tail"
                                    | "wc"
                                    | "which"
                                    | "file"
                                    | "stat"
                                    | "echo"
                                    | "tree"
                                    | "du"
                                    | "grep"
                                    | "pytest"
                                    | "mypy"
                                    | "gofmt"
                                    | "make"
                                    | "mkdir"
                                    | "touch"
                                    | "cp"
                                    | "mv"
                            );
                        assert!(ok, "{} is a whole program", k.rule);
                    }
                }
            }
        }
    }

    #[test]
    fn nothing_destructive_is_in_any_category() {
        let floor = auto_floor();
        for c in auto_categories() {
            for k in &c.commands {
                assert!(
                    !floor.contains(&k.rule),
                    "{} is on the always-blocked list",
                    k.rule
                );
                for bad in ["rm", "sudo", "dd", "mkfs", "chmod", "chown", "curl", "wget"] {
                    assert!(!k.rule.starts_with(&format!("Bash({bad}")), "{}", k.rule);
                }
            }
        }
    }

    #[test]
    fn presets_and_levels_use_known_categories() {
        let cats = auto_categories();
        for p in auto_presets() {
            for l in &p.levels {
                assert!(
                    find_category(&cats, &l.category).is_some(),
                    "{}",
                    l.category
                );
            }
        }
    }

    #[test]
    fn developer_allows_builds_in_the_project_only_and_keeps_the_floor() {
        let rules = compile_auto_rules(&policy("developer", "/Users/me/proj")).unwrap();
        // the floor comes first
        assert!(rules[0]["deny"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == "Bash(rm:*)"));
        assert!(rules[0].get("root").is_none());
        let project = rules
            .iter()
            .find(|e| e["root"] == "/Users/me/proj")
            .unwrap();
        let allow: Vec<&str> = project["allow"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.as_str().unwrap())
            .collect();
        assert!(allow.contains(&"Bash(cargo test:*)"));
        assert!(allow.contains(&"Read"));
        assert!(
            !allow.contains(&"Bash(git commit:*)"),
            "git-local is Ask in Developer"
        );
        assert_eq!(project["edits"], false, "files is Ask in Developer");
    }

    #[test]
    fn trusted_project_allows_edits_inside_and_a_project_cannot_lift_a_block() {
        let rules = compile_auto_rules(&policy("trusted-project", "/w/p")).unwrap();
        let project = rules.iter().find(|e| e["root"] == "/w/p").unwrap();
        assert_eq!(project["edits"], true);
        assert!(project["allow"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r != "Edit"));

        let mut p = policy("developer", "/w/p");
        p.levels.push(level("network", AutoLevel::Block));
        p.projects[0]
            .overrides
            .push(level("network", AutoLevel::Auto));
        let rules = compile_auto_rules(&p).unwrap();
        let denies: Vec<&Value> = rules
            .iter()
            .filter(|e| e.get("root").is_none() && !e["deny"].as_array().unwrap().is_empty())
            .collect();
        assert!(denies.iter().any(|e| e["deny"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == "WebFetch")));
        let project = rules.iter().find(|e| e["root"] == "/w/p").unwrap();
        let allow: Vec<&str> = project["allow"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.as_str().unwrap())
            .collect();
        // a Block is settled before the project is looked at: it is not even offered as allowed
        assert!(!allow.contains(&"WebFetch"));
        // but a project may not even *state* a Block
        let mut bad = policy("developer", "/w/p");
        bad.projects[0]
            .overrides
            .push(level("rust", AutoLevel::Block));
        assert!(validate_auto_policy(&bad).is_err());
    }

    #[test]
    fn agents_that_cannot_be_allowed_never_get_auto() {
        let mut p = policy("developer", "/w/p");
        p.agent_overrides.push(AutoAgentOverride {
            agent: "grok".into(),
            levels: vec![level("rust", AutoLevel::Auto)],
        });
        assert!(validate_auto_policy(&p).is_err());
        let rules = compile_auto_rules(&policy("developer", "/w/p")).unwrap();
        for e in rules.iter().filter(|e| e.get("root").is_some()) {
            let agents = e["agents"].as_array().unwrap();
            assert!(agents.iter().all(|a| a != "grok"));
        }
    }

    #[test]
    fn lockdown_and_empty_policies_compile_to_nothing() {
        assert!(compile_auto_rules(&AutoPolicy::default())
            .unwrap()
            .is_empty());
        // Lockdown with a project keeps only the always-blocked list: nothing is allowed.
        let rules = compile_auto_rules(&policy("lockdown", "/w/p")).unwrap();
        assert_eq!(rules.len(), 1);
        assert!(rules[0]["allow"].as_array().unwrap().is_empty());
    }

    #[test]
    fn calls_map_to_categories_only_when_every_part_is_covered() {
        let cat = |c: &str| auto_categories_for_call("Bash".into(), c.into());
        assert_eq!(cat("cargo test -p x"), vec!["rust"]);
        assert_eq!(cat("cargo fmt && git status"), vec!["rust", "inspect"]);
        assert!(cat("cargo test; rm -rf x").is_empty());
        assert!(cat("cargo test > out.txt").is_empty());
        assert!(cat("ls $(rm x)").is_empty());
        assert!(cat("cargo run").is_empty(), "cargo run is in no category");
        assert_eq!(
            auto_categories_for_call("Edit".into(), String::new()),
            vec!["files"]
        );
    }

    #[test]
    fn trust_bytes_are_length_prefixed_and_not_an_approval() {
        let b = create_trust_signing_bytes(
            "id".into(),
            "n".into(),
            1,
            2,
            "claude".into(),
            "s".into(),
            "/p".into(),
            false,
            vec!["Bash(cargo test:*)".into()],
        );
        assert!(b.starts_with(b"SSH_APP_TRUST_V1:2:id\n1:n\n1:1\n1:2\n6:claude\n"));
        // moving a byte between two fields changes the message
        let c = create_trust_signing_bytes(
            "i".into(),
            "dn".into(),
            1,
            2,
            "claude".into(),
            "s".into(),
            "/p".into(),
            false,
            vec!["Bash(cargo test:*)".into()],
        );
        assert_ne!(b, c);
    }

    #[test]
    fn a_trust_is_offered_only_inside_a_project_for_covered_commands() {
        let p = policy("developer", "/w/p");
        let offer = |cmd: &str, cwd: &str| {
            auto_trust_offer(p.clone(), "Bash".into(), cmd.into(), cwd.into())
        };
        let o = offer("cargo test", "/w/p/sub").unwrap();
        assert_eq!(o.categories, vec!["rust"]);
        assert_eq!(o.project_root, "/w/p");
        assert_eq!(o.max_secs, DEFAULT_TRUST_MAX_SECS);
        assert!(offer("cargo test", "/w/other").is_none());
        assert!(offer("/w/p-evil/x", "/w/p-evil").is_none());
        assert!(offer("cargo test; curl x", "/w/p").is_none());
        assert!(offer("rm -rf x", "/w/p").is_none());
    }

    #[test]
    fn host_output_is_parsed_defensively() {
        let id = "a".repeat(32);
        let t = parse_trusts(&format!(
            r#"{{"items":[{{"id":"{id}","agent":"claude","session":"s","root":"/p","expires":5,"edits":true,"rules":["Read"]}},{{"id":"bad"}}]}}"#
        ));
        assert_eq!(t.len(), 1);
        assert!(t[0].edits);
        assert!(parse_trusts("garbage").is_empty());
        assert_eq!(
            parse_auto_log(r#"{"items":[{"ts":3,"decision":"allow"}]}"#).len(),
            1
        );
        assert!(parse_auto_log("").is_empty());
    }

    #[test]
    fn project_roots_are_checked() {
        for bad in ["relative/dir", "/", "/a/../b", "/a\nb"] {
            let p = AutoPolicy {
                projects: vec![AutoProject {
                    root: bad.into(),
                    label: "x".into(),
                    overrides: vec![],
                    allow_file_edits: false,
                }],
                ..Default::default()
            };
            assert!(validate_auto_policy(&p).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_policy_survives_json() {
        let p = policy("developer", "/w/p");
        let back = policy_from_json(Some(&policy_to_json(&p)));
        assert_eq!(back, validate_auto_policy(&p).unwrap());
        assert_eq!(
            policy_from_json(Some(&json!({"levels": "nonsense"}))),
            AutoPolicy::default()
        );
    }
}
