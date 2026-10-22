//! Container runtimes other than plain Docker (feature 51): Podman and nerdctl (the "engine
//! family"), Compose stacks, Kubernetes pods through `kubectl`, and Incus/LXD instances. Every
//! command is built here from a closed runtime enum and identifiers checked by a validator; the
//! only text taken from the host is cleaned before it is shown.
//!
//! Only Docker has been seen on a real host. The others are built on documented output formats
//! (Go templates, `--format csv`, `kubectl -o json`) and tested with fixtures taken from that
//! documentation.

use crate::docker::{
    clean, docker_script, parse_docker_list, parse_size, str_of, validate_container_identifier,
    ContainerState, DockerAction, DockerContainerInfo, DockerContainerStats,
    DockerContainersSnapshot, DockerUnavailable, DOCKER_PATH, LOG_DEFAULT_TAIL, LOG_MAX_BYTES,
    LOG_MAX_TAIL, MAX_CONTAINERS,
};
use crate::error::SshCoreError;
use crate::tmux::shell_quote;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Separates the section of Compose labels from the container list in one Docker call.
const PROJECTS_MARKER: &str = "---PROJECTS---";
/// Pods are read up to this many bytes (under the 4 MB the exec channel keeps); a longer document
/// is refused, not guessed from.
const KUBECTL_MAX_BYTES: usize = 3 * 1024 * 1024;
/// A stack action covers at most this many containers.
pub const MAX_STACK: usize = 20;
const ACTION_OK: &str = "ACTION_OK";
/// The script's own watchdog, shorter than the 120 s the client waits, so the answer arrives.
const STACK_SECS: u32 = 110;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ContainerRuntime {
    Docker,
    Podman,
    Nerdctl,
    Kubernetes,
    Incus,
    Lxd,
}

impl ContainerRuntime {
    /// The command on the host. A closed set: never taken from input.
    fn cli(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Podman => "podman",
            Self::Nerdctl => "nerdctl",
            Self::Kubernetes => "kubectl",
            Self::Incus => "incus",
            Self::Lxd => "lxc",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Docker => "Docker",
            Self::Podman => "Podman",
            Self::Nerdctl => "nerdctl",
            Self::Kubernetes => "Kubernetes",
            Self::Incus => "Incus",
            Self::Lxd => "LXD",
        }
    }

    fn is_engine(self) -> bool {
        matches!(self, Self::Docker | Self::Podman | Self::Nerdctl)
    }

    fn is_incus_family(self) -> bool {
        matches!(self, Self::Incus | Self::Lxd)
    }

    /// Prefix of the tmux session that keeps a shell alive.
    fn session_prefix(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Podman => "podman",
            Self::Nerdctl => "nerdctl",
            Self::Kubernetes => "k8s",
            Self::Incus => "incus",
            Self::Lxd => "lxd",
        }
    }
}

/// What the screen may offer for a runtime; the UI reads this instead of guessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct RuntimeCaps {
    pub stats: bool,
    pub restart: bool,
    pub start_stop: bool,
    pub logs: bool,
    pub shell: bool,
    pub groups: bool,
}

#[uniffi::export]
pub fn runtime_caps(runtime: ContainerRuntime) -> RuntimeCaps {
    let engine = runtime.is_engine();
    RuntimeCaps {
        stats: engine,
        restart: true,
        start_stop: runtime != ContainerRuntime::Kubernetes,
        logs: true,
        shell: true,
        groups: engine || runtime == ContainerRuntime::Kubernetes,
    }
}

fn invalid(reason: &str) -> SshCoreError {
    crate::docker::invalid(reason)
}

fn unsupported(runtime: ContainerRuntime, what: &str) -> SshCoreError {
    invalid(&format!("{} has no {what}", runtime.label()))
}

// ---------- probe ----------

/// Prints `RUNTIME:<command>` for each command the host has.
#[uniffi::export]
pub fn runtime_probe_command() -> String {
    docker_script(
        "for c in docker podman nerdctl kubectl incus lxc; do command -v \"$c\" >/dev/null 2>&1 && echo \"RUNTIME:$c\"; done; true",
        10,
    )
}

/// The runtimes found, in detection order. `lxc` counts only when `incus` is absent (the older name
/// of the same commands). Lines that are not a known token are ignored.
#[uniffi::export]
pub fn parse_runtime_probe(output: String) -> Vec<ContainerRuntime> {
    let found: Vec<&str> = output
        .lines()
        .filter_map(|l| l.trim().strip_prefix("RUNTIME:"))
        .collect();
    let has = |c: &str| found.contains(&c);
    let mut out = Vec::new();
    for (cmd, rt) in [
        ("docker", ContainerRuntime::Docker),
        ("podman", ContainerRuntime::Podman),
        ("nerdctl", ContainerRuntime::Nerdctl),
        ("kubectl", ContainerRuntime::Kubernetes),
        ("incus", ContainerRuntime::Incus),
    ] {
        if has(cmd) {
            out.push(rt);
        }
    }
    if has("lxc") && !has("incus") {
        out.push(ContainerRuntime::Lxd);
    }
    out
}

// ---------- identifiers ----------

fn dns_label(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && s.ends_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn dns_subdomain(s: &str) -> bool {
    s.len() <= 253 && !s.is_empty() && s.split('.').all(|part| dns_label(part, 63))
}

/// `namespace/pod`, both DNS-1123 (a namespace is a label, a pod name a subdomain).
pub fn validate_pod_ref(id: &str) -> Result<(String, String), SshCoreError> {
    match id.split_once('/') {
        Some((ns, pod)) if dns_label(ns, 63) && dns_subdomain(pod) => {
            Ok((ns.to_string(), pod.to_string()))
        }
        _ => Err(invalid("Invalid pod reference")),
    }
}

/// What a command may be given for this runtime, checked and ready to quote.
enum Target {
    Plain(String),
    Pod { ns: String, name: String },
}

fn target(runtime: ContainerRuntime, id: &str) -> Result<Target, SshCoreError> {
    if runtime == ContainerRuntime::Kubernetes {
        let (ns, name) = validate_pod_ref(id)?;
        Ok(Target::Pod { ns, name })
    } else {
        Ok(Target::Plain(validate_container_identifier(
            id.to_string(),
        )?))
    }
}

// ---------- list ----------

const US: char = '\u{1f}';

/// One container per line, fields split on U+001F. Every value but the labels is generated by the
/// engine from a restricted alphabet (names, image references, states); a label is free text, so it
/// goes through the template's `json` function, which escapes newlines and U+001F: a hostile label
/// can then neither hide its container (extra separator) nor forge another entry (extra line).
/// The separator is a template action: a bare `\x1f` outside an action is not interpreted by the
/// formatter (checked against Docker).
fn family_template() -> &'static str {
    r#"ID={{.ID}}{{"\x1f"}}NAME={{.Names}}{{"\x1f"}}IMAGE={{.Image}}{{"\x1f"}}STATE={{.State}}{{"\x1f"}}STATUS={{.Status}}{{"\x1f"}}PORTS={{.Ports}}{{"\x1f"}}CREATED={{.CreatedAt}}{{"\x1f"}}PROJECT={{json (print (.Label "com.docker.compose.project") (.Label "io.podman.compose.project"))}}{{"\x1f"}}SERVICE={{json (print (.Label "com.docker.compose.service") (.Label "io.podman.compose.service"))}}"#
}

/// The same without the labels, for a runtime that rejects the template above: stacks are lost,
/// the list is not.
fn family_template_plain() -> &'static str {
    r#"ID={{.ID}}{{"\x1f"}}NAME={{.Names}}{{"\x1f"}}IMAGE={{.Image}}{{"\x1f"}}STATE={{.State}}{{"\x1f"}}STATUS={{.Status}}{{"\x1f"}}PORTS={{.Ports}}{{"\x1f"}}CREATED={{.CreatedAt}}"#
}

/// The call that lists everything the runtime manages.
#[uniffi::export]
pub fn container_list_command(runtime: ContainerRuntime) -> String {
    docker_script(&list_body(runtime), 15)
}

fn list_body(runtime: ContainerRuntime) -> String {
    let cli = runtime.cli();
    let t = family_template();
    match runtime {
        ContainerRuntime::Docker => format!(
            "if command -v docker >/dev/null 2>&1; then docker ps -a --format '{{{{json .}}}}' 2>&1; echo {PROJECTS_MARKER}; docker ps -a --format '{t}' 2>/dev/null; else echo DOCKER_NOT_INSTALLED; fi"
        ),
        ContainerRuntime::Podman | ContainerRuntime::Nerdctl => format!(
            "if command -v {cli} >/dev/null 2>&1; then \
             out=$({cli} ps -a --format '{t}' 2>&1); \
             case \"$out\" in *ID=*) printf '%s\\n' \"$out\";; \
             *) {cli} ps -a --format '{plain}' 2>&1;; esac; \
             else echo RUNTIME_NOT_INSTALLED; fi",
            plain = family_template_plain()
        ),
        // Without a kubeconfig kubectl falls back to localhost:8080 and says "connection refused",
        // which reads like a down cluster; so the context is asked for first (a pod running inside
        // the cluster has none and does not need one).
        ContainerRuntime::Kubernetes => format!(
            "if command -v kubectl >/dev/null 2>&1; then \
             if [ -z \"$KUBERNETES_SERVICE_HOST\" ] && ! kubectl config current-context >/dev/null 2>&1; then echo KUBE_NO_CONTEXT; else \
             out=$(kubectl get pods --all-namespaces -o json 2>&1); \
             case \"$out\" in '{{'*) ;; *Forbidden*|*forbidden*) echo KUBE_CURRENT_NAMESPACE; out=$(kubectl get pods -o json 2>&1);; esac; \
             printf '%s\\n' \"$out\" | head -c {KUBECTL_MAX_BYTES}; fi; \
             else echo RUNTIME_NOT_INSTALLED; fi"
        ),
        ContainerRuntime::Incus | ContainerRuntime::Lxd => format!(
            "if command -v {cli} >/dev/null 2>&1; then {cli} list --format csv -c nst 2>&1; else echo RUNTIME_NOT_INSTALLED; fi"
        ),
    }
}

fn unavailable(runtime: ContainerRuntime, text: &str) -> DockerContainersSnapshot {
    let low = text.to_ascii_lowercase();
    let label = runtime.label();
    let (why, msg) = if low.contains("current-context is not set")
        || low.contains("no configuration has been provided")
        || low.contains("invalid configuration")
    {
        (
            DockerUnavailable::NoCluster,
            "kubectl has no cluster selected on this host".to_string(),
        )
    } else if low.contains("permission denied")
        || low.contains("forbidden")
        || low.contains("needed permissions")
    {
        (
            DockerUnavailable::NoPermission,
            format!("No permission to use {label}"),
        )
    } else if low.contains("cannot connect")
        || low.contains("is the docker daemon running")
        || low.contains("podman.sock")
        || low.contains("connection refused")
        || low.contains("unable to connect")
        || low.contains("was refused")
        || low.contains("no route to host")
        || low.contains("i/o timeout")
        || low.contains("socket")
    {
        (
            DockerUnavailable::DaemonDown,
            if runtime == ContainerRuntime::Kubernetes {
                "The cluster is unreachable from the host".to_string()
            } else {
                format!("{label} is not running")
            },
        )
    } else {
        let first = text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        (
            DockerUnavailable::Unexpected,
            format!("{label} did not answer as expected: {}", clean(first, 120)),
        )
    };
    DockerContainersSnapshot {
        docker_available: false,
        error_message: Some(msg),
        unavailable: Some(why),
        containers: vec![],
        total: 0,
        unreadable: 0,
    }
}

fn not_installed(runtime: ContainerRuntime) -> DockerContainersSnapshot {
    DockerContainersSnapshot {
        docker_available: false,
        error_message: Some(format!("{} is not installed on this host", runtime.label())),
        unavailable: Some(DockerUnavailable::NotInstalled),
        containers: vec![],
        total: 0,
        unreadable: 0,
    }
}

fn state_from_status(status: &str) -> ContainerState {
    let s = status.trim();
    let low = s.to_ascii_lowercase();
    if low.contains("(paused)") || low.starts_with("paused") {
        ContainerState::Paused
    } else if low.starts_with("up") {
        ContainerState::Running
    } else if low.starts_with("exited") || low.starts_with("created") {
        ContainerState::Exited
    } else if low.starts_with("restarting") {
        ContainerState::Restarting
    } else {
        ContainerState::Unknown
    }
}

fn name_like(s: &str) -> Option<String> {
    let ok = !s.is_empty()
        && s.len() <= 64
        && s.starts_with(|c: char| c.is_ascii_alphanumeric())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
    ok.then(|| s.to_string())
}

/// A label field: a JSON string. Anything else is no label, not a reason to drop the container.
fn label_value(raw: &str, key: &str) -> Option<String> {
    let json = raw.strip_prefix(key)?.strip_prefix('=')?;
    let text: String = serde_json::from_str(json).ok()?;
    name_like(&text)
}

/// One line of `family_template` (9 fields) or `family_template_plain` (7), or `None` when it is
/// not exactly that shape.
fn parse_family_line(line: &str) -> Option<DockerContainerInfo> {
    const KEYS: [&str; 7] = ["ID", "NAME", "IMAGE", "STATE", "STATUS", "PORTS", "CREATED"];
    let parts: Vec<&str> = line.split(US).collect();
    if parts.len() != 7 && parts.len() != 9 {
        return None;
    }
    let mut vals = [""; 7];
    for (i, (part, key)) in parts.iter().zip(KEYS).enumerate() {
        vals[i] = part.strip_prefix(key)?.strip_prefix('=')?;
    }
    let id = validate_container_identifier(vals[0].to_string()).ok()?;
    let status = vals[4];
    let mut state = ContainerState::from_state_field(vals[3]);
    if state == ContainerState::Unknown {
        state = state_from_status(status);
    }
    let (project, service) = if parts.len() == 9 {
        (
            label_value(parts[7], "PROJECT"),
            label_value(parts[8], "SERVICE"),
        )
    } else {
        (None, None)
    };
    Some(DockerContainerInfo {
        id,
        name: clean(vals[1].split(',').next().unwrap_or(""), 128),
        image: clean(vals[2], 200),
        state,
        status_text: clean(status, 120),
        created_at: clean(vals[6], 60),
        ports: clean(vals[5], 200),
        unhealthy: status.contains("(unhealthy)"),
        project,
        service,
        scope: String::new(),
        restartable: true,
    })
}

fn parse_family_list(runtime: ContainerRuntime, output: &str) -> DockerContainersSnapshot {
    let mut containers: Vec<DockerContainerInfo> = Vec::new();
    let mut other = Vec::new();
    for line in output.lines().filter(|l| !l.trim().is_empty()) {
        if line.trim() == "RUNTIME_NOT_INSTALLED" {
            return not_installed(runtime);
        }
        match parse_family_line(line) {
            // The first entry of an id wins; a repeat is not shown as a second container.
            Some(info) if containers.iter().any(|c| c.id == info.id) => other.push(line.trim()),
            Some(info) => containers.push(info),
            None => other.push(line.trim()),
        }
    }
    if containers.is_empty() && !other.is_empty() {
        return unavailable(runtime, &other.join(" "));
    }
    let mut snap = finish(containers);
    snap.unreadable = other.len() as u32;
    snap
}

fn finish(mut containers: Vec<DockerContainerInfo>) -> DockerContainersSnapshot {
    let total = containers.len() as u32;
    containers.truncate(MAX_CONTAINERS);
    DockerContainersSnapshot {
        docker_available: true,
        error_message: None,
        unavailable: None,
        containers,
        total,
        unreadable: 0,
    }
}

fn parse_incus_list(runtime: ContainerRuntime, output: &str) -> DockerContainersSnapshot {
    let mut containers = Vec::new();
    let mut other = Vec::new();
    for line in output.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if line == "RUNTIME_NOT_INSTALLED" {
            return not_installed(runtime);
        }
        let f: Vec<&str> = line.split(',').collect();
        let named = f
            .first()
            .and_then(|n| validate_container_identifier(n.to_string()).ok());
        match (f.len(), named) {
            (3, Some(name)) => {
                let state = match f[1].trim().to_ascii_uppercase().as_str() {
                    "RUNNING" => ContainerState::Running,
                    "STOPPED" => ContainerState::Exited,
                    "FROZEN" => ContainerState::Paused,
                    _ => ContainerState::Unknown,
                };
                let vm = f[2].trim().eq_ignore_ascii_case("VIRTUAL-MACHINE");
                containers.push(DockerContainerInfo {
                    id: name.clone(),
                    name,
                    image: if vm { "virtual machine" } else { "container" }.to_string(),
                    state,
                    status_text: clean(f[1], 40),
                    created_at: String::new(),
                    ports: String::new(),
                    unhealthy: false,
                    project: None,
                    service: None,
                    scope: String::new(),
                    restartable: true,
                });
            }
            _ => other.push(line),
        }
    }
    if containers.is_empty() && !other.is_empty() {
        return unavailable(runtime, &other.join(" "));
    }
    let mut snap = finish(containers);
    snap.unreadable = other.len() as u32;
    snap
}

const RECREATING_OWNERS: [&str; 4] = [
    "ReplicaSet",
    "StatefulSet",
    "DaemonSet",
    "ReplicationController",
];

fn parse_pod(v: &Value) -> Option<DockerContainerInfo> {
    let meta = v.get("metadata")?;
    let ns = str_of(meta, "namespace");
    let name = str_of(meta, "name");
    let id = format!("{ns}/{name}");
    validate_pod_ref(&id).ok()?;
    let status = v.get("status").unwrap_or(&Value::Null);
    let spec_containers = v
        .get("spec")
        .and_then(|s| s.get("containers"))
        .and_then(Value::as_array);
    let mut image = spec_containers
        .and_then(|c| c.first())
        .map(|c| str_of(c, "image"))
        .unwrap_or_default();
    if let Some(n) = spec_containers.map(Vec::len).filter(|n| *n > 1) {
        image.push_str(&format!(" +{}", n - 1));
    }
    let phase = str_of(status, "phase");
    let statuses: &[Value] = status
        .get("containerStatuses")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    let restarts = statuses
        .iter()
        .filter_map(|c| c.get("restartCount").and_then(Value::as_u64))
        .max()
        .unwrap_or(0);
    let waiting = statuses.iter().find_map(|c| {
        c.get("state")
            .and_then(|s| s.get("waiting"))
            .map(|w| str_of(w, "reason"))
            .filter(|r| !r.is_empty())
    });
    let not_ready = statuses
        .iter()
        .any(|c| c.get("ready").and_then(Value::as_bool) == Some(false));
    let crashing = matches!(waiting.as_deref(), Some("CrashLoopBackOff"));
    let state = match phase.as_str() {
        "Running" if crashing => ContainerState::Restarting,
        "Running" => ContainerState::Running,
        "Pending" => ContainerState::Restarting,
        "Succeeded" => ContainerState::Exited,
        "Failed" => ContainerState::Dead,
        _ => ContainerState::Unknown,
    };
    let mut text = phase.clone();
    if restarts > 0 {
        text.push_str(&format!(", {restarts} restarts"));
    }
    if let Some(w) = &waiting {
        text.push_str(&format!(", {w}"));
    }
    let owned = meta
        .get("ownerReferences")
        .and_then(Value::as_array)
        .is_some_and(|o| {
            o.iter()
                .any(|r| RECREATING_OWNERS.contains(&str_of(r, "kind").as_str()))
        });
    Some(DockerContainerInfo {
        id,
        name: clean(&name, 128),
        image: clean(&image, 200),
        state,
        status_text: clean(&text, 120),
        created_at: clean(&str_of(meta, "creationTimestamp"), 60),
        ports: String::new(),
        unhealthy: (phase == "Running" && not_ready) || crashing,
        project: None,
        service: None,
        scope: ns,
        restartable: owned,
    })
}

fn parse_kube_list(output: &str) -> DockerContainersSnapshot {
    let runtime = ContainerRuntime::Kubernetes;
    if output.lines().any(|l| l.trim() == "RUNTIME_NOT_INSTALLED") {
        return not_installed(runtime);
    }
    if output.lines().any(|l| l.trim() == "KUBE_NO_CONTEXT") {
        return unavailable(runtime, "current-context is not set");
    }
    let body: String = output
        .lines()
        .filter(|l| l.trim() != "KUBE_CURRENT_NAMESPACE")
        .collect::<Vec<_>>()
        .join("\n");
    let current_only = output.lines().any(|l| l.trim() == "KUBE_CURRENT_NAMESPACE");
    let Some(start) = body.find('{') else {
        return unavailable(runtime, &body);
    };
    let doc: Value = match serde_json::from_str(body[start..].trim()) {
        Ok(v) => v,
        Err(_) if body.len() >= KUBECTL_MAX_BYTES => {
            return DockerContainersSnapshot {
                docker_available: false,
                error_message: Some("Too many pods to list from the phone".to_string()),
                unavailable: Some(DockerUnavailable::Unexpected),
                containers: vec![],
                total: 0,
                unreadable: 0,
            }
        }
        Err(_) => return unavailable(runtime, &body),
    };
    let Some(items) = doc.get("items").and_then(Value::as_array) else {
        return unavailable(runtime, &body);
    };
    let mut pods: Vec<DockerContainerInfo> = items.iter().filter_map(parse_pod).collect();
    let unreadable = (items.len() - pods.len()) as u32;
    // Trouble first, then by namespace and name: the cap should never hide a failing pod.
    pods.sort_by(|a, b| {
        let rank = |p: &DockerContainerInfo| match (p.unhealthy, p.state) {
            (true, _) | (_, ContainerState::Dead) => 0,
            (_, ContainerState::Restarting) => 1,
            _ => 2,
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.scope.cmp(&b.scope))
            .then_with(|| a.name.cmp(&b.name))
    });
    let mut snap = finish(pods);
    snap.unreadable = unreadable;
    if current_only {
        snap.error_message = Some("Showing the host's current namespace only".to_string());
    }
    snap
}

/// Reads the output of `container_list_command`. Trouble is a snapshot with a reason, not an
/// error.
#[uniffi::export]
pub fn parse_container_list(runtime: ContainerRuntime, output: String) -> DockerContainersSnapshot {
    match runtime {
        ContainerRuntime::Docker => {
            // The marker counts only as a line of its own: a label or a name that contains the
            // text must not cut the list in two.
            let (mut json_part, mut projects, mut after) = (String::new(), String::new(), false);
            for line in output.lines() {
                if !after && line == PROJECTS_MARKER {
                    after = true;
                    continue;
                }
                let dst = if after { &mut projects } else { &mut json_part };
                dst.push_str(line);
                dst.push('\n');
            }
            let mut snap = parse_docker_list(json_part);
            if snap.docker_available {
                for line in projects.lines() {
                    if let Some(f) = parse_family_line(line) {
                        if let Some(c) = snap
                            .containers
                            .iter_mut()
                            .find(|c| c.id == f.id || f.id.starts_with(&c.id))
                        {
                            c.project = f.project;
                            c.service = f.service;
                        }
                    }
                }
            }
            snap
        }
        ContainerRuntime::Podman | ContainerRuntime::Nerdctl => parse_family_list(runtime, &output),
        ContainerRuntime::Kubernetes => parse_kube_list(&output),
        ContainerRuntime::Incus | ContainerRuntime::Lxd => parse_incus_list(runtime, &output),
    }
}

// ---------- stats ----------

/// Memory and CPU: engines only.
fn container_stats_command(runtime: ContainerRuntime, id: &str) -> Result<String, SshCoreError> {
    if !runtime.is_engine() {
        return Err(unsupported(runtime, "usage numbers"));
    }
    let Target::Plain(id) = target(runtime, id)? else {
        return Err(invalid("Invalid container name or id"));
    };
    let cli = runtime.cli();
    let format = if runtime == ContainerRuntime::Docker {
        "'{{json .}}'"
    } else {
        r#"'{{.MemUsage}}{{"\x1f"}}{{.CPUPerc}}'"#
    };
    Ok(docker_script(
        &format!(
            "{cli} stats --no-stream --format {format} {} 2>&1",
            shell_quote(&id)
        ),
        10,
    ))
}

#[uniffi::export]
pub fn runtime_stats_command(
    runtime: ContainerRuntime,
    container_id: String,
) -> Result<String, SshCoreError> {
    container_stats_command(runtime, &container_id)
}

#[uniffi::export]
pub fn parse_runtime_stats(runtime: ContainerRuntime, output: String) -> DockerContainerStats {
    if runtime == ContainerRuntime::Docker {
        return crate::docker::parse_docker_stats(output);
    }
    let line = output
        .lines()
        .find(|l| l.contains(US))
        .unwrap_or("")
        .to_string();
    let mut f = line.split(US);
    let mem = f.next().unwrap_or("");
    let mut parts = mem.split('/');
    let usage = parts.next().and_then(parse_size);
    let limit = parts.next().and_then(parse_size);
    let cpu = f
        .next()
        .unwrap_or("")
        .trim()
        .trim_end_matches('%')
        .parse::<f32>()
        .ok()
        .filter(|c| c.is_finite() && *c >= 0.0);
    DockerContainerStats {
        memory_usage_bytes: usage,
        memory_limit_bytes: limit,
        cpu_percentage: cpu,
    }
}

// ---------- actions ----------

fn verb(action: DockerAction) -> &'static str {
    match action {
        DockerAction::Start => "start",
        DockerAction::Stop => "stop",
        DockerAction::Restart => "restart",
    }
}

#[uniffi::export]
pub fn runtime_action_command(
    runtime: ContainerRuntime,
    action: DockerAction,
    container_id: String,
) -> Result<String, SshCoreError> {
    Ok(docker_script(
        &action_body(runtime, action, &container_id)?,
        40,
    ))
}

fn action_body(
    runtime: ContainerRuntime,
    action: DockerAction,
    container_id: &str,
) -> Result<String, SshCoreError> {
    let t = target(runtime, container_id)?;
    let cli = runtime.cli();
    match (runtime, t) {
        (ContainerRuntime::Kubernetes, Target::Pod { ns, name }) => {
            if action != DockerAction::Restart {
                return Err(unsupported(runtime, "start or stop for a pod"));
            }
            let (ns, name) = (shell_quote(&ns), shell_quote(&name));
            let owners = RECREATING_OWNERS.join("|");
            Ok(
                format!(
                    "kinds=$(kubectl get pod -n {ns} {name} -o jsonpath='{{.metadata.ownerReferences[*].kind}}' 2>&1) || {{ echo \"$kinds\"; exit 1; }}; \
                     ok=0; for k in $kinds; do case \"$k\" in {owners}) ok=1;; esac; done; \
                     [ \"$ok\" = 1 ] || {{ echo NO_CONTROLLER; exit 1; }}; \
                     kubectl delete pod -n {ns} {name} --wait=false 2>&1 && echo {ACTION_OK}"
                ),
            )
        }
        (_, Target::Plain(id)) => Ok(format!(
            "{cli} {} {} 2>&1 && echo {ACTION_OK}",
            verb(action),
            shell_quote(&id)
        )),
        _ => Err(invalid("Invalid container name or id")),
    }
}

/// `Ok` when the host confirmed, else the cleaned first line of what the tool said.
#[uniffi::export]
pub fn parse_runtime_action(runtime: ContainerRuntime, output: String) -> Result<(), SshCoreError> {
    if output
        .lines()
        .any(|l| l.trim() == ACTION_OK || l.trim() == "DOCKER_OK")
    {
        return Ok(());
    }
    if output.lines().any(|l| l.trim() == "NO_CONTROLLER") {
        return Err(invalid(
            "This pod has no controller that would recreate it; deleting it would lose it",
        ));
    }
    let first = output
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("no answer from the host");
    Err(SshCoreError::IoError {
        reason: format!("{}: {}", runtime.label(), clean(first, 200)),
    })
}

// ---------- stacks ----------

/// What a stack action did, read from the script's output.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StackOutcome {
    pub done: Vec<String>,
    pub failed_id: Option<String>,
    pub failed_reason: Option<String>,
    pub not_started: Vec<String>,
    /// The output ended without a verdict (the call was cut): the list is the truth.
    pub unknown: bool,
}

/// One script on the host for a whole Compose project, so it finishes even if the app goes to
/// the background. Engines only, at most `MAX_STACK` containers, stops at the first failure.
#[uniffi::export]
pub fn runtime_stack_command(
    runtime: ContainerRuntime,
    action: DockerAction,
    container_ids: Vec<String>,
) -> Result<String, SshCoreError> {
    Ok(docker_script(
        &stack_body(runtime, action, container_ids)?,
        STACK_SECS,
    ))
}

fn stack_body(
    runtime: ContainerRuntime,
    action: DockerAction,
    container_ids: Vec<String>,
) -> Result<String, SshCoreError> {
    if !runtime.is_engine() {
        return Err(unsupported(runtime, "stacks"));
    }
    if container_ids.is_empty() || container_ids.len() > MAX_STACK {
        return Err(invalid(&format!(
            "A stack action covers 1 to {MAX_STACK} containers"
        )));
    }
    let mut quoted = Vec::new();
    for id in container_ids {
        quoted.push(shell_quote(&validate_container_identifier(id)?));
    }
    let cli = runtime.cli();
    let v = verb(action);
    Ok(format!(
        "for id in {}; do out=$({cli} {v} \"$id\" 2>&1) && echo \"DONE:$id\" || {{ echo \"FAILED:$id $(printf '%s' \"$out\" | head -n 1)\"; exit 1; }}; done",
        quoted.join(" ")
    ))
}

#[uniffi::export]
pub fn parse_stack_outcome(output: String, container_ids: Vec<String>) -> StackOutcome {
    let mut done = Vec::new();
    let mut failed: Option<(String, String)> = None;
    for line in output.lines().map(str::trim) {
        if let Some(id) = line.strip_prefix("DONE:") {
            if container_ids.iter().any(|c| c == id) {
                done.push(id.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("FAILED:") {
            let (id, reason) = rest.split_once(' ').unwrap_or((rest, ""));
            if container_ids.iter().any(|c| c == id) && failed.is_none() {
                failed = Some((id.to_string(), clean(reason, 200)));
            }
        }
    }
    let seen = done.len() + usize::from(failed.is_some());
    let not_started: Vec<String> = container_ids
        .iter()
        .filter(|c| !done.contains(c) && failed.as_ref().map(|f| &f.0) != Some(c))
        .cloned()
        .collect();
    let unknown = failed.is_none() && seen < container_ids.len();
    StackOutcome {
        done,
        failed_id: failed.as_ref().map(|f| f.0.clone()),
        failed_reason: failed.map(|f| f.1),
        not_started,
        unknown,
    }
}

// ---------- logs ----------

#[uniffi::export]
pub fn runtime_logs_command(
    runtime: ContainerRuntime,
    container_id: String,
    tail_lines: u32,
) -> Result<String, SshCoreError> {
    Ok(docker_script(
        &logs_body(runtime, &container_id, tail_lines)?,
        15,
    ))
}

fn logs_body(
    runtime: ContainerRuntime,
    container_id: &str,
    tail_lines: u32,
) -> Result<String, SshCoreError> {
    let t = target(runtime, container_id)?;
    let tail = if tail_lines == 0 {
        LOG_DEFAULT_TAIL
    } else {
        tail_lines.min(LOG_MAX_TAIL)
    };
    let cap = LOG_MAX_BYTES + 1;
    let cli = runtime.cli();
    Ok(match t {
        Target::Pod { ns, name } => format!(
            "kubectl logs --tail={tail} --timestamps -n {} {} --all-containers=true 2>&1 | tail -c {cap}",
            shell_quote(&ns),
            shell_quote(&name)
        ),
        Target::Plain(id) if runtime.is_incus_family() => format!(
            "{cli} console {} --show-log 2>&1 | tail -n {tail} | tail -c {cap}",
            shell_quote(&id)
        ),
        Target::Plain(id) => format!(
            "{cli} logs --tail {tail} --timestamps {} 2>&1 | tail -c {cap}",
            shell_quote(&id)
        ),
    })
}

// ---------- shell ----------

const SHELL_PICK: &str = "sh -c 'command -v bash >/dev/null 2>&1 && exec bash || exec sh'";

/// The shell in a container, pod or instance; the pane stays open with the exit code until Enter.
/// No `--privileged`, no `--user`.
#[uniffi::export]
pub fn build_runtime_exec_command(
    runtime: ContainerRuntime,
    container_id: String,
) -> Result<String, SshCoreError> {
    let t = target(runtime, &container_id)?;
    let cli = runtime.cli();
    let run = match t {
        Target::Pod { ns, name } => format!(
            "kubectl exec -it -n {} {} -- {SHELL_PICK}",
            shell_quote(&ns),
            shell_quote(&name)
        ),
        Target::Plain(id) if runtime.is_incus_family() => {
            format!("{cli} exec {} -- {SHELL_PICK}", shell_quote(&id))
        }
        Target::Plain(id) => format!("{cli} exec -it {} {SHELL_PICK}", shell_quote(&id)),
    };
    Ok(format!(
        "{DOCKER_PATH}; {run}; \
         c=$?; printf '\\r\\n\\033[33m[Container session ended (exit code %s)]\\033[0m\\r\\nPress Enter to close.\\r\\n' \"$c\"; read -r _"
    ))
}

/// `<runtime>-<name>-<6 characters>` for the tmux session that keeps the shell alive. The suffix
/// is the start of the id for engines and a hash of the reference for pods and instances, which
/// keeps `a.b` and `a-b` apart after the dots are turned into dashes.
#[uniffi::export]
pub fn runtime_session_name(
    runtime: ContainerRuntime,
    container_name: String,
    container_id: String,
) -> Result<String, SshCoreError> {
    target(runtime, &container_id)?;
    let suffix: String = if runtime.is_engine() {
        container_id.chars().take(6).collect()
    } else {
        Sha256::digest(container_id.as_bytes())
            .iter()
            .take(3)
            .map(|b| format!("{b:02x}"))
            .collect()
    };
    let slug: String = container_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-');
    let prefix = runtime.session_prefix();
    let keep = 64usize.saturating_sub(prefix.len() + 2 + suffix.len());
    let slug: String = slug.chars().take(keep).collect();
    let name = if slug.is_empty() {
        format!("{prefix}-{suffix}")
    } else {
        format!("{prefix}-{slug}-{suffix}")
    };
    crate::tmux::validate_session_name(&name)?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: [ContainerRuntime; 6] = [
        ContainerRuntime::Docker,
        ContainerRuntime::Podman,
        ContainerRuntime::Nerdctl,
        ContainerRuntime::Kubernetes,
        ContainerRuntime::Incus,
        ContainerRuntime::Lxd,
    ];

    fn fam(id: &str, name: &str, state: &str, status: &str, ports: &str, proj: &str) -> String {
        let f = [
            format!("ID={id}"),
            format!("NAME={name}"),
            "IMAGE=nginx".into(),
            format!("STATE={state}"),
            format!("STATUS={status}"),
            format!("PORTS={ports}"),
            "CREATED=2026-10-20".into(),
            format!("PROJECT={}", serde_json::to_string(proj).unwrap()),
            "SERVICE=\"web\"".into(),
        ];
        f.join(&US.to_string())
    }

    #[test]
    fn probe_lists_what_the_host_has_in_order() {
        let out = "noise\nRUNTIME:kubectl\nRUNTIME:docker\nRUNTIME:lxc\nRUNTIME:bogus\n";
        assert_eq!(
            parse_runtime_probe(out.into()),
            [
                ContainerRuntime::Docker,
                ContainerRuntime::Kubernetes,
                ContainerRuntime::Lxd
            ]
        );
        let with_incus = "RUNTIME:incus\nRUNTIME:lxc\n";
        assert_eq!(
            parse_runtime_probe(with_incus.into()),
            [ContainerRuntime::Incus]
        );
        assert!(parse_runtime_probe(String::new()).is_empty());
        assert!(runtime_probe_command().contains("RUNTIME:"));
    }

    #[test]
    fn capabilities_follow_the_table() {
        let k = runtime_caps(ContainerRuntime::Kubernetes);
        assert!(!k.stats && !k.start_stop && k.restart && k.groups);
        let i = runtime_caps(ContainerRuntime::Incus);
        assert!(!i.stats && i.start_stop && !i.groups);
        let p = runtime_caps(ContainerRuntime::Podman);
        assert!(p.stats && p.start_stop && p.groups);
    }

    #[test]
    fn a_podman_list_parses_and_commas_stay_in_their_field() {
        let out = format!(
            "{}\n{}\n",
            fam(
                "3f2a9c1d",
                "web",
                "running",
                "Up 2 hours",
                "0.0.0.0:80->80/tcp,:::80->80/tcp",
                "shop"
            ),
            fam("aa11bb22", "db", "", "Exited (1) 3 minutes ago", "", "")
        );
        let s = parse_container_list(ContainerRuntime::Podman, out);
        assert!(s.docker_available);
        assert_eq!(s.containers.len(), 2);
        let web = &s.containers[0];
        assert_eq!(web.state, ContainerState::Running);
        assert_eq!(web.ports, "0.0.0.0:80->80/tcp,:::80->80/tcp");
        assert_eq!(web.project.as_deref(), Some("shop"));
        assert_eq!(web.service.as_deref(), Some("web"));
        let db = &s.containers[1];
        assert_eq!(
            db.state,
            ContainerState::Exited,
            "state from the Status text"
        );
        assert_eq!(db.project, None);
    }

    #[test]
    fn a_line_of_the_wrong_shape_is_dropped_not_guessed() {
        let good = fam("3f2a9c1d", "web", "running", "Up", "", "");
        let short = good
            .split(US)
            .take(5)
            .collect::<Vec<_>>()
            .join(&US.to_string());
        let wrong_key = good.replacen("NAME=", "NOPE=", 1);
        let bad_id = fam("x; rm -rf /", "web", "running", "Up", "", "");
        for bad in [short, wrong_key, bad_id] {
            let s = parse_container_list(ContainerRuntime::Podman, format!("{good}\n{bad}\n"));
            assert_eq!(s.containers.len(), 1, "{bad:?}");
        }
    }

    #[test]
    fn family_failures_are_classified_per_reason() {
        let cases = [
            ("RUNTIME_NOT_INSTALLED", DockerUnavailable::NotInstalled),
            (
                "Cannot connect to Podman. Please verify your connection to the Linux system using `podman system connection list`",
                DockerUnavailable::DaemonDown,
            ),
            (
                "permission denied while trying to connect",
                DockerUnavailable::NoPermission,
            ),
            ("template parsing error: boom", DockerUnavailable::Unexpected),
        ];
        for (text, why) in cases {
            let s = parse_container_list(ContainerRuntime::Podman, text.into());
            assert_eq!(s.unavailable, Some(why), "{text}");
            assert!(!s.docker_available);
        }
        let s = parse_container_list(
            ContainerRuntime::Nerdctl,
            "template parsing error: boom".into(),
        );
        assert!(s.error_message.unwrap().contains("boom"));
    }

    #[test]
    fn docker_keeps_its_json_and_gains_projects() {
        let json = r#"{"ID":"bc262e486183","Names":"shop-web-1","Image":"nginx","State":"running","Status":"Up 3 hours","CreatedAt":"x","Ports":""}"#;
        let proj = fam("bc262e486183", "shop-web-1", "running", "Up", "", "shop");
        let out = format!("{json}\n{PROJECTS_MARKER}\n{proj}\n");
        let s = parse_container_list(ContainerRuntime::Docker, out);
        assert_eq!(s.containers.len(), 1);
        assert_eq!(s.containers[0].project.as_deref(), Some("shop"));
        // The old output (no marker) is read as before.
        let old = parse_container_list(ContainerRuntime::Docker, json.into());
        assert_eq!(old.containers[0].project, None);
        let none = parse_container_list(ContainerRuntime::Docker, "DOCKER_NOT_INSTALLED".into());
        assert_eq!(none.unavailable, Some(DockerUnavailable::NotInstalled));
    }

    #[test]
    fn incus_csv_maps_states_and_marks_virtual_machines() {
        let out = "web,RUNNING,CONTAINER\ndb,STOPPED,VIRTUAL-MACHINE\nold,FROZEN,CONTAINER\nodd,WEIRD,CONTAINER\nbroken,line\n";
        let s = parse_container_list(ContainerRuntime::Incus, out.into());
        let states: Vec<_> = s.containers.iter().map(|c| c.state).collect();
        assert_eq!(
            states,
            [
                ContainerState::Running,
                ContainerState::Exited,
                ContainerState::Paused,
                ContainerState::Unknown
            ]
        );
        assert_eq!(s.containers[1].image, "virtual machine");
        let down = parse_container_list(
            ContainerRuntime::Lxd,
            "Error: Get \"http://unix.socket/1.0\": dial unix /var/lib/lxd/unix.socket: connect: permission denied".into(),
        );
        assert_eq!(down.unavailable, Some(DockerUnavailable::NoPermission));
    }

    fn pod(ns: &str, name: &str, phase: &str, owner: Option<&str>, extra: &str) -> String {
        let owners = owner
            .map(|k| format!(r#","ownerReferences":[{{"kind":"{k}"}}]"#))
            .unwrap_or_default();
        format!(
            r#"{{"metadata":{{"namespace":"{ns}","name":"{name}","creationTimestamp":"2026-10-20T10:00:00Z"{owners}}},"spec":{{"containers":[{{"image":"app:1"}},{{"image":"side:2"}}]}},"status":{{"phase":"{phase}"{extra}}}}}"#
        )
    }

    #[test]
    fn kubernetes_pods_parse_with_problems_first() {
        let crash = r#","containerStatuses":[{"ready":false,"restartCount":5,"state":{"waiting":{"reason":"CrashLoopBackOff"}}}]"#;
        let ok = r#","containerStatuses":[{"ready":true,"restartCount":0,"state":{"running":{}}}]"#;
        let doc = format!(
            r#"{{"items":[{},{},{},{},{}]}}"#,
            pod("app", "zzz-ok", "Running", Some("ReplicaSet"), ok),
            pod("app", "api-7d9", "Running", Some("ReplicaSet"), crash),
            pod("app", "bare", "Running", None, ok),
            pod("job", "once", "Succeeded", Some("Job"), ""),
            pod("app", "mirror", "Running", Some("Node"), ok),
        );
        let s = parse_container_list(ContainerRuntime::Kubernetes, doc);
        assert_eq!(s.total, 5);
        assert_eq!(
            s.containers[0].id, "app/api-7d9",
            "the crashing pod is first"
        );
        let api = &s.containers[0];
        assert!(api.unhealthy && api.restartable);
        assert_eq!(api.state, ContainerState::Restarting);
        assert!(
            api.status_text.contains("5 restarts") && api.status_text.contains("CrashLoopBackOff")
        );
        assert_eq!(api.image, "app:1 +1");
        assert_eq!(api.scope, "app");
        let by = |n: &str| s.containers.iter().find(|c| c.name == n).unwrap();
        assert!(!by("bare").restartable, "no owner");
        assert!(!by("mirror").restartable, "owned by a node");
        assert!(!by("once").restartable, "a Job pod is not recreated");
        assert_eq!(by("once").state, ContainerState::Exited);
    }

    #[test]
    fn kubernetes_trouble_is_classified() {
        let k = ContainerRuntime::Kubernetes;
        let cases = [
            ("error: current-context is not set", DockerUnavailable::NoCluster),
            (
                "The connection to the server localhost:8080 was refused - did you specify the right host or port?",
                DockerUnavailable::DaemonDown,
            ),
            (
                "Error from server (Forbidden): pods is forbidden: User cannot list",
                DockerUnavailable::NoPermission,
            ),
            ("RUNTIME_NOT_INSTALLED", DockerUnavailable::NotInstalled),
        ];
        for (text, why) in cases {
            let s = parse_container_list(k, text.into());
            assert_eq!(s.unavailable, Some(why), "{text}");
        }
        let s = parse_container_list(k, "{\"items\": [".into());
        assert_eq!(s.unavailable, Some(DockerUnavailable::Unexpected));
        let cut = format!("{{\"items\": [{}", "x".repeat(KUBECTL_MAX_BYTES));
        let s = parse_container_list(k, cut);
        assert!(s.error_message.unwrap().contains("Too many pods"));
        let current = format!(
            "KUBE_CURRENT_NAMESPACE\n{{\"items\":[{}]}}",
            pod("a", "p", "Running", None, "")
        );
        let s = parse_container_list(k, current);
        assert!(s.docker_available);
        assert!(s.error_message.unwrap().contains("current namespace"));
    }

    #[test]
    fn pod_references_are_strict() {
        for ok in [
            "app/api-7d9",
            "kube-system/coredns-5d78c9869d-abcde",
            "a/b.c",
        ] {
            assert!(validate_pod_ref(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "app",
            "/x",
            "app/",
            "App/x",
            "app/X",
            "app/-x",
            "app/x-",
            "-n/x",
            "a b/x",
            "app/x;y",
            "app/x$(y)",
            "a/b/c",
            "app/..",
            "app/a..b",
            "app/a\nb",
            "a_b/x",
        ] {
            assert!(validate_pod_ref(bad).is_err(), "{bad:?}");
        }
        assert!(validate_pod_ref(&format!("a/{}", "x".repeat(254))).is_err());
    }

    #[test]
    fn hostile_identifiers_never_reach_a_command() {
        let bad = [
            "x; rm -rf /",
            "$(id)",
            "-rf",
            "a b",
            "a\nb",
            "a`b`",
            "../x",
            "",
        ];
        for rt in R {
            for id in bad {
                assert!(
                    runtime_logs_command(rt, id.into(), 10).is_err(),
                    "{rt:?} {id:?}"
                );
                assert!(build_runtime_exec_command(rt, id.into()).is_err());
                assert!(
                    runtime_action_command(rt, DockerAction::Restart, id.into()).is_err(),
                    "{rt:?} {id:?}"
                );
                assert!(runtime_session_name(rt, "n".into(), id.into()).is_err());
            }
        }
        assert!(runtime_stack_command(
            ContainerRuntime::Podman,
            DockerAction::Restart,
            vec!["ok1".into(), "a;b".into()]
        )
        .is_err());
    }

    #[test]
    fn commands_use_the_binary_of_the_runtime_and_quote_the_id() {
        let c = action_body(ContainerRuntime::Podman, DockerAction::Stop, "web-1").unwrap();
        assert!(c.contains("podman stop 'web-1'"), "{c}");
        let c = logs_body(ContainerRuntime::Nerdctl, "web-1", 0).unwrap();
        assert!(
            c.contains("nerdctl logs --tail 200 --timestamps 'web-1'"),
            "{c}"
        );
        let c = logs_body(ContainerRuntime::Incus, "web", 50).unwrap();
        assert!(c.contains("incus console 'web' --show-log"), "{c}");
        let c = logs_body(ContainerRuntime::Kubernetes, "app/api-7d9", 50).unwrap();
        assert!(
            c.contains(
                "kubectl logs --tail=50 --timestamps -n 'app' 'api-7d9' --all-containers=true"
            ),
            "{c}"
        );
        let c =
            build_runtime_exec_command(ContainerRuntime::Kubernetes, "app/api-7d9".into()).unwrap();
        assert!(
            c.contains("kubectl exec -it -n 'app' 'api-7d9' -- sh -c"),
            "{c}"
        );
        let c = build_runtime_exec_command(ContainerRuntime::Lxd, "web".into()).unwrap();
        assert!(c.contains("lxc exec 'web' -- sh -c"), "{c}");
        let c = build_runtime_exec_command(ContainerRuntime::Podman, "web-1".into()).unwrap();
        assert!(c.contains("podman exec -it 'web-1' sh -c"), "{c}");
        assert!(!c.contains("--privileged") && !c.contains("--user"));
        let l = list_body(ContainerRuntime::Podman);
        assert!(l.contains(r#"{{"\x1f"}}"#) && l.contains("podman ps -a"));
        assert!(list_body(ContainerRuntime::Docker).contains(PROJECTS_MARKER));
        assert!(list_body(ContainerRuntime::Kubernetes)
            .contains(&format!("head -c {KUBECTL_MAX_BYTES}")));
    }

    #[test]
    fn what_a_runtime_cannot_do_is_refused() {
        let k = ContainerRuntime::Kubernetes;
        assert!(runtime_action_command(k, DockerAction::Stop, "a/b".into()).is_err());
        assert!(runtime_action_command(k, DockerAction::Start, "a/b".into()).is_err());
        let restart = action_body(k, DockerAction::Restart, "app/api").unwrap();
        assert!(restart.contains("ReplicaSet|StatefulSet|DaemonSet|ReplicationController"));
        assert!(
            restart.contains("NO_CONTROLLER")
                && restart.contains("delete pod -n 'app' 'api' --wait=false")
        );
        assert!(runtime_stats_command(k, "a/b".into()).is_err());
        assert!(runtime_stats_command(ContainerRuntime::Incus, "web".into()).is_err());
        assert!(runtime_stack_command(k, DockerAction::Restart, vec!["a".into()]).is_err());
        assert!(runtime_stats_command(ContainerRuntime::Podman, "web-1".into()).is_ok());
    }

    #[test]
    fn action_answers_are_read() {
        let k = ContainerRuntime::Kubernetes;
        assert!(parse_runtime_action(k, "pod \"x\" deleted\nACTION_OK\n".into()).is_ok());
        assert!(parse_runtime_action(ContainerRuntime::Docker, "DOCKER_OK".into()).is_ok());
        let e = parse_runtime_action(k, "NO_CONTROLLER".into()).unwrap_err();
        assert!(matches!(e, SshCoreError::InvalidInput { .. }));
        let e = parse_runtime_action(
            ContainerRuntime::Podman,
            "Error: no such container\n".into(),
        )
        .unwrap_err();
        assert!(e.to_string().contains("Podman: Error: no such container"));
    }

    #[test]
    fn podman_stats_read_the_template_output() {
        let line = format!("142MiB / 3.9GiB{US}12.50%");
        let s = parse_runtime_stats(ContainerRuntime::Podman, line);
        assert_eq!(s.memory_usage_bytes, Some(142 * 1024 * 1024));
        assert!(s.memory_limit_bytes.is_some());
        assert_eq!(s.cpu_percentage, Some(12.5));
        let none = parse_runtime_stats(ContainerRuntime::Nerdctl, "garbage".into());
        assert_eq!(none.memory_usage_bytes, None);
    }

    #[test]
    fn a_stack_runs_as_one_script_and_stops_at_the_first_failure() {
        let ids: Vec<String> = ["aa11", "bb22", "cc33"].map(String::from).to_vec();
        let c = stack_body(ContainerRuntime::Podman, DockerAction::Restart, ids.clone()).unwrap();
        assert!(c.contains("for id in 'aa11' 'bb22' 'cc33'") && c.contains("podman restart"));
        assert!(
            runtime_stack_command(ContainerRuntime::Docker, DockerAction::Stop, vec![]).is_err()
        );
        let many: Vec<String> = (0..=MAX_STACK).map(|i| format!("c{i:02}x")).collect();
        assert!(runtime_stack_command(ContainerRuntime::Docker, DockerAction::Stop, many).is_err());

        let all = parse_stack_outcome("DONE:aa11\nDONE:bb22\nDONE:cc33\n".into(), ids.clone());
        assert_eq!(all.done.len(), 3);
        assert!(all.failed_id.is_none() && !all.unknown && all.not_started.is_empty());

        let part = parse_stack_outcome(
            "DONE:aa11\nFAILED:bb22 Error: no such container\n".into(),
            ids.clone(),
        );
        assert_eq!(part.done, ["aa11"]);
        assert_eq!(part.failed_id.as_deref(), Some("bb22"));
        assert!(part.failed_reason.unwrap().contains("no such container"));
        assert_eq!(part.not_started, ["cc33"]);
        assert!(!part.unknown);

        let cut = parse_stack_outcome("DONE:aa11\n".into(), ids.clone());
        assert!(cut.unknown, "no verdict for the rest");
        // Ids the script was not given are ignored.
        let forged = parse_stack_outcome("DONE:zz99\nFAILED:zz99 x\n".into(), ids);
        assert!(forged.done.is_empty() && forged.failed_id.is_none() && forged.unknown);
    }

    #[test]
    fn session_names_are_valid_and_keep_lookalikes_apart() {
        let a = runtime_session_name(ContainerRuntime::Podman, "a.b".into(), "3f2a9c1d77".into())
            .unwrap();
        let b = runtime_session_name(ContainerRuntime::Podman, "a-b".into(), "aa11bb22cc".into())
            .unwrap();
        assert!(a.starts_with("podman-a-b-") && a != b);
        let p1 = runtime_session_name(ContainerRuntime::Kubernetes, "api".into(), "app/api".into())
            .unwrap();
        let p2 = runtime_session_name(
            ContainerRuntime::Kubernetes,
            "api".into(),
            "other/api".into(),
        )
        .unwrap();
        assert!(
            p1.starts_with("k8s-api-") && p1 != p2,
            "same pod name in two namespaces"
        );
        let long =
            runtime_session_name(ContainerRuntime::Incus, "x".repeat(200), "web".into()).unwrap();
        assert!(long.len() <= 64);
        let empty =
            runtime_session_name(ContainerRuntime::Lxd, "...".into(), "web".into()).unwrap();
        assert!(empty.starts_with("lxd-"));
    }

    #[test]
    fn the_cap_keeps_the_total() {
        let lines: Vec<String> = (0..MAX_CONTAINERS + 30)
            .map(|i| fam(&format!("id{i:04}"), "c", "running", "Up", "", ""))
            .collect();
        let s = parse_container_list(ContainerRuntime::Podman, lines.join("\n"));
        assert_eq!(s.containers.len(), MAX_CONTAINERS);
        assert_eq!(s.total as usize, MAX_CONTAINERS + 30);
    }

    /// Read-only: runs the real list script of this module through `sh` against the kubectl of
    /// this machine. With no context selected it must say so (kubectl itself would answer with a
    /// refused connection to localhost:8080, which reads like a down cluster). Skipped when the
    /// machine has a context: this test never touches a real cluster.
    #[test]
    fn live_kubectl_without_a_context_reports_no_cluster() {
        let has_ctx = std::process::Command::new("kubectl")
            .args(["config", "current-context"])
            .output();
        let Ok(has_ctx) = has_ctx else {
            eprintln!("SKIP: no kubectl");
            return;
        };
        if has_ctx.status.success() || std::env::var_os("KUBERNETES_SERVICE_HOST").is_some() {
            eprintln!("SKIP: kubectl has a context; not touching a real cluster");
            return;
        }
        let out = std::process::Command::new("sh")
            .args(["-c", &list_body(ContainerRuntime::Kubernetes)])
            .env_remove("KUBERNETES_SERVICE_HOST")
            .output()
            .unwrap();
        let s = parse_container_list(
            ContainerRuntime::Kubernetes,
            String::from_utf8_lossy(&out.stdout).into_owned(),
        );
        assert_eq!(s.unavailable, Some(DockerUnavailable::NoCluster));
    }

    /// Read-only, against the Docker of this machine when its daemon answers: the real output
    /// of the list command must parse, projects included, and the family template must carry the
    /// separator (not its spelling).
    #[test]
    fn live_docker_list_with_projects() {
        let run = |args: &[&str]| {
            std::process::Command::new("docker")
                .args(args)
                .output()
                .ok()
        };
        let Some(info) =
            run(&["info", "--format", "{{.ServerVersion}}"]).filter(|o| o.status.success())
        else {
            eprintln!("SKIP: no docker daemon");
            return;
        };
        let _ = info;
        let json = run(&["ps", "-a", "--format", "{{json .}}"]).unwrap();
        let fam_out = run(&["ps", "-a", "--format", family_template()]).unwrap();
        let text = format!(
            "{}{PROJECTS_MARKER}\n{}",
            String::from_utf8_lossy(&json.stdout),
            String::from_utf8_lossy(&fam_out.stdout)
        );
        let s = parse_container_list(ContainerRuntime::Docker, text);
        let raw = String::from_utf8_lossy(&fam_out.stdout);
        assert!(
            !raw.contains("\\x1f"),
            "the separator must be the character, not its spelling"
        );
        assert!(s.docker_available, "{:?}", s.error_message);
        // Every container the family template can read is also read from the JSON.
        let family = parse_family_list(ContainerRuntime::Docker, &raw);
        assert_eq!(family.containers.len(), s.containers.len());
    }

    /// Property test of the builders: whatever text is thrown at them, an accepted identifier is
    /// plain characters, and the script that comes out runs exactly the commands meant (fake
    /// binaries record their arguments; a canary file must never appear).
    #[test]
    fn accepted_identifiers_are_plain_and_scripts_run_only_what_they_should() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("ssh-app-fakebin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("calls.log");
        let canary = dir.join("PWNED");
        for bin in ["docker", "podman", "nerdctl", "kubectl", "incus", "lxc"] {
            let path = dir.join(bin);
            std::fs::write(
                &path,
                format!(
                    "#!/bin/sh\necho \"{bin} $*\" >> '{}'\nexit 0\n",
                    log.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let run = |body: &str| {
            let _ = std::fs::remove_file(&log);
            std::process::Command::new("sh")
                .args(["-c", body])
                .env("PATH", format!("{}:/usr/bin:/bin", dir.display()))
                .env_remove("KUBERNETES_SERVICE_HOST")
                .output()
                .unwrap();
            std::fs::read_to_string(&log).unwrap_or_default()
        };
        let c = canary.display();
        let hostile = [
            format!("$(touch {c})"),
            format!("`touch {c}`"),
            format!("x;touch {c}"),
            format!("x\ntouch {c}"),
            format!("x'; touch {c}; '"),
            format!("x\"; touch {c}; \""),
            format!("x|touch {c}"),
            format!("x&&touch {c}"),
            "-rf".into(),
            "--privileged".into(),
            "a b".into(),
            "ns/pod; id".into(),
            "$IFS".into(),
            "a\u{1f}b".into(),
        ];
        let mut state = 0x2545F4914F6CDD1Du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let alphabet: Vec<char> = "ab9-_./ ;|&$`'\"\\\n\t(){}<>*?!#~%=:,@\u{1f}é"
            .chars()
            .collect();
        let mut corpus: Vec<String> = hostile.to_vec();
        for _ in 0..4000 {
            let len = 1 + (next() % 12) as usize;
            corpus.push(
                (0..len)
                    .map(|_| alphabet[(next() % alphabet.len() as u64) as usize])
                    .collect(),
            );
        }
        corpus.extend(["web-1", "app/api-7d9", "db.main"].map(String::from));
        let mut ran = 0;
        for id in &corpus {
            for rt in R {
                let accepted = [
                    action_body(rt, DockerAction::Restart, id).ok(),
                    logs_body(rt, id, 10).ok(),
                ];
                for body in accepted.into_iter().flatten() {
                    // Accepted means: nothing outside the validators' alphabet.
                    assert!(
                        id.chars()
                            .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c)),
                        "{rt:?} accepted {id:?}"
                    );
                    let calls = run(&body);
                    assert!(!canary.exists(), "{rt:?} ran injected code for {id:?}");
                    assert!(
                        calls.lines().count() >= 1
                            && calls.lines().all(|l| l.split(' ').next().is_some_and(|b| [
                                "docker", "podman", "nerdctl", "kubectl", "incus", "lxc"
                            ]
                            .contains(&b))),
                        "{rt:?} {id:?}: {calls}"
                    );
                    ran += 1;
                }
            }
        }
        assert!(ran >= 10, "the property test ran {ran} scripts");
        // Exact argv for one of each, as the host would see it.
        assert_eq!(
            run(&action_body(ContainerRuntime::Podman, DockerAction::Stop, "web-1").unwrap()),
            "podman stop web-1\n"
        );
        assert_eq!(
            run(&logs_body(ContainerRuntime::Kubernetes, "app/api", 5).unwrap()),
            "kubectl logs --tail=5 --timestamps -n app api --all-containers=true\n"
        );
        let stack = stack_body(
            ContainerRuntime::Docker,
            DockerAction::Restart,
            vec!["aa11".into(), "bb22".into()],
        )
        .unwrap();
        assert_eq!(run(&stack), "docker restart aa11\ndocker restart bb22\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn text_inside_a_field_cannot_cut_the_list_or_narrow_it() {
        // A container whose label carries the marker text does not split the output.
        let evil = r#"{"ID":"aa11bb22cc","Names":"evil","Image":"x","State":"running","Status":"Up","CreatedAt":"x","Ports":"","Labels":"k=---PROJECTS---"}"#;
        let good = r#"{"ID":"bc262e486183","Names":"shop-web-1","Image":"nginx","State":"running","Status":"Up","CreatedAt":"x","Ports":""}"#;
        let proj = fam("bc262e486183", "shop-web-1", "running", "Up", "", "shop");
        let out = format!("{evil}\n{good}\n{PROJECTS_MARKER}\n{proj}\n");
        let s = parse_container_list(ContainerRuntime::Docker, out);
        assert_eq!(s.containers.len(), 2, "both containers survive");
        assert_eq!(s.containers[1].project.as_deref(), Some("shop"));
        // A pod whose name or annotation says "Forbidden" is not an access error: the list
        // script only retries for the current namespace on a non-JSON answer.
        let body = list_body(ContainerRuntime::Kubernetes);
        assert!(body.contains("'{'*) ;; *Forbidden*"), "{body}");
    }

    fn fam3(id: &str, name: &str, proj: &str) -> String {
        fam(id, name, "running", "Up", "", proj)
    }

    #[test]
    fn a_hostile_label_cannot_hide_a_container_or_forge_another() {
        // The runtime escapes the label, so the separator and the newline arrive as \u001f and \n.
        let hidden = fam3("aa11bb22cc", "evil", &format!("p{US}q"));
        assert!(
            !hidden.contains("p\u{1f}q"),
            "the helper encodes the label as JSON"
        );
        let visible = fam3("bb22cc33dd", "ok", "shop");
        let s = parse_container_list(ContainerRuntime::Podman, format!("{hidden}\n{visible}\n"));
        assert_eq!(
            s.containers.len(),
            2,
            "the container with the odd label is still listed"
        );
        assert_eq!(s.containers[0].project, None, "an odd label is no project");
        assert_eq!(s.unreadable, 0);

        // The attack from the review: a label that carries a newline and a whole forged line. Escaped
        // by the template it stays inside one JSON string: one line, the real container, no forgery.
        let forged = [
            "ID=ffff00001111",
            "NAME=prod-db",
            "IMAGE=postgres",
            "STATE=running",
            "STATUS=Up 9 days",
            "PORTS=",
            "CREATED=x",
            "PROJECT=\"x\"",
        ]
        .join(&US.to_string());
        let evil = fam3("aa11bb22cc", "evil", &format!("p\n{forged}"));
        assert_eq!(
            evil.lines().count(),
            1,
            "the escaped label has no raw newline"
        );
        let s = parse_container_list(ContainerRuntime::Podman, format!("{evil}\n{visible}\n"));
        let names: Vec<_> = s.containers.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["evil", "ok"], "nothing forged, nothing hidden");
        // A line cut short (8 fields) is not read and is counted, not silently lost.
        let short = forged.clone();
        let s = parse_container_list(ContainerRuntime::Podman, format!("{short}\n{visible}\n"));
        assert_eq!(s.containers.len(), 1);
        assert_eq!(s.unreadable, 1);

        // A repeated id is not a second container.
        let dup = format!("{visible}\n{}\n", fam3("bb22cc33dd", "impostor", "x"));
        let s = parse_container_list(ContainerRuntime::Podman, dup);
        assert_eq!(s.containers.len(), 1);
        assert_eq!(s.containers[0].name, "ok");
        assert_eq!(s.unreadable, 1);
    }

    #[test]
    fn the_list_script_escapes_labels_and_falls_back_to_a_plain_list() {
        let t = family_template();
        assert!(t.contains(r#"json (print (.Label "com.docker.compose.project")"#));
        assert!(!family_template_plain().contains("Label"));
        let body = list_body(ContainerRuntime::Podman);
        assert!(body.contains(family_template_plain()) && body.contains("*ID=*"));
        // The plain form (7 fields) is read too.
        let plain = [
            "ID=aa11bb22cc",
            "NAME=web",
            "IMAGE=x",
            "STATE=running",
            "STATUS=Up",
            "PORTS=",
            "CREATED=x",
        ]
        .join(&US.to_string());
        let s = parse_container_list(ContainerRuntime::Nerdctl, plain);
        assert_eq!(s.containers.len(), 1);
        assert_eq!(s.containers[0].project, None);
    }

    #[test]
    fn unreadable_pods_and_instances_are_counted() {
        let doc = format!(
            r#"{{"items":[{},{{"metadata":{{"namespace":"BAD NS","name":"x"}}}}]}}"#,
            pod("a", "p", "Running", None, "")
        );
        let s = parse_container_list(ContainerRuntime::Kubernetes, doc);
        assert_eq!((s.containers.len(), s.unreadable), (1, 1));
        let s = parse_container_list(
            ContainerRuntime::Incus,
            "web,RUNNING,CONTAINER\nbroken,line\n".into(),
        );
        assert_eq!((s.containers.len(), s.unreadable), (1, 1));
    }
}
