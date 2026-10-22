//! Docker containers on the host (feature 45): the list, optional numbers per container, start /
//! stop / restart, the log tail, and the command for a shell inside a container. Everything goes
//! through the `docker` CLI over the SSH exec channel with fixed argv; the only variable text is a
//! container identifier checked by `validate_container_identifier`.

use crate::error::SshCoreError;
use crate::tmux::shell_quote;
use serde_json::Value;

/// Containers listed at most (a host with thousands is not a phone's business).
const MAX_CONTAINERS: usize = 200;
/// Log output kept (the host also cuts it).
const LOG_MAX_BYTES: usize = 1024 * 1024;
const LOG_DEFAULT_TAIL: u32 = 200;
const LOG_MAX_TAIL: u32 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ContainerState {
    Running,
    Exited,
    Paused,
    Restarting,
    Dead,
    Unknown,
}

impl ContainerState {
    /// From the `State` field of `docker ps --format '{{json .}}'`, never from the human `Status`
    /// text: "Up 4 hours (Paused)" and "(unhealthy)" would read as running there.
    pub fn from_state_field(state: &str) -> Self {
        match state.trim().to_ascii_lowercase().as_str() {
            "running" => Self::Running,
            "exited" | "created" | "removing" => Self::Exited,
            "paused" => Self::Paused,
            "restarting" => Self::Restarting,
            "dead" => Self::Dead,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DockerContainerInfo {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: ContainerState,
    /// "Up 4 hours", "Exited (1) 10m ago": for display only.
    pub status_text: String,
    pub created_at: String,
    pub ports: String,
    /// A badge only, from the health check; it never changes `state`.
    pub unhealthy: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DockerContainerStats {
    pub memory_usage_bytes: Option<u64>,
    pub memory_limit_bytes: Option<u64>,
    pub cpu_percentage: Option<f32>,
}

/// Why Docker cannot be used on the host; the app words the explanation and the way out for each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DockerUnavailable {
    NotInstalled,
    NoPermission,
    DaemonDown,
    Unexpected,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DockerContainersSnapshot {
    pub docker_available: bool,
    /// Why there are no containers (not installed, daemon down, no permission), or `None`.
    pub error_message: Option<String>,
    /// The same reason as a value, `None` when Docker answered.
    pub unavailable: Option<DockerUnavailable>,
    pub containers: Vec<DockerContainerInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DockerAction {
    Start,
    Stop,
    Restart,
}

fn invalid(reason: &str) -> SshCoreError {
    SshCoreError::InvalidInput {
        reason: reason.to_string(),
    }
}

/// A container id (hex) or name: `[A-Za-z0-9][A-Za-z0-9_.-]{1,127}`. Nothing else ever reaches a
/// command line.
#[uniffi::export]
pub fn validate_container_identifier(id: String) -> Result<String, SshCoreError> {
    let ok = (2..=128).contains(&id.len())
        && id.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
    if ok {
        Ok(id)
    } else {
        Err(invalid("Invalid container name or id"))
    }
}

const DOCKER_PATH: &str = "export PATH=\"/usr/local/bin:/opt/homebrew/bin:/Applications/Docker.app/Contents/Resources/bin:$HOME/.docker/bin:$PATH\"";

/// Runs `body` (a POSIX script that uses `docker`) the way the app runs everything on the host:
/// the user's login shell for PATH, a host-side watchdog, stderr merged into stdout.
fn docker_script(body: &str, secs: u32) -> String {
    crate::hostprobe::login_sh_script(&format!("{DOCKER_PATH}; {body}"), &[], secs)
}

/// All containers, one JSON object per line, or one of the marker lines parsed below.
#[uniffi::export]
pub fn docker_list_command() -> String {
    docker_script(
        "if command -v docker >/dev/null 2>&1; then docker ps -a --format '{{json .}}' 2>&1; else echo DOCKER_NOT_INSTALLED; fi",
        10,
    )
}

#[uniffi::export]
pub fn docker_stats_command(container_id: String) -> Result<String, SshCoreError> {
    let id = validate_container_identifier(container_id)?;
    Ok(docker_script(
        &format!(
            "docker stats --no-stream --format '{{{{json .}}}}' {} 2>&1",
            shell_quote(&id)
        ),
        10,
    ))
}

#[uniffi::export]
pub fn docker_action_command(
    action: DockerAction,
    container_id: String,
) -> Result<String, SshCoreError> {
    let id = validate_container_identifier(container_id)?;
    let verb = match action {
        DockerAction::Start => "start",
        DockerAction::Stop => "stop",
        DockerAction::Restart => "restart",
    };
    // `docker stop` waits up to 10 s for the process; the host cuts the whole call at 40 s.
    Ok(docker_script(
        &format!("docker {verb} {} 2>&1 && echo DOCKER_OK", shell_quote(&id)),
        40,
    ))
}

#[uniffi::export]
pub fn docker_logs_command(container_id: String, tail_lines: u32) -> Result<String, SshCoreError> {
    let id = validate_container_identifier(container_id)?;
    let tail = if tail_lines == 0 {
        LOG_DEFAULT_TAIL
    } else {
        tail_lines.min(LOG_MAX_TAIL)
    };
    Ok(docker_script(
        &format!(
            "docker logs --tail {tail} --timestamps {} 2>&1 | tail -c {}",
            shell_quote(&id),
            LOG_MAX_BYTES + 1
        ),
        15,
    ))
}

/// The shell in a container: bash when it has one, else sh. The pane stays open with the exit
/// code when the container stops, until Enter is pressed. No `--privileged`, no `--user`.
#[uniffi::export]
pub fn build_docker_exec_command(container_id: String) -> Result<String, SshCoreError> {
    let id = validate_container_identifier(container_id)?;
    let q = shell_quote(&id);
    Ok(format!(
        "{DOCKER_PATH}; docker exec -it {q} sh -c 'command -v bash >/dev/null 2>&1 && exec bash || exec sh'; \
         c=$?; printf '\\r\\n\\033[33m[Container session ended (exit code %s)]\\033[0m\\r\\nPress Enter to close.\\r\\n' \"$c\"; read -r _"
    ))
}

/// The tmux session that keeps a container shell alive: `docker-<name>-<first 6 of the id>`. The
/// id suffix keeps `a.b` and `a-b` apart after the dots are turned into dashes.
#[uniffi::export]
pub fn docker_session_name(
    container_name: String,
    container_id: String,
) -> Result<String, SshCoreError> {
    let id = validate_container_identifier(container_id)?;
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
    let suffix: String = id.chars().take(6).collect();
    let keep = 64usize.saturating_sub("docker-".len() + 1 + suffix.len());
    let slug: String = slug.chars().take(keep).collect();
    let name = if slug.is_empty() {
        format!("docker-{suffix}")
    } else {
        format!("docker-{slug}-{suffix}")
    };
    crate::tmux::validate_session_name(&name)?;
    Ok(name)
}

fn clean(s: &str, max: usize) -> String {
    crate::transcript::strip_ansi_codes(s.to_string())
        .chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect()
}

fn str_of(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// Reads the output of `docker_list_command`.
#[uniffi::export]
pub fn parse_docker_list(output: String) -> DockerContainersSnapshot {
    let none = |why: DockerUnavailable, msg: &str| DockerContainersSnapshot {
        docker_available: false,
        error_message: Some(msg.to_string()),
        unavailable: Some(why),
        containers: vec![],
    };
    if output.lines().any(|l| l.trim() == "DOCKER_NOT_INSTALLED") {
        return none(
            DockerUnavailable::NotInstalled,
            "Docker is not installed on this host",
        );
    }
    let mut containers = Vec::new();
    let mut other: Vec<&str> = Vec::new();
    for line in output.lines().map(str::trim).filter(|l| !l.is_empty()) {
        match serde_json::from_str::<Value>(line) {
            Ok(v) if v.is_object() && v.get("ID").is_some() => {
                if containers.len() >= MAX_CONTAINERS {
                    continue;
                }
                let status = str_of(&v, "Status");
                let health = str_of(&v, "HealthStatus");
                let id = str_of(&v, "ID");
                if validate_container_identifier(id.clone()).is_err() {
                    continue;
                }
                containers.push(DockerContainerInfo {
                    id,
                    name: clean(str_of(&v, "Names").split(',').next().unwrap_or(""), 128),
                    image: clean(&str_of(&v, "Image"), 200),
                    state: ContainerState::from_state_field(&str_of(&v, "State")),
                    status_text: clean(&status, 120),
                    created_at: clean(&str_of(&v, "CreatedAt"), 60),
                    ports: clean(&str_of(&v, "Ports"), 200),
                    unhealthy: health.eq_ignore_ascii_case("unhealthy")
                        || status.contains("(unhealthy)"),
                });
            }
            _ => other.push(line),
        }
    }
    if containers.is_empty() && !other.is_empty() {
        let text = other.join(" ");
        let low = text.to_ascii_lowercase();
        return if low.contains("permission denied") {
            none(
                DockerUnavailable::NoPermission,
                "No permission to use Docker (the user is not in the docker group)",
            )
        } else if low.contains("cannot connect") || low.contains("is the docker daemon running") {
            none(
                DockerUnavailable::DaemonDown,
                "The Docker daemon is not running",
            )
        } else {
            none(
                DockerUnavailable::Unexpected,
                "Docker did not answer as expected",
            )
        };
    }
    DockerContainersSnapshot {
        docker_available: true,
        error_message: None,
        unavailable: None,
        containers,
    }
}

/// "142MiB", "3.9GiB", "1.5kB", "0B" to bytes (docker prints both SI and IEC units).
fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let split = s.find(|c: char| c.is_ascii_alphabetic())?;
    let (num, unit) = s.split_at(split);
    let n: f64 = num.trim().parse().ok()?;
    let mult: f64 = match unit.trim().to_ascii_lowercase().as_str() {
        "b" => 1.0,
        "kb" => 1e3,
        "kib" => 1024.0,
        "mb" => 1e6,
        "mib" => 1024.0 * 1024.0,
        "gb" => 1e9,
        "gib" => 1024.0 * 1024.0 * 1024.0,
        "tb" => 1e12,
        "tib" => 1024f64.powi(4),
        _ => return None,
    };
    (n >= 0.0 && n.is_finite()).then_some((n * mult) as u64)
}

/// Reads the output of `docker_stats_command` (one JSON object).
#[uniffi::export]
pub fn parse_docker_stats(output: String) -> DockerContainerStats {
    let v = output
        .lines()
        .find_map(|l| {
            serde_json::from_str::<Value>(l.trim())
                .ok()
                .filter(Value::is_object)
        })
        .unwrap_or(Value::Null);
    let mem = str_of(&v, "MemUsage");
    let mut parts = mem.split('/');
    let usage = parts.next().and_then(parse_size);
    let limit = parts.next().and_then(parse_size);
    let cpu = str_of(&v, "CPUPerc")
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

/// Reads the output of `docker_action_command`: `Ok` when the host confirmed, else the (cleaned)
/// first line of what docker said.
#[uniffi::export]
pub fn parse_docker_action(output: String) -> Result<(), SshCoreError> {
    if output.lines().any(|l| l.trim() == "DOCKER_OK") {
        return Ok(());
    }
    let first = output
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("no answer from the host");
    Err(SshCoreError::IoError {
        reason: format!("Docker: {}", clean(first, 200)),
    })
}

/// Makes the log text safe to show: no escape sequences or control characters, cut at 1 MB.
#[uniffi::export]
pub fn clean_docker_logs(output: String) -> String {
    let truncated = output.len() > LOG_MAX_BYTES;
    let mut text = crate::transcript::strip_ansi_codes(output);
    if truncated || text.len() > LOG_MAX_BYTES {
        let mut cut = LOG_MAX_BYTES.min(text.len());
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str("\n[Log truncated at 1 MB limit]");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: &str, name: &str, state: &str, status: &str) -> String {
        format!(
            r#"{{"ID":"{id}","Names":"{name}","Image":"postgres:16","State":"{state}","Status":"{status}","CreatedAt":"2026-10-20 14:02:11 +0000 UTC","Ports":"0.0.0.0:5432->5432/tcp"}}"#
        )
    }

    #[test]
    fn identifiers_are_checked() {
        for ok in ["web-app_1", "postgres-db", "e3b0c44298fc", "a.b-c", "ab"] {
            assert!(validate_container_identifier(ok.into()).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "a",
            "-x",
            ".x",
            "app; rm -rf /",
            "app$(whoami)",
            "a b",
            "a`b`",
            "a|b",
            "a\nb",
            "a'b",
            "a\"b",
            "../x",
            "é",
            &"x".repeat(129),
        ] {
            assert!(
                validate_container_identifier(bad.into()).is_err(),
                "{bad:?}"
            );
        }
        assert!(docker_action_command(DockerAction::Restart, "a; reboot".into()).is_err());
        assert!(docker_logs_command("$(id)".into(), 10).is_err());
        assert!(docker_stats_command("a b".into()).is_err());
        assert!(build_docker_exec_command("a;b".into()).is_err());
    }

    #[test]
    fn the_state_field_wins_over_the_status_text() {
        let out = [
            line("aaaaaaaaaaaa", "up", "running", "Up 4 hours (Paused)"),
            line("bbbbbbbbbbbb", "paused", "paused", "Up 4 hours (Paused)"),
            line("cccccccccccc", "made", "created", "Created"),
            line("dddddddddddd", "sick", "running", "Up 2 hours (unhealthy)"),
            line("eeeeeeeeeeee", "gone", "exited", "Exited (1) 10m ago"),
            line(
                "ffffffffffff",
                "loop",
                "restarting",
                "Restarting (1) 5 seconds ago",
            ),
            line("111111111111", "odd", "weird", "?"),
        ]
        .join("\n");
        let s = parse_docker_list(out);
        assert!(s.docker_available && s.error_message.is_none());
        let st: Vec<_> = s.containers.iter().map(|c| c.state).collect();
        assert_eq!(
            st,
            vec![
                ContainerState::Running,
                ContainerState::Paused,
                ContainerState::Exited,
                ContainerState::Running,
                ContainerState::Exited,
                ContainerState::Restarting,
                ContainerState::Unknown
            ]
        );
        assert!(s.containers[3].unhealthy && !s.containers[0].unhealthy);
        assert_eq!(s.containers[0].name, "up");
        assert_eq!(s.containers[0].ports, "0.0.0.0:5432->5432/tcp");
    }

    #[test]
    fn docker_trouble_is_reported_in_words() {
        let p = parse_docker_list("DOCKER_NOT_INSTALLED\n".into());
        assert_eq!(p.unavailable, Some(DockerUnavailable::NotInstalled));
        assert!(!p.docker_available && p.error_message.unwrap().contains("not installed"));
        let p = parse_docker_list(
            "permission denied while trying to connect to the Docker daemon socket".into(),
        );
        assert_eq!(p.unavailable, Some(DockerUnavailable::NoPermission));
        assert!(p.error_message.unwrap().contains("permission"));
        let p = parse_docker_list("Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?".into());
        assert_eq!(p.unavailable, Some(DockerUnavailable::DaemonDown));
        assert!(p.error_message.unwrap().contains("daemon"));
        let p = parse_docker_list(String::new());
        assert!(p.docker_available && p.containers.is_empty());
    }

    #[test]
    fn hostile_names_and_floods_are_cleaned_and_bounded() {
        let nasty = r#"{"ID":"abcdef123456","Names":"evil\u001b]52;c;aGk=\u0007name","Image":"i\u001b[31m","State":"running","Status":"Up","CreatedAt":"","Ports":""}"#;
        let s = parse_docker_list(nasty.into());
        assert_eq!(
            s.containers[0].name,
            "evil]52;c;aGk=name".replace("]52;c;aGk=", "")
        ); // OSC removed whole
        assert!(!s.containers[0].image.contains('\u{1b}'));
        // an id that is not a plain identifier is dropped
        let bad = r#"{"ID":"x; rm","Names":"n","Image":"i","State":"running","Status":"Up"}"#;
        assert!(parse_docker_list(bad.into()).containers.is_empty());
        let many: Vec<String> = (0..500)
            .map(|i| line(&format!("{i:012x}"), "n", "running", "Up"))
            .collect();
        assert_eq!(
            parse_docker_list(many.join("\n")).containers.len(),
            MAX_CONTAINERS
        );
    }

    #[test]
    fn stats_are_read_in_both_unit_families() {
        let s =
            parse_docker_stats(r#"{"CPUPerc":"12.40%","MemUsage":"148.3MiB / 3.844GiB"}"#.into());
        assert_eq!(s.cpu_percentage, Some(12.4));
        assert_eq!(s.memory_usage_bytes, Some((148.3 * 1024.0 * 1024.0) as u64));
        assert_eq!(
            s.memory_limit_bytes,
            Some((3.844 * 1024.0 * 1024.0 * 1024.0) as u64)
        );
        let s = parse_docker_stats(r#"{"CPUPerc":"--","MemUsage":"0B / 0B"}"#.into());
        assert_eq!((s.cpu_percentage, s.memory_usage_bytes), (None, Some(0)));
        let s = parse_docker_stats("Error: no such container".into());
        assert_eq!(s.memory_usage_bytes, None);
        assert_eq!(parse_size("1.5kB"), Some(1500));
        assert_eq!(parse_size("-3MiB"), None);
        assert_eq!(parse_size("5 parsecs"), None);
    }

    #[test]
    fn actions_and_logs_are_read_and_bounded() {
        assert!(parse_docker_action("web\nDOCKER_OK\n".into()).is_ok());
        let e = parse_docker_action("Error response from daemon: No such container: x\n".into())
            .unwrap_err();
        assert!(e.to_string().contains("No such container"));
        assert!(docker_logs_command("web".into(), 0)
            .unwrap()
            .contains("--tail 200"));
        assert!(docker_logs_command("web".into(), 99999)
            .unwrap()
            .contains("--tail 1000"));
        let big = "x".repeat(LOG_MAX_BYTES + 500);
        let cleaned = clean_docker_logs(big);
        assert!(cleaned.ends_with("[Log truncated at 1 MB limit]"));
        assert!(cleaned.len() < LOG_MAX_BYTES + 100);
        assert_eq!(
            clean_docker_logs("a\u{1b}[31mred\u{1b}[0m\nb".into()),
            "ared\nb"
        );
    }

    #[test]
    fn session_names_do_not_collide() {
        let a = docker_session_name("a.b".into(), "abcdef123456".into()).unwrap();
        let b = docker_session_name("a-b".into(), "123456abcdef".into()).unwrap();
        assert_ne!(a, b);
        assert_eq!(a, "docker-a-b-abcdef");
        let long = docker_session_name("n".repeat(300), "abcdef123456".into()).unwrap_or_default();
        assert!(long.len() <= 64 && long.starts_with("docker-"));
        assert_eq!(
            docker_session_name("...".into(), "abcdef123456".into()).unwrap(),
            "docker-abcdef"
        );
    }

    #[test]
    fn the_exec_command_runs_and_reports_the_exit_code() {
        // A container that does not exist: docker fails fast (or is missing), the pane still says why.
        let cmd = build_docker_exec_command("no-such-container-xyz".into()).unwrap();
        assert!(cmd.contains("docker exec -it 'no-such-container-xyz'"));
        assert!(!cmd.contains("--privileged") && !cmd.contains("--user"));
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            text.contains("Container session ended (exit code"),
            "{text:?}"
        );
    }

    /// Read-only, against the docker of this machine when its daemon answers.
    #[test]
    fn the_list_command_works_against_a_real_docker_when_there_is_one() {
        let up = std::process::Command::new("docker")
            .args(["info", "--format", "{{.ID}}"])
            .output();
        if !up.is_ok_and(|o| o.status.success()) {
            eprintln!("SKIP: no docker daemon");
            return;
        }
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(docker_list_command())
            .env("SHELL", "/bin/sh")
            .output()
            .unwrap();
        let s = parse_docker_list(String::from_utf8_lossy(&out.stdout).to_string());
        assert!(s.docker_available, "{:?}", s.error_message);
        for c in &s.containers {
            assert!(validate_container_identifier(c.id.clone()).is_ok());
        }
    }
}
