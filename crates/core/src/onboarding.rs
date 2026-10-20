//! Zero-touch host onboarding (feature 44): putting the phone's key on a new host, and reading
//! what the host looks like.
//!
//! - `pairing_paste_command` is the command a user pastes on the host (or the app runs once over a
//!   password login): it appends the phone's public key to `~/.ssh/authorized_keys` and needs no
//!   helper. The key and the device name are validated here first, so the script holds only
//!   characters that mean nothing to a shell.
//! - `pair_with_password` is the one place a password is used: it logs in once, runs that command,
//!   and forgets the password. The host key is shown to the user and pinned before it is sent.
//! - With the helper installed, `pair` (stdin) does the same more carefully, and `doctor` returns
//!   facts about the host that `onboarding_findings` turns into suggestions.

use crate::error::SshCoreError;
use crate::hostprobe::login_sh_script;
use crate::tmux::shell_quote;
use crate::usage::validate_sha256;
use serde::Deserialize;
use zeroize::Zeroizing;

fn bad(what: &str) -> SshCoreError {
    crate::agents::invalid(what)
}

const ED25519_PREFIX: &str = "AAAAC3NzaC1lZDI1NTE5AAAA";
const SK_ED25519_PREFIX: &str = "AAAAGnNrLXNzaC1lZDI1NTE5QG9wZW5zc2guY29t";

/// `(type, blob)` of a plain public key (no options); a trailing comment is dropped.
fn split_key(line: &str) -> Result<(&str, &str), SshCoreError> {
    let line = line.trim_end_matches(['\n', '\r']);
    if line.len() > 600 || line.chars().any(char::is_control) {
        return Err(bad("Not a public key"));
    }
    let mut parts = line.split(' ');
    let (Some(kind), Some(blob)) = (parts.next(), parts.next()) else {
        return Err(bad("Not a public key"));
    };
    let shape = match kind {
        "ssh-ed25519" => blob.len() == 68 && blob.starts_with(ED25519_PREFIX),
        "sk-ssh-ed25519@openssh.com" => blob.len() <= 400 && blob.starts_with(SK_ED25519_PREFIX),
        _ => false,
    };
    let alphabet = blob
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='));
    if shape && alphabet {
        Ok((kind, blob))
    } else {
        Err(bad("Not a supported public key"))
    }
}

/// Letters, digits, space and `._-`, 1 to 40 characters, no space at either end.
fn validate_device_name(name: &str) -> Result<(), SshCoreError> {
    let n = name.chars().count();
    if (1..=40).contains(&n)
        && name.trim() == name
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'))
    {
        Ok(())
    } else {
        Err(bad(
            "The device name may hold letters, digits, spaces and . _ - (1 to 40)",
        ))
    }
}

/// The comment the key gets in `authorized_keys`.
fn comment(name: &str) -> String {
    format!("ssh-app: {name}")
}

/// A command for a person to paste on the host, or for the app to run once. It runs under `sh`
/// whatever the user's shell is, refuses a symlinked `~/.ssh` or `authorized_keys`, sets the usual
/// modes, completes a missing final newline, and adds the key only when it is not there yet.
pub fn pairing_command(public_key: &str, device_name: &str) -> Result<String, SshCoreError> {
    let (kind, blob) = split_key(public_key)?;
    validate_device_name(device_name)?;
    let line = format!("{kind} {blob} {}", comment(device_name));
    // Everything between the quotes is a validated, shell-neutral alphabet; `$` appears only in the
    // fixed variable names below.
    let script = format!(
        "umask 077; d=\"$HOME/.ssh\"; f=\"$d/authorized_keys\"; \
         if [ -L \"$d\" ] || [ -L \"$f\" ]; then echo \"ssh-app: refused, a symlink\"; exit 0; fi; \
         mkdir -p \"$d\" && chmod 700 \"$d\" && touch \"$f\" && chmod 600 \"$f\" \
         || {{ echo \"ssh-app: failed\"; exit 0; }}; \
         if grep -v '^[[:space:]]*#' \"$f\" | grep -qF -- \"{blob}\"; then echo \"ssh-app: key already present\"; exit 0; fi; \
         [ -z \"$(tail -c1 \"$f\")\" ] || echo >> \"$f\"; \
         echo \"{line}\" >> \"$f\" && echo \"ssh-app: key added\""
    );
    Ok(format!("sh -c {}", shell_quote(&script)))
}

/// What the pasted or remotely run command reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PairingState {
    Added,
    AlreadyPresent,
    Refused,
    Failed,
}

pub(crate) fn parse_pairing_output(out: &str) -> PairingState {
    if out.contains("ssh-app: key already present") {
        PairingState::AlreadyPresent
    } else if out.contains("ssh-app: key added") {
        PairingState::Added
    } else if out.contains("ssh-app: refused") {
        PairingState::Refused
    } else {
        PairingState::Failed
    }
}

/// The command to paste on the host (feature 44, path B), checked and ready.
#[uniffi::export]
pub fn pairing_paste_command(
    public_key: String,
    device_name: String,
) -> Result<String, SshCoreError> {
    pairing_command(&public_key, &device_name)
}

fn validate_endpoint(host: &str, port: u16, username: &str) -> Result<(), SshCoreError> {
    let host_ok = !host.is_empty()
        && host.len() <= 253
        && !host.starts_with('-')
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'_' | b'%'));
    let user_ok = !username.is_empty()
        && username.len() <= 64
        && !username.starts_with('-')
        && username
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if host_ok && user_ok && port != 0 {
        Ok(())
    } else {
        Err(bad("The server address or the user name is not valid"))
    }
}

/// The host key a server presents, to show to the user before anything secret is sent.
#[uniffi::export(async_runtime = "tokio")]
pub async fn probe_server_host_key(host: String, port: u16) -> Result<String, SshCoreError> {
    validate_endpoint(&host, port, "user")?;
    crate::transport::probe_host_key(&host, port).await
}

/// Logs in once with a password, puts the phone's key on the host and forgets the password.
/// `host_key` is the fingerprint the user confirmed (`probe_server_host_key`); another key aborts
/// before the password is sent.
#[uniffi::export(async_runtime = "tokio")]
pub async fn pair_with_password(
    host: String,
    port: u16,
    username: String,
    password: String,
    host_key: String,
    public_key: String,
    device_name: String,
) -> Result<PairingState, SshCoreError> {
    let password = Zeroizing::new(password);
    validate_endpoint(&host, port, &username)?;
    if password.is_empty() || password.len() > 256 || password.contains('\0') {
        return Err(bad("The password is not valid"));
    }
    if !host_key.starts_with("SHA256:")
        || host_key.len() > 80
        || !host_key.bytes().all(|b| b.is_ascii_graphic())
    {
        return Err(bad("The host key fingerprint is not valid"));
    }
    let command = pairing_command(&public_key, &device_name)?;
    let conn =
        crate::transport::connect_with_password(&host, port, &username, &password, &host_key)
            .await?;
    drop(password);
    let channel = crate::transport::exec_start(&conn.handle, &command).await?;
    let out = crate::transport::collect_output(channel, std::time::Duration::from_secs(20)).await?;
    let _ = conn
        .handle
        .disconnect(russh::Disconnect::ByApplication, "", "en")
        .await;
    Ok(parse_pairing_output(&out.text()))
}

// ---- the helper's `pair` -------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HostPairResult {
    pub ok: bool,
    /// False when the key was already there.
    pub added: bool,
    /// The file's mode was too open and was set to `0600`.
    pub tightened: bool,
    /// A short reason, empty when `ok`.
    pub message: String,
}

/// The command and the JSON for its stdin (the key never appears in the command line).
pub(crate) fn build_pair_command(
    sha256: &str,
    public_key: &str,
    device_name: &str,
) -> Result<(String, String), SshCoreError> {
    validate_sha256(sha256)?;
    let (kind, blob) = split_key(public_key)?;
    validate_device_name(device_name)?;
    let json = serde_json::json!({"key": format!("{kind} {blob}"), "name": device_name});
    Ok((crate::push::command(sha256, "pair", true), json.to_string()))
}

fn last_object(out: &str) -> Option<serde_json::Value> {
    out.lines()
        .rev()
        .find_map(|l| serde_json::from_str::<serde_json::Value>(l.trim()).ok())
        .filter(serde_json::Value::is_object)
}

pub(crate) fn parse_pair_result(out: &str) -> HostPairResult {
    let fail = |message: &str| HostPairResult {
        ok: false,
        added: false,
        tightened: false,
        message: message.to_string(),
    };
    if out.lines().any(|l| l.trim() == "HELPER_ABSENT") {
        return fail("The host helper is not installed or is another version");
    }
    let Some(v) = last_object(out) else {
        return fail("No answer from the host");
    };
    if v["ok"] == true {
        return HostPairResult {
            ok: true,
            added: v["added"] == true,
            tightened: v["tightened"] == true,
            message: String::new(),
        };
    }
    let reason: String = v["error"]
        .as_str()
        .unwrap_or("failed")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '-' | ',' | '~' | '/'))
        .take(80)
        .collect();
    fail(&reason)
}

// ---- the helper's `doctor` -----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Default, uniffi::Record)]
pub struct DoctorTool {
    pub installed: bool,
    /// Empty when unknown.
    pub version: String,
}

/// Facts about a host, as the helper's `doctor` reports them.
#[derive(Debug, Clone, PartialEq, Eq, Default, uniffi::Record)]
pub struct HostDoctor {
    pub helper_version: String,
    pub os: String,
    pub arch: String,
    pub tmux: DoctorTool,
    pub git: DoctorTool,
    pub python3: DoctorTool,
    pub tailscale: DoctorTool,
    /// The host's Tailscale IPv4, empty when unknown.
    pub tailscale_ip: String,
    pub claude_found: bool,
    pub claude_settings_exists: bool,
    pub antigravity_found: bool,
    pub antigravity_hooks_file_exists: bool,
    pub ollama_installed: bool,
    pub ollama_running: bool,
    pub approval_key_trusted: bool,
    pub hook_script_installed: bool,
    pub helper_dir_private: bool,
}

#[derive(Deserialize, Default)]
struct RawTool {
    #[serde(default)]
    installed: bool,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    ip: Option<String>,
    #[serde(default)]
    running: bool,
}

#[derive(Deserialize, Default)]
struct RawAgent {
    #[serde(default)]
    found: bool,
    #[serde(default)]
    settings_exists: bool,
    #[serde(default)]
    hooks_file_exists: bool,
}

#[derive(Deserialize, Default)]
struct RawAgents {
    #[serde(default)]
    claude: RawAgent,
    #[serde(default)]
    antigravity: RawAgent,
    #[serde(default)]
    ollama: RawTool,
}

#[derive(Deserialize, Default)]
struct RawSshApp {
    #[serde(default)]
    allowed_signers: bool,
    #[serde(default)]
    hook_script: bool,
    #[serde(default)]
    helper_dir_private: bool,
}

#[derive(Deserialize)]
struct RawDoctor {
    ok: bool,
    #[serde(default)]
    version: String,
    #[serde(default)]
    os: String,
    #[serde(default)]
    arch: String,
    #[serde(default)]
    tmux: RawTool,
    #[serde(default)]
    git: RawTool,
    #[serde(default)]
    python3: RawTool,
    #[serde(default)]
    tailscale: RawTool,
    #[serde(default)]
    agents: RawAgents,
    #[serde(default)]
    ssh_app: RawSshApp,
}

fn word(s: &str, max: usize) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .take(max)
        .collect()
}

fn ipv4(s: &str) -> String {
    let parts: Vec<&str> = s.split('.').collect();
    let ok = parts.len() == 4
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 3
                && p.bytes().all(|b| b.is_ascii_digit())
                && p.parse::<u16>().is_ok_and(|n| n <= 255)
        });
    if ok {
        s.to_string()
    } else {
        String::new()
    }
}

fn tool(t: &RawTool) -> DoctorTool {
    DoctorTool {
        installed: t.installed,
        version: t
            .version
            .as_deref()
            .filter(|_| t.installed)
            .map(|v| word(v, 24))
            .unwrap_or_default(),
    }
}

pub(crate) fn parse_host_doctor(out: &str) -> Option<HostDoctor> {
    let raw: RawDoctor = out.lines().find_map(|l| {
        let l = l.trim();
        if !l.starts_with('{') {
            return None;
        }
        serde_json::from_str::<RawDoctor>(l).ok().filter(|d| d.ok)
    })?;
    Some(HostDoctor {
        helper_version: word(&raw.version, 24),
        os: word(&raw.os, 16),
        arch: word(&raw.arch, 16),
        tmux: tool(&raw.tmux),
        git: tool(&raw.git),
        python3: tool(&raw.python3),
        tailscale: tool(&raw.tailscale),
        tailscale_ip: if raw.tailscale.installed {
            raw.tailscale.ip.as_deref().map(ipv4).unwrap_or_default()
        } else {
            String::new()
        },
        claude_found: raw.agents.claude.found,
        claude_settings_exists: raw.agents.claude.found && raw.agents.claude.settings_exists,
        antigravity_found: raw.agents.antigravity.found,
        antigravity_hooks_file_exists: raw.agents.antigravity.found
            && raw.agents.antigravity.hooks_file_exists,
        ollama_installed: raw.agents.ollama.installed,
        ollama_running: raw.agents.ollama.installed && raw.agents.ollama.running,
        approval_key_trusted: raw.ssh_app.allowed_signers,
        hook_script_installed: raw.ssh_app.hook_script,
        helper_dir_private: raw.ssh_app.helper_dir_private,
    })
}

pub(crate) fn build_doctor_command(sha256: &str) -> Result<String, SshCoreError> {
    validate_sha256(sha256)?;
    Ok(login_sh_script(
        &format!(
            "h={path}; if [ ! -f \"$h\" ] || [ -L \"$h\" ] || \
             [ \"{digest}\" != '{sha256}' ]; then echo HELPER_ABSENT; \
             else \"$h\" doctor 2>/dev/null; fi",
            path = crate::usage::HELPER_PATH_EXPR,
            digest = crate::hostprobe::SHA256_OF_H
        ),
        &[],
        10,
    ))
}

// ---- suggestions ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FindingLevel {
    /// Something that blocks a feature.
    Needed,
    /// Worth doing.
    Suggested,
    /// Only a fact.
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OnboardingFinding {
    /// Stable key for the app to attach an action to (`install-tmux`, `hooks-claude`, ...).
    pub id: String,
    pub level: FindingLevel,
    pub title: String,
    pub detail: String,
}

fn finding(id: &str, level: FindingLevel, title: &str, detail: &str) -> OnboardingFinding {
    OnboardingFinding {
        id: id.to_string(),
        level,
        title: title.to_string(),
        detail: detail.to_string(),
    }
}

/// What to offer after pairing, in the order it matters.
pub fn findings_for(d: &HostDoctor) -> Vec<OnboardingFinding> {
    use FindingLevel::{Info, Needed, Suggested};
    let mut v = Vec::new();
    if !d.tmux.installed {
        v.push(finding(
            "install-tmux",
            Needed,
            "tmux is not installed",
            "Sessions live in tmux so they survive the phone going to sleep. Install it with your package manager.",
        ));
    }
    if !d.python3.installed {
        v.push(finding(
            "install-python3",
            Suggested,
            "Python 3 is not installed",
            "The agent hook script starts through Python 3; without it approvals from the phone do not run.",
        ));
    }
    if !d.approval_key_trusted {
        v.push(finding(
            "trust-approval-key",
            Suggested,
            "Phone approvals are not set up",
            "Trust this phone's key on the host so Approve and Deny can be verified.",
        ));
    }
    if d.claude_found {
        v.push(finding(
            "hooks-claude",
            Suggested,
            "Claude Code found",
            "Install the approval hook so its permission requests reach the phone.",
        ));
    }
    if d.antigravity_found {
        v.push(finding(
            "hooks-antigravity",
            Suggested,
            "Antigravity found",
            "Install the approval hook so its tool requests reach the phone.",
        ));
    }
    if (d.claude_found || d.antigravity_found) && !d.hook_script_installed {
        v.push(finding(
            "install-hook-script",
            Needed,
            "The hook script is not installed",
            "The helper installs it together with the hooks.",
        ));
    }
    if d.hook_script_installed && !d.helper_dir_private {
        v.push(finding(
            "private-dir",
            Needed,
            "~/.ssh-app is not private",
            "Other users could read or change the approval queue. Run chmod 700 ~/.ssh-app.",
        ));
    }
    if d.ollama_installed && !d.ollama_running {
        v.push(finding(
            "ollama-stopped",
            Info,
            "Ollama is installed but not running",
            "Start it to use local models.",
        ));
    }
    if !d.tailscale.installed {
        v.push(finding(
            "no-tailscale",
            Info,
            "Tailscale was not found on the host",
            "Reaching this host from outside your network needs a VPN such as Tailscale.",
        ));
    }
    if !d.git.installed {
        v.push(finding(
            "no-git",
            Info,
            "git is not installed",
            "The project view needs git.",
        ));
    }
    v
}

#[uniffi::export]
pub fn onboarding_findings(doctor: HostDoctor) -> Vec<OnboardingFinding> {
    findings_for(&doctor)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOB: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";
    const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    fn key() -> String {
        format!("ssh-ed25519 {BLOB}")
    }

    fn run_at(home: &std::path::Path, cmd: &str) -> String {
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", home)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    #[test]
    fn the_paste_command_adds_a_key_once_and_sets_private_modes() {
        use std::os::unix::fs::MetadataExt;
        let h = tempfile::tempdir().unwrap();
        let cmd = pairing_command(&key(), "My iPhone").unwrap();
        assert_eq!(
            parse_pairing_output(&run_at(h.path(), &cmd)),
            PairingState::Added
        );
        assert_eq!(
            parse_pairing_output(&run_at(h.path(), &cmd)),
            PairingState::AlreadyPresent
        );
        let f = h.path().join(".ssh/authorized_keys");
        assert_eq!(
            std::fs::read_to_string(&f).unwrap(),
            format!("{} ssh-app: My iPhone\n", key())
        );
        assert_eq!(std::fs::metadata(&f).unwrap().mode() & 0o777, 0o600);
        assert_eq!(
            std::fs::metadata(h.path().join(".ssh")).unwrap().mode() & 0o777,
            0o700
        );
    }

    #[test]
    fn the_paste_command_completes_a_missing_final_newline() {
        let h = tempfile::tempdir().unwrap();
        std::fs::create_dir(h.path().join(".ssh")).unwrap();
        std::fs::write(h.path().join(".ssh/authorized_keys"), "ssh-rsa OLD").unwrap();
        let cmd = pairing_command(&key(), "P").unwrap();
        assert_eq!(
            parse_pairing_output(&run_at(h.path(), &cmd)),
            PairingState::Added
        );
        assert_eq!(
            std::fs::read_to_string(h.path().join(".ssh/authorized_keys")).unwrap(),
            format!("ssh-rsa OLD\n{} ssh-app: P\n", key())
        );
    }

    #[test]
    fn a_commented_out_key_is_not_taken_for_an_authorized_one() {
        let h = tempfile::tempdir().unwrap();
        std::fs::create_dir(h.path().join(".ssh")).unwrap();
        let old = format!("# {} old phone\n", key());
        std::fs::write(h.path().join(".ssh/authorized_keys"), &old).unwrap();
        let cmd = pairing_command(&key(), "P").unwrap();
        assert_eq!(
            parse_pairing_output(&run_at(h.path(), &cmd)),
            PairingState::Added
        );
        assert_eq!(
            std::fs::read_to_string(h.path().join(".ssh/authorized_keys")).unwrap(),
            format!("{old}{} ssh-app: P\n", key())
        );
    }

    #[test]
    fn the_paste_command_refuses_a_symlink_and_changes_nothing() {
        let h = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(other.path(), h.path().join(".ssh")).unwrap();
        let cmd = pairing_command(&key(), "P").unwrap();
        assert_eq!(
            parse_pairing_output(&run_at(h.path(), &cmd)),
            PairingState::Refused
        );
        assert_eq!(std::fs::read_dir(other.path()).unwrap().count(), 0);
    }

    #[test]
    fn hostile_keys_and_names_never_reach_a_command() {
        for bad in [
            "".to_string(),
            "ssh-rsa AAAAB3NzaC1yc2E".into(),
            format!("{}\n{}", key(), key()),
            format!("command=\"x\" {}", key()),
            format!("ssh-ed25519 {}'; rm -rf ~; '", &BLOB[..40]),
            format!("ssh-ed25519 {}", BLOB.replace('I', "$")),
        ] {
            assert!(pairing_command(&bad, "P").is_err(), "{bad:?}");
        }
        for bad in [
            "",
            "a\nb",
            "x'; id #",
            "$(id)",
            "`id`",
            "a\"b",
            " a",
            "é",
            &"n".repeat(41),
        ] {
            assert!(pairing_command(&key(), bad).is_err(), "{bad:?}");
        }
        let cmd = pairing_command(&format!("{} laptop comment", key()), "iPad 2").unwrap();
        assert!(!cmd.contains("laptop"));
        assert!(cmd.starts_with("sh -c '"));
    }

    #[test]
    fn pairing_output_is_classified() {
        assert_eq!(
            parse_pairing_output("ssh-app: key added\n"),
            PairingState::Added
        );
        assert_eq!(
            parse_pairing_output("ssh-app: refused, a symlink"),
            PairingState::Refused
        );
        assert_eq!(
            parse_pairing_output("Welcome\nsh: mkdir: nope"),
            PairingState::Failed
        );
        assert_eq!(parse_pairing_output(""), PairingState::Failed);
    }

    #[test]
    fn endpoint_checks_refuse_option_like_and_odd_values() {
        assert!(validate_endpoint("mac.tail1234.ts.net", 22, "dev").is_ok());
        assert!(validate_endpoint("100.64.0.7", 2222, "dev_1").is_ok());
        for (h, p, u) in [
            ("", 22, "dev"),
            ("-oProxyCommand=x", 22, "dev"),
            ("a b", 22, "dev"),
            ("mac", 0, "dev"),
            ("mac", 22, ""),
            ("mac", 22, "-x"),
            ("mac", 22, "a b"),
            ("mac", 22, "a;b"),
        ] {
            assert!(validate_endpoint(h, p, u).is_err(), "{h} {p} {u}");
        }
    }

    #[test]
    fn the_helper_pair_request_is_on_stdin_and_the_command_has_no_secret() {
        let (cmd, input) =
            build_pair_command(SHA, &format!("{} extra", key()), "My iPhone").unwrap();
        assert!(!cmd.contains(BLOB) && !cmd.contains("iPhone"));
        assert!(cmd.contains("pair"));
        let v: serde_json::Value = serde_json::from_str(&input).unwrap();
        assert_eq!(v["key"], key());
        assert_eq!(v["name"], "My iPhone");
        assert!(build_pair_command("zz", &key(), "P").is_err());
        assert!(build_pair_command(SHA, "junk", "P").is_err());
        assert!(build_pair_command(SHA, &key(), "a\nb").is_err());
    }

    #[test]
    fn pair_replies_are_parsed_and_sanitised() {
        let ok = parse_pair_result("{\"ok\":true,\"added\":true,\"tightened\":true}\n");
        assert_eq!((ok.ok, ok.added, ok.tightened), (true, true, true));
        let same = parse_pair_result("{\"ok\":true,\"added\":false,\"tightened\":false}");
        assert_eq!((same.ok, same.added), (true, false));
        let absent = parse_pair_result("HELPER_ABSENT\n");
        assert!(!absent.ok && absent.message.contains("helper"));
        let err =
            parse_pair_result("{\"ok\":false,\"error\":\"~/.ssh is a symlink \\u001b[31m;rm\"}");
        assert!(!err.ok && !err.message.contains('\u{1b}') && !err.message.contains(';'));
        assert_eq!(
            parse_pair_result("garbage").message,
            "No answer from the host"
        );
    }

    const DOCTOR: &str = r#"{"ok":true,"version":"0.1.0","os":"macos","arch":"aarch64",
      "tmux":{"installed":true,"version":"3.4"},"git":{"installed":true,"version":"2.43.0"},
      "python3":{"installed":true},"tailscale":{"installed":true,"ip":"100.92.14.88"},
      "agents":{"claude":{"found":true,"config_dir":"~/.claude","settings_exists":true},
        "antigravity":{"found":false,"hooks_file_exists":true},
        "ollama":{"installed":true,"running":false}},
      "ssh_app":{"allowed_signers":true,"hook_script":true,"helper_dir_private":true}}"#;

    #[test]
    fn a_doctor_reply_becomes_typed_facts() {
        let d = parse_host_doctor(&DOCTOR.replace('\n', "")).unwrap();
        assert_eq!(
            d.tmux,
            DoctorTool {
                installed: true,
                version: "3.4".into()
            }
        );
        assert_eq!(d.tailscale_ip, "100.92.14.88");
        assert!(d.claude_found && d.claude_settings_exists);
        // A file that exists for an agent that is not there is not a fact worth keeping.
        assert!(!d.antigravity_found && !d.antigravity_hooks_file_exists);
        assert!(d.ollama_installed && !d.ollama_running);
        assert!(d.approval_key_trusted && d.hook_script_installed && d.helper_dir_private);
    }

    #[test]
    fn hostile_or_partial_doctor_replies_are_safe() {
        assert!(parse_host_doctor("").is_none());
        assert!(parse_host_doctor("HELPER_ABSENT").is_none());
        assert!(parse_host_doctor("{\"ok\":false}").is_none());
        let d = parse_host_doctor(
            "{\"ok\":true,\"version\":\"1.0\\n; rm -rf\",\"tmux\":{\"installed\":true,\"version\":\"3.4\\u001b[2J\"},\"tailscale\":{\"installed\":true,\"ip\":\"1.2.3.4; id\"}}",
        )
        .unwrap();
        assert_eq!(d.helper_version, "1.0rm-rf");
        assert_eq!(d.tmux.version, "3.42J");
        assert_eq!(d.tailscale_ip, "");
        assert!(!d.python3.installed && !d.claude_found);
        assert_eq!(
            parse_host_doctor("{\"ok\":true}").unwrap(),
            HostDoctor::default()
        );
    }

    #[test]
    fn the_doctor_command_is_pinned_and_read_only() {
        let cmd = build_doctor_command(SHA).unwrap();
        assert!(cmd.contains("doctor") && cmd.contains(SHA));
        assert!(build_doctor_command("nope").is_err());
    }

    #[test]
    fn findings_follow_the_facts() {
        let d = parse_host_doctor(&DOCTOR.replace('\n', "")).unwrap();
        let ids: Vec<_> = findings_for(&d).into_iter().map(|f| f.id).collect();
        assert_eq!(ids, ["hooks-claude", "ollama-stopped"]);

        let empty = HostDoctor::default();
        let ids: Vec<_> = findings_for(&empty).into_iter().map(|f| f.id).collect();
        assert_eq!(
            ids,
            [
                "install-tmux",
                "install-python3",
                "trust-approval-key",
                "no-tailscale",
                "no-git"
            ]
        );

        let mut risky = d;
        risky.helper_dir_private = false;
        assert!(findings_for(&risky)
            .iter()
            .any(|f| f.id == "private-dir" && f.level == FindingLevel::Needed));
        risky.hook_script_installed = false;
        assert!(findings_for(&risky)
            .iter()
            .any(|f| f.id == "install-hook-script"));
    }
}
