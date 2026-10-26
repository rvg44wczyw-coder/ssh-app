//! Sandboxed agent sessions (spec 62): the project and its agent run in a container that sees a
//! private clone of the project, a sandbox home and nothing else. This module is the pure planner:
//! it validates a profile and returns the shell commands the host runs (build, network, proxy,
//! clone, run, fetch, cleanup). Nothing here talks to an engine; every command is a closed set of
//! flags plus validated, quoted values, so a golden test can pin them.

use crate::docker::DOCKER_PATH;
use crate::error::SshCoreError;
use crate::tmux::shell_quote;

const MAX_HOSTS: usize = 64;
const PROXY_PORT: u16 = 3128;
/// The agent runs as the host user's uid/gid (never root), so files it writes in the shared sandbox
/// folders stay owned by the user on every host; resolved by the host's shell.
const USER_ARG: &str = "\"$(id -u):$(id -g)\"";
const IMAGE_UID: u32 = 1000;
/// Where the phone-approval hook finds its state inside the container.
const SBX_HOME: &str = "/ssh-app";
/// `debian:12-slim` as the registry served it on 2026-10-26 (OCI index digest). Updating it is a
/// reviewed change: resolve, read what changed, replace.
pub const DEFAULT_BASE_IMAGE: &str =
    "debian:12-slim@sha256:7ff6ba31527b9e336162fa061746c366c4b2032f4d2cff2078bea43f78efe3f8";

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SandboxEngine {
    Docker,
    Podman,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SandboxAgent {
    ClaudeCode,
    Antigravity,
    Codex,
    Grok,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SandboxNetwork {
    /// Only the allow-listing proxy is reachable; unknown hosts are refused (and asked about).
    Online,
    /// No network at all.
    Offline,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SandboxProfile {
    pub engine: SandboxEngine,
    pub agent: SandboxAgent,
    /// `[a-z0-9-]{3,32}`; names every resource of the sandbox.
    pub sandbox_id: String,
    /// The project on the host (absolute or `~/…`); only its committed content is copied.
    pub project_folder: String,
    pub network: SandboxNetwork,
    /// Extra hosts the proxy lets through, on top of the agent's own API hosts.
    pub allow_hosts: Vec<String>,
    pub cpus: u32,
    pub memory_mb: u32,
    pub pids: u32,
    /// `true`: the agent's own auto-approve flag. `false`: every approval goes to the phone.
    pub auto_approve: bool,
    /// The pinned base image, `name@sha256:<64 hex>`.
    pub base_image: String,
    /// Copy the agent's login file from the host into the sandbox home (owner's default: on).
    pub copy_login: bool,
    /// Claude Code's config folder on the host when it is not `~/.claude` (empty = the default).
    pub claude_config_dir: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SandboxPlan {
    pub image_tag: String,
    pub dockerfile: String,
    /// Builds the agent image (writes the context to a temporary folder first).
    pub build_command: String,
    /// Builds the proxy image (empty when the sandbox is offline).
    pub proxy_build_command: String,
    /// Creates volume, networks, proxy and the sandbox folders; idempotent.
    pub setup_commands: Vec<String>,
    /// Copies the committed content of the project into the work volume.
    pub clone_command: String,
    /// Copies the agent's login file into the sandbox home when it is not there yet (empty when off).
    pub login_command: String,
    /// The pane process: the agent in its container.
    pub run_command: String,
    /// Brings the sandbox's commits back under the `sandbox/<id>/` branch prefix (a bundle fetch).
    pub fetch_command: String,
    /// Removes containers, networks and the work volume (never the sandbox home).
    pub cleanup_commands: Vec<String>,
    /// The proxy's host filter, one regex per line.
    pub proxy_filter: String,
}

struct AgentSpec {
    binary: &'static str,
    install: &'static str,
    auto_flag: &'static str,
    /// Hosts the agent needs to log in and talk to its model: starting points from the vendors'
    /// documentation, widened per sandbox by phone approvals.
    hosts: &'static [&'static str],
}

fn spec(agent: SandboxAgent) -> AgentSpec {
    match agent {
        SandboxAgent::ClaudeCode => AgentSpec {
            binary: "claude",
            install: "npm install -g @anthropic-ai/claude-code",
            auto_flag: "--dangerously-skip-permissions",
            hosts: &[
                "api.anthropic.com",
                "console.anthropic.com",
                "platform.claude.com",
                "claude.ai",
            ],
        },
        SandboxAgent::Codex => AgentSpec {
            binary: "codex",
            install: "npm install -g @openai/codex",
            auto_flag: "--dangerously-bypass-approvals-and-sandbox",
            hosts: &["api.openai.com", "auth.openai.com", "chatgpt.com"],
        },
        SandboxAgent::Grok => AgentSpec {
            binary: "grok",
            install: "curl -fsSL https://x.ai/cli/install.sh | bash",
            auto_flag: "--always-approve",
            hosts: &["x.ai", "api.x.ai", "grok.com"],
        },
        SandboxAgent::Antigravity => AgentSpec {
            binary: "agy",
            install: "curl -fsSL https://antigravity.google/cli/install.sh | bash",
            auto_flag: "--dangerously-skip-permissions",
            hosts: &[
                "antigravity.google",
                "accounts.google.com",
                "*.googleapis.com",
            ],
        },
    }
}

fn invalid(reason: impl Into<String>) -> SshCoreError {
    SshCoreError::InvalidInput {
        reason: reason.into(),
    }
}

fn cli(engine: SandboxEngine) -> &'static str {
    match engine {
        SandboxEngine::Docker => "docker",
        SandboxEngine::Podman => "podman",
    }
}

fn valid_id(id: &str) -> bool {
    (3..=32).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-')
}

/// `name` or `*.name`: letters, digits, dots, dashes; at least two labels.
fn valid_host(h: &str) -> bool {
    let body = h.strip_prefix("*.").unwrap_or(h);
    body.len() <= 253
        && body.split('.').count() >= 2
        && body.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// Folders that hold the user's credentials or are too wide to share, by path component.
const FORBIDDEN_COMPONENTS: [&str; 10] = [
    ".ssh", ".gnupg", ".aws", ".azure", ".kube", ".docker", ".config", ".claude", ".codex", ".grok",
];

fn validate_folder(folder: &str) -> Result<(), SshCoreError> {
    let f = folder.trim();
    if f.is_empty() || f.len() > 400 || f.chars().any(|c| c.is_control()) {
        return Err(invalid("Invalid project folder"));
    }
    if !(f.starts_with('/') || f.starts_with("~/")) {
        return Err(invalid(
            "The project folder must be absolute or start with ~/",
        ));
    }
    let comps: Vec<&str> = f
        .trim_start_matches("~/")
        .split('/')
        .filter(|c| !c.is_empty())
        .collect();
    if comps.is_empty() || comps.contains(&"..") {
        return Err(invalid("The project folder is too wide or contains .."));
    }
    // A user's whole home (`/Users/x`, `/home/x`, `~`) is not a project.
    if f.starts_with('/')
        && comps.len() <= 2
        && matches!(comps.first(), Some(&"Users") | Some(&"home"))
        || comps.len() <= 1 && f.starts_with('/')
    {
        return Err(invalid(
            "Choose a project folder, not a home or system folder",
        ));
    }
    if comps.iter().any(|c| FORBIDDEN_COMPONENTS.contains(c)) {
        return Err(invalid("This folder holds credentials or agent state"));
    }
    Ok(())
}

/// The folder as the host's shell should see it: `~/x` becomes `"$HOME/x"`, quoted otherwise.
fn host_path(folder: &str) -> String {
    match folder.trim().strip_prefix("~/") {
        Some(rest) => format!("\"$HOME\"/{}", shell_quote(rest)),
        None => shell_quote(folder.trim()),
    }
}

/// The `debian:12-slim` base the app pins by default.
#[uniffi::export]
pub fn sandbox_default_base_image() -> String {
    DEFAULT_BASE_IMAGE.to_string()
}

/// The plan for `profile`, or why it is refused.
#[uniffi::export]
pub fn plan_sandbox_profile(profile: SandboxProfile) -> Result<SandboxPlan, SshCoreError> {
    plan_sandbox(&profile)
}

/// Changed paths of a sandbox branch that the host would run later (for the review screen).
#[uniffi::export]
pub fn sandbox_review_flags(changed: Vec<String>) -> Vec<String> {
    implicit_execution_paths(&changed)
}

/// Where the agent's hook config lives inside the sandbox home (`~/.ssh-app/sandboxes/<id>/home/…`)
/// and the config directory to hand to the hook installer. The hook script sits at
/// `/usr/local/bin/ssh-app-hook` in the image.
pub fn sandbox_hook_config_dir(p: &SandboxProfile) -> Result<String, SshCoreError> {
    validate_sandbox_profile(p)?;
    let rel = match p.agent {
        SandboxAgent::ClaudeCode => ".claude",
        SandboxAgent::Antigravity => ".gemini/config",
        SandboxAgent::Codex => ".codex",
        SandboxAgent::Grok => ".grok/hooks",
    };
    Ok(format!("~/.ssh-app/sandboxes/{}/home/{rel}", p.sandbox_id))
}

/// The in-image directory of the hook script.
pub const SANDBOX_HOOK_DIR: &str = "/usr/local/bin";

/// Checks a profile; the planner calls it first.
pub fn validate_sandbox_profile(p: &SandboxProfile) -> Result<(), SshCoreError> {
    if !valid_id(&p.sandbox_id) {
        return Err(invalid("The sandbox id must be 3-32 of a-z, 0-9 and -"));
    }
    validate_folder(&p.project_folder)?;
    if p.claude_config_dir.len() > 512
        || p.claude_config_dir.chars().any(|c| c.is_control())
        || p.claude_config_dir.split('/').any(|c| c == "..")
    {
        return Err(invalid("Invalid Claude config folder"));
    }
    if p.allow_hosts.len() > MAX_HOSTS || !p.allow_hosts.iter().all(|h| valid_host(h)) {
        return Err(invalid("Invalid allowed host"));
    }
    if !(1..=16).contains(&p.cpus)
        || !(256..=65_536).contains(&p.memory_mb)
        || !(16..=4096).contains(&p.pids)
    {
        return Err(invalid("Resource limits out of range"));
    }
    let (name, digest) = p
        .base_image
        .split_once("@sha256:")
        .ok_or_else(|| invalid("The base image must be pinned by digest"))?;
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"./_-:".contains(&b))
        || digest.len() != 64
        || !digest
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(invalid("Invalid base image"));
    }
    if p.auto_approve && spec(p.agent).auto_flag.is_empty() {
        return Err(invalid("This agent has no auto-approve mode"));
    }
    Ok(())
}

fn names(id: &str) -> (String, String, String, String, String) {
    let base = format!("ssh-app-sbx-{id}");
    (
        base.clone(),
        format!("{base}-work"),
        format!("{base}-int"),
        format!("{base}-out"),
        format!("{base}-proxy"),
    )
}

/// One anchored regex per host for the proxy filter.
pub fn proxy_filter(hosts: &[String]) -> String {
    hosts
        .iter()
        .map(|h| match h.strip_prefix("*.") {
            Some(rest) => format!("(^|\\.){}$", rest.replace('.', "\\.")),
            None => format!("^{}$", h.replace('.', "\\.")),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn dockerfile(p: &SandboxProfile) -> String {
    let s = spec(p.agent);
    // npm for the two npm agents; the curl installers need curl and ca-certificates.
    let packages = "git ca-certificates curl python3 openssh-client nodejs npm";
    format!(
        "FROM {base}\n\
         RUN apt-get update && apt-get install -y --no-install-recommends {packages} && rm -rf /var/lib/apt/lists/*\n\
         RUN useradd -m -u {IMAGE_UID} -s /bin/bash agent\n\
         COPY ssh-app-hook /usr/local/bin/ssh-app-hook\n\
         RUN chmod 0755 /usr/local/bin/ssh-app-hook\n\
         RUN mkdir -p /opt/agent && chown agent:agent /opt/agent\n\
         USER agent\n\
         ENV HOME=/opt/agent NPM_CONFIG_PREFIX=/opt/agent/.npm-global\n\
         ENV PATH=/opt/agent/.npm-global/bin:/opt/agent/.local/bin:/opt/agent/.grok/bin:/usr/local/bin:/usr/bin:/bin\n\
         RUN {install}\n\
         ENV HOME=/home/agent\n\
         USER root\n\
         RUN mkdir -p /work /home/agent && chmod 0777 /work /home/agent\n\
         USER agent\n\
         WORKDIR /work\n",
        base = p.base_image,
        install = s.install,
    )
}

/// The allow-listing proxy: tinyproxy that denies everything not in the filter and only tunnels 443.
fn proxy_dockerfile(base_image: &str) -> String {
    format!(
        "FROM {base_image}\n\
         RUN apt-get update && apt-get install -y --no-install-recommends tinyproxy && rm -rf /var/lib/apt/lists/*\n\
         COPY tinyproxy.conf /etc/tinyproxy/tinyproxy.conf\n"
    )
}

const PROXY_CONF: &str = "Port 3128\nListen 0.0.0.0\nTimeout 120\nLogLevel Info\n\
MaxClients 64\nConnectPort 443\nFilter \"/etc/tinyproxy/filter\"\nFilterURLs Off\nFilterExtended On\n\
FilterCaseSensitive Off\nFilterDefaultDeny Yes\nDisableViaHeader Yes\n";

fn image_tag(p: &SandboxProfile, df: &str) -> String {
    use sha2::{Digest, Sha256};
    let d = hex::encode(Sha256::digest(df.as_bytes()));
    format!("ssh-app-sbx-{}:{}", spec(p.agent).binary, &d[..12])
}

/// Egress approvals (spec 62 §6): the proxy refuses every host that is not in its filter and logs
/// the refusal; the phone reads those names and the owner decides. Only the phone's SSH session
/// runs these commands, never the agent, so a tap is the signature.
const MAX_DENIED: usize = 20;

/// The proxy's recent log (stdout and stderr) as one block of text.
pub fn egress_log_command(p: &SandboxProfile) -> Result<String, SshCoreError> {
    validate_sandbox_profile(p)?;
    let (_, _, _, _, proxy) = names(&p.sandbox_id);
    Ok(format!(
        "{DOCKER_PATH}; {} logs --tail 3000 {proxy} 2>&1",
        cli(p.engine)
    ))
}

/// Hosts the proxy refused, newest first, without duplicates, at most 20. Only plain DNS names are
/// kept (the name came from the agent: no wildcards, no IP literals, nothing but letters, digits,
/// dots and hyphens), and hosts that are already allowed are left out.
pub fn parse_denied_hosts(log: &str, allowed: &[String]) -> Vec<String> {
    const KEY: &str = "refused on filtered domain \"";
    let mut out: Vec<String> = Vec::new();
    for line in log.lines().rev() {
        let Some(at) = line.find(KEY) else { continue };
        let rest = &line[at + KEY.len()..];
        let Some(end) = rest.find('"') else { continue };
        let host = rest[..end].to_ascii_lowercase();
        let named = host
            .rsplit('.')
            .next()
            .is_some_and(|tld| tld.bytes().any(|b| b.is_ascii_alphabetic()));
        if host.starts_with("*.")
            || !valid_host(&host)
            || !named
            || out.contains(&host)
            || allowed.iter().any(|a| allows(a, &host))
        {
            continue;
        }
        out.push(host);
        if out.len() == MAX_DENIED {
            break;
        }
    }
    out
}

/// Whether an allow-list entry (`host` or `*.suffix`) covers `host`.
fn allows(entry: &str, host: &str) -> bool {
    match entry.strip_prefix("*.") {
        Some(suffix) => host == suffix || host.ends_with(&format!(".{suffix}")),
        None => entry == host,
    }
}

/// Reads the proxy filter back as host entries (`^a\.b$` is `a.b`, `(^|\.)b$` is `*.b`).
pub fn parse_allowed_hosts(filter: &str) -> Vec<String> {
    filter
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            let (wild, body) = match l.strip_prefix("(^|\\.)") {
                Some(r) => (true, r.strip_suffix('$')?),
                None => (false, l.strip_prefix('^')?.strip_suffix('$')?),
            };
            let host = body.replace("\\.", ".");
            valid_host(&host).then(|| if wild { format!("*.{host}") } else { host })
        })
        .take(MAX_HOSTS * 2)
        .collect()
}

/// The proxy filter file as it is now.
pub fn egress_allowed_command(p: &SandboxProfile) -> Result<String, SshCoreError> {
    validate_sandbox_profile(p)?;
    Ok(format!(
        "cat \"$HOME\"/.ssh-app/sandboxes/{}/proxy-filter",
        p.sandbox_id
    ))
}

/// Adds one host (or `*.suffix`, typed by the owner) to the sandbox's filter and makes the running proxy re-read it.
pub fn egress_allow_command(p: &SandboxProfile, host: &str) -> Result<String, SshCoreError> {
    validate_sandbox_profile(p)?;
    let host = host.trim().to_ascii_lowercase();
    if !valid_host(&host) {
        return Err(invalid("Invalid host name"));
    }
    let (_, _, _, _, proxy) = names(&p.sandbox_id);
    let line = proxy_filter(std::slice::from_ref(&host));
    Ok(format!(
        "{DOCKER_PATH}; set -e; f=\"$HOME\"/.ssh-app/sandboxes/{id}/proxy-filter; \
         [ -f \"$f\" ] && [ ! -L \"$f\" ]; \
         [ \"$(grep -c . \"$f\")\" -lt {cap} ]; \
         [ -z \"$(tail -c1 \"$f\")\" ] || printf '\\n' >> \"$f\"; \
         printf '%s\\n' {line} >> \"$f\"; {e} kill -s USR1 {proxy} >/dev/null",
        id = p.sandbox_id,
        cap = MAX_HOSTS * 2,
        line = shell_quote(&line),
        e = cli(p.engine),
    ))
}

/// Takes one entry out of the sandbox's filter (any entry, the agent's own hosts included) and
/// makes the running proxy re-read it. The file is rewritten in place: the container holds a
/// bind mount of this very file, a replaced file would not reach it.
pub fn egress_remove_command(p: &SandboxProfile, host: &str) -> Result<String, SshCoreError> {
    validate_sandbox_profile(p)?;
    let host = host.trim().to_ascii_lowercase();
    if !valid_host(&host) {
        return Err(invalid("Invalid host name"));
    }
    let (_, _, _, _, proxy) = names(&p.sandbox_id);
    let line = proxy_filter(std::slice::from_ref(&host));
    Ok(format!(
        "{DOCKER_PATH}; set -e; f=\"$HOME\"/.ssh-app/sandboxes/{id}/proxy-filter; \
         [ -f \"$f\" ] && [ ! -L \"$f\" ]; \
         t=$(mktemp); trap 'rm -f \"$t\"' EXIT; \
         grep -vxF -- {line} \"$f\" > \"$t\" || true; cat \"$t\" > \"$f\"; \
         {e} kill -s USR1 {proxy} >/dev/null",
        id = p.sandbox_id,
        line = shell_quote(&line),
        e = cli(p.engine),
    ))
}

/// Where each agent keeps the file that holds its login, relative to the config folder, and the
/// same path inside the sandbox home. Only this one file crosses, never the agent's config (hooks,
/// MCP servers and permissions in it would defeat the sandbox).
fn login_file(p: &SandboxProfile) -> (String, &'static str) {
    match p.agent {
        SandboxAgent::ClaudeCode => {
            let dir = if p.claude_config_dir.trim().is_empty() {
                "\"$HOME\"/.claude".to_string()
            } else {
                host_path(&p.claude_config_dir)
            };
            (
                format!("{dir}/.credentials.json"),
                ".claude/.credentials.json",
            )
        }
        SandboxAgent::Codex => ("\"$HOME\"/.codex/auth.json".into(), ".codex/auth.json"),
        SandboxAgent::Grok => ("\"$HOME\"/.grok/auth.json".into(), ".grok/auth.json"),
        SandboxAgent::Antigravity => (
            "\"$HOME\"/.gemini/antigravity-cli/antigravity-oauth-token".into(),
            ".gemini/antigravity-cli/antigravity-oauth-token",
        ),
    }
}

fn login_copy_command(p: &SandboxProfile) -> String {
    if !p.copy_login {
        return String::new();
    }
    let (src, rel) = login_file(p);
    let state = format!("\"$HOME\"/.ssh-app/sandboxes/{}/home", p.sandbox_id);
    // The copy is made by `cp` on the host: the content never reaches the phone. A symlink is not
    // followed, an existing (possibly refreshed) sandbox copy is kept, and the file is owner-only.
    format!(
        "set -e; umask 077; s={src}; d={state}/{rel}; \
         if [ -f \"$s\" ] && [ ! -L \"$s\" ] && [ ! -e \"$d\" ]; then \
         mkdir -p \"$(dirname \"$d\")\"; cp -- \"$s\" \"$d\"; fi"
    )
}

/// Everything the host runs for this sandbox. Pure: no engine is contacted.
pub fn plan_sandbox(p: &SandboxProfile) -> Result<SandboxPlan, SshCoreError> {
    validate_sandbox_profile(p)?;
    let e = cli(p.engine);
    let s = spec(p.agent);
    let (base, vol, int_net, out_net, proxy) = names(&p.sandbox_id);
    let df = dockerfile(p);
    let tag = image_tag(p, &df);
    let id = &p.sandbox_id;
    let folder = host_path(&p.project_folder);
    let state = format!("\"$HOME\"/.ssh-app/sandboxes/{id}");

    let mut hosts: Vec<String> = s.hosts.iter().map(|h| (*h).to_string()).collect();
    for h in &p.allow_hosts {
        if !hosts.contains(h) {
            hosts.push(h.clone());
        }
    }
    let filter = proxy_filter(&hosts);

    let hook = crate::agents::agent_hook_script();
    let build_command = format!(
        "{DOCKER_PATH}; set -e; d=$(mktemp -d); trap 'rm -rf \"$d\"' EXIT; \
         printf %s {df} > \"$d/Dockerfile\"; printf %s {hook} > \"$d/ssh-app-hook\"; \
         {e} build -t {tag} \"$d\"",
        df = shell_quote(&df),
        hook = shell_quote(&hook),
        tag = shell_quote(&tag),
    );

    let proxy_df = proxy_dockerfile(&p.base_image);
    let proxy_tag = {
        use sha2::{Digest, Sha256};
        format!(
            "ssh-app-sbx-proxy:{}",
            &hex::encode(Sha256::digest(format!("{proxy_df}{PROXY_CONF}").as_bytes()))[..12]
        )
    };
    let proxy_build_command = if p.network == SandboxNetwork::Online {
        format!(
            "{DOCKER_PATH}; set -e; d=$(mktemp -d); trap 'rm -rf \"$d\"' EXIT; \
             printf %s {} > \"$d/Dockerfile\"; printf %s {} > \"$d/tinyproxy.conf\"; \
             {e} build -t {} \"$d\"",
            shell_quote(&proxy_df),
            shell_quote(PROXY_CONF),
            shell_quote(&proxy_tag),
        )
    } else {
        String::new()
    };

    let mut setup = vec![format!(
        "{DOCKER_PATH}; set -e; umask 077; mkdir -p {state}/home {state}/run/pending {state}/run/decisions; \
         {e} volume create --label ssh-app.sandbox={id} {vol} >/dev/null"
    )];
    if p.network == SandboxNetwork::Online {
        setup.push(format!(
            "{DOCKER_PATH}; set -e; \
             {e} network create --internal --label ssh-app.sandbox={id} {int_net} >/dev/null; \
             {e} network create --label ssh-app.sandbox={id} {out_net} >/dev/null; \
             printf '%s\\n' {filter} > {state}/proxy-filter; chmod 0644 {state}/proxy-filter; \
             {e} run -d --rm --name {proxy} --label ssh-app.sandbox={id} --network {out_net} \
               --user 65534 --cap-drop=ALL --security-opt=no-new-privileges --read-only --tmpfs /tmp \
               --pids-limit 128 --memory 128m \
               -v {state}/proxy-filter:/etc/tinyproxy/filter:ro \
               {ptag} tinyproxy -d -c /etc/tinyproxy/tinyproxy.conf >/dev/null; \
             {e} network connect {int_net} {proxy}",
            filter = shell_quote(&filter),
            ptag = shell_quote(&proxy_tag),
        ));
    }

    // The project's committed content only: `git bundle` of HEAD, cloned inside; untracked and
    // ignored files, the host's .git/hooks and .git/config never leave the host.
    let clone_command = format!(
        "{DOCKER_PATH}; set -e; git -C {folder} rev-parse --verify HEAD >/dev/null; \
         git -C {folder} bundle create - HEAD 2>/dev/null | \
         {e} run --rm -i --user {USER_ARG} --network none --cap-drop=ALL --security-opt=no-new-privileges \
           -v {vol}:/work {tag} sh -c 'cat > /tmp/p.bundle && git clone -q /tmp/p.bundle /work/repo && git -C /work/repo checkout -q -b sandbox-work'",
        tag = shell_quote(&tag),
    );

    let mut run: Vec<String> = vec![
        format!("{DOCKER_PATH}; {e} run -it --rm"),
        format!("--name {base}"),
        format!("--label ssh-app.sandbox={id}"),
        format!("--user {USER_ARG}"),
        "--cap-drop=ALL --security-opt=no-new-privileges".into(),
        "--read-only --tmpfs /tmp:rw,nosuid,size=256m".into(),
        format!(
            "--pids-limit {} --memory {}m --cpus {}",
            p.pids, p.memory_mb, p.cpus
        ),
    ];
    match p.network {
        SandboxNetwork::Offline => run.push("--network none".into()),
        SandboxNetwork::Online => {
            run.push(format!("--network {int_net}"));
            let url = format!("http://{proxy}:{PROXY_PORT}");
            for var in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
                run.push(format!("-e {var}={}", shell_quote(&url)));
            }
            run.push("-e NO_PROXY=localhost,127.0.0.1".into());
        }
    }
    run.push(format!("-v {vol}:/work"));
    run.push(format!("-v {state}/home:/home/agent"));
    if !p.auto_approve {
        // The hook's state lives in a per-sandbox folder; the phone's public key is a separate
        // read-only file, and decisions are read-only: the agent cannot approve itself.
        run.push(format!("-e SSH_APP_HOME={SBX_HOME}"));
        run.push(format!("-v {state}/run/pending:{SBX_HOME}/run/pending"));
        run.push(format!(
            "-v {state}/run/decisions:{SBX_HOME}/run/decisions:ro"
        ));
        run.push(format!(
            "-v \"$HOME\"/.ssh-app/allowed_signers:{SBX_HOME}/allowed_signers:ro"
        ));
    }
    run.push("-w /work/repo".into());
    run.push(shell_quote(&tag));
    run.push(s.binary.to_string());
    if p.auto_approve {
        run.push(s.auto_flag.to_string());
    } else if p.agent == SandboxAgent::Codex {
        // Codex runs only hooks it was told to trust; the installer wrote ours into the sandbox
        // home, and the sandbox is where untrusted project hooks are meant to be contained.
        run.push("--dangerously-bypass-hook-trust".to_string());
    }
    let run_command = run.join(" ");

    let fetch_command = format!(
        "{DOCKER_PATH}; set -e; t=$(mktemp); trap 'rm -f \"$t\"' EXIT; \
         {e} run --rm --user {USER_ARG} --network none --cap-drop=ALL --security-opt=no-new-privileges \
           --read-only -v {vol}:/work:ro {tag} git -C /work/repo bundle create - --branches > \"$t\"; \
         git bundle verify \"$t\" >/dev/null; \
         git -C {folder} -c transfer.fsckObjects=true fetch --no-tags \"$t\" 'refs/heads/*:refs/heads/sandbox/{id}/*'",
        tag = shell_quote(&tag),
    );

    let mut cleanup = vec![format!(
        "{DOCKER_PATH}; {e} rm -f {base} >/dev/null 2>&1; {e} rm -f {proxy} >/dev/null 2>&1; \
         {e} network rm {int_net} {out_net} >/dev/null 2>&1; {e} volume rm {vol} >/dev/null 2>&1; true"
    )];
    cleanup.push(format!("rm -f {state}/proxy-filter"));

    let login_command = login_copy_command(p);

    Ok(SandboxPlan {
        image_tag: tag,
        dockerfile: df,
        build_command,
        proxy_build_command,
        setup_commands: setup,
        clone_command,
        login_command,
        run_command,
        fetch_command,
        cleanup_commands: cleanup,
        proxy_filter: filter,
    })
}

/// Paths in a sandbox branch that run on the host later (git hooks, build files, CI, IDE tasks,
/// agent configs including ours). The review screen puts them first.
pub fn implicit_execution_paths(changed: &[String]) -> Vec<String> {
    const PREFIXES: [&str; 14] = [
        ".git/",
        ".github/",
        ".gitlab-ci",
        ".vscode/",
        ".idea/",
        ".claude/",
        ".agents/",
        ".codex/",
        ".grok/",
        ".qwen/",
        ".opencode/",
        ".gemini/",
        ".husky/",
        ".circleci/",
    ];
    const NAMES: [&str; 14] = [
        "Makefile",
        "makefile",
        "package.json",
        "Cargo.toml",
        "build.rs",
        "setup.py",
        "pyproject.toml",
        "Dockerfile",
        "docker-compose.yml",
        ".envrc",
        ".gitmodules",
        ".gitattributes",
        "Rakefile",
        "justfile",
    ];
    changed
        .iter()
        .filter(|p| {
            let p = p.trim_start_matches("./");
            PREFIXES.iter().any(|x| p.starts_with(x))
                || p.contains("/.git/")
                || NAMES
                    .iter()
                    .any(|n| p == *n || p.ends_with(&format!("/{n}")))
                || p.ends_with(".sh")
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests;
