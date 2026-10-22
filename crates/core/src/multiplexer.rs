//! Which program keeps a session alive on the host (feature 34, phase W1): tmux on macOS, Linux and
//! WSL2, the helper's own `mux` on native Windows. Both builders return one remote command line for
//! the same four jobs (attach or create, detach, kill, list), so the session code does not care.
//! The tmux side is the existing code, unchanged.

use crate::error::SshCoreError;
use crate::tmux::{self, validate_session_name};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, uniffi::Enum)]
pub enum MultiplexerKind {
    #[default]
    Tmux,
    /// `ssh-app-host.exe mux` on a native Windows host.
    WindowsMux,
    /// Zellij (feature 49, milestone A): attach, detach and kill only. No agent feed, screen
    /// watch or key injection, since those rely on tmux's `capture-pane` and `send-keys`.
    Zellij,
}

/// Which multiplexer a server uses when the host has both.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, uniffi::Enum)]
pub enum MultiplexerPreference {
    /// tmux when it is there, else Zellij.
    #[default]
    Auto,
    PreferZellij,
    PreferTmux,
}

/// What the host has: versions from `multiplexer_probe_command`.
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct MultiplexerProbe {
    pub tmux: Option<String>,
    pub zellij: Option<String>,
}

const PATH_FIX: &str = "export PATH=\"/opt/homebrew/bin:/usr/local/bin:$HOME/.cargo/bin:$PATH\"";

/// One remote command that prints `tmux=<version or empty>` and `zellij=<version or empty>`.
#[uniffi::export]
pub fn multiplexer_probe_command() -> String {
    format!(
        "${{SHELL:-/bin/zsh}} -l -c '{PATH_FIX}; \
         t=$(tmux -V 2>/dev/null | head -n 1 | cut -c1-40); z=$(zellij --version 2>/dev/null | head -n 1 | cut -c1-40); \
         printf \"tmux=%s\\nzellij=%s\\n\" \"$t\" \"$z\"'"
    )
}

#[uniffi::export]
pub fn parse_multiplexer_probe(out: &str) -> MultiplexerProbe {
    let field = |key: &str| {
        out.lines()
            .find_map(|l| l.trim().strip_prefix(key))
            .map(|v| v.chars().filter(|c| !c.is_control()).collect::<String>())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    MultiplexerProbe {
        tmux: field("tmux="),
        zellij: field("zellij="),
    }
}

/// The multiplexer to use on a host: the preference when that program exists, else the other
/// one; tmux when neither was found (the existing fallback to a plain shell then applies).
#[uniffi::export]
pub fn choose_multiplexer(
    preference: MultiplexerPreference,
    probe: MultiplexerProbe,
) -> MultiplexerKind {
    match (probe.tmux.is_some(), probe.zellij.is_some(), preference) {
        (true, true, MultiplexerPreference::PreferZellij) => MultiplexerKind::Zellij,
        (_, true, _) if probe.tmux.is_none() => MultiplexerKind::Zellij,
        _ => MultiplexerKind::Tmux,
    }
}

/// Names Zellij gets: stricter than tmux's, and refused rather than rewritten.
fn validate_zellij_name(name: &str) -> Result<(), SshCoreError> {
    validate_session_name(name)?;
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if ok {
        Ok(())
    } else {
        Err(SshCoreError::InvalidSessionName {
            name: name.to_string(),
            reason: "A Zellij session name may hold letters, digits, '_' and '-' only".to_string(),
        })
    }
}

fn single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The helper as seen from the SSH login directory of a Windows account: a relative path works in
/// cmd.exe and PowerShell alike and needs no variable expansion or quoting. The file name carries
/// the digest of the build this app embeds (see `windows_install`).
fn win_helper() -> String {
    match crate::usage::bundled_host_helper_sha256_for(crate::hosttarget::HostTarget::WindowsX86_64)
    {
        Some(sha) => crate::windows_install::run_path(&sha),
        None => r".\.ssh-app\bin\ssh-app-host.exe".to_string(),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The user-controlled parts of a new session travel as hex of a JSON object, so neither cmd.exe
/// nor PowerShell ever sees them as text to interpret.
fn win_spec(
    session: &str,
    command: Option<&str>,
    start_dir: Option<&str>,
) -> Result<String, SshCoreError> {
    let spec = serde_json::json!({
        "session": session,
        "command": command.map(str::trim).filter(|c| !c.is_empty()),
        "start_dir": start_dir.map(str::trim).filter(|d| !d.is_empty()),
    });
    let text = spec.to_string();
    if text.len() > 4000 {
        return Err(crate::agents::invalid("Session command is too long"));
    }
    Ok(hex(text.as_bytes()))
}

impl MultiplexerKind {
    pub fn attach_or_create(
        self,
        session: &str,
        command: Option<&str>,
        start_dir: Option<&str>,
    ) -> Result<String, SshCoreError> {
        match self {
            MultiplexerKind::Tmux => {
                tmux::build_tmux_attach_or_create_command(session, command, start_dir)
            }
            MultiplexerKind::WindowsMux => {
                validate_session_name(session)?;
                let spec = win_spec(session, command, start_dir)?;
                Ok(format!("{} mux attach --spec {spec}", win_helper()))
            }
            MultiplexerKind::Zellij => zellij_attach(session, command, start_dir),
        }
    }

    pub fn detach(self, session: &str) -> Result<String, SshCoreError> {
        match self {
            MultiplexerKind::Tmux => tmux::build_tmux_detach_command(session),
            MultiplexerKind::WindowsMux => {
                validate_session_name(session)?;
                Ok(format!("{} mux detach --session {session}", win_helper()))
            }
            MultiplexerKind::Zellij => {
                validate_zellij_name(session)?;
                // `zellij action detach` only works from inside a session; there is no way to
                // detach another client from outside, so the app just closes its own channel.
                Err(SshCoreError::Unsupported {
                    reason: "Zellij cannot detach a client from outside the session".to_string(),
                })
            }
        }
    }

    pub fn kill(self, session: &str) -> Result<String, SshCoreError> {
        match self {
            MultiplexerKind::Tmux => tmux::build_tmux_kill_session_command(session),
            MultiplexerKind::WindowsMux => {
                validate_session_name(session)?;
                Ok(format!("{} mux kill --session {session}", win_helper()))
            }
            MultiplexerKind::Zellij => {
                validate_zellij_name(session)?;
                // `delete-session --force` kills a running session and forgets it; older releases
                // lack `--force`, where kill then delete does the same.
                Ok(format!(
                    "${{SHELL:-/bin/zsh}} -l -c '{PATH_FIX}; zellij delete-session --force {session} 2>/dev/null || {{ zellij kill-session {session} 2>/dev/null; zellij delete-session {session} 2>/dev/null; }}; true'"
                ))
            }
        }
    }

    /// Lines `name|windows|attached|created_epoch`, as `parse_tmux_sessions_output` reads them.
    pub fn list_detailed(self) -> String {
        match self {
            MultiplexerKind::Tmux => tmux::build_tmux_list_sessions_detailed_command(),
            MultiplexerKind::WindowsMux => format!("{} mux list", win_helper()),
            // `name [Created 2s ago]` per line (`--no-formatting`); dead sessions that could only
            // be resurrected are left out. Windows, attached flag and creation time are unknown.
            MultiplexerKind::Zellij => format!(
                "${{SHELL:-/bin/zsh}} -l -c '{PATH_FIX}; zellij list-sessions --no-formatting 2>/dev/null | awk \"\\$1 != \\\"\\\" && !/EXITED/ {{ print \\$1 \\\"|1|0|0\\\" }}\" || true'"
            ),
        }
    }

    /// Whether the agent feed, screen watch and key injection can work: they need tmux's
    /// `capture-pane` and `send-keys`. Everything that needs them checks this and shows why it is off.
    pub fn supports_screen_access(self) -> bool {
        matches!(self, MultiplexerKind::Tmux)
    }
}

fn zellij_attach(
    session: &str,
    command: Option<&str>,
    start_dir: Option<&str>,
) -> Result<String, SshCoreError> {
    validate_zellij_name(session)?;
    let dir = tmux::validate_start_dir(start_dir)?;
    let mut run = String::new();
    if let Some(d) = &dir {
        // The folder is made first; the session starts where the client stands.
        run.push_str(&format!("mkdir -p {d} 2>/dev/null; cd {d} || exit 1; "));
    }
    let mut attach = format!("exec zellij attach --create {session}");
    if let Some(cmd) = command.map(str::trim).filter(|c| !c.is_empty()) {
        // Used by Zellij only when it creates the session, like tmux's initial command.
        attach.push_str(&format!(
            " -- ${{SHELL:-/bin/zsh}} -l -c {}",
            single_quote(cmd)
        ));
    }
    run.push_str(&attach);
    let inner = format!(
        "{PATH_FIX}; if command -v zellij >/dev/null 2>&1; then {run}; else printf \"\\033[1;33m[Warning] zellij is not installed on remote host. Falling back to plain login shell.\\033[0m\\r\\n\"; exec ${{SHELL:-/bin/zsh}} -l; fi"
    );
    Ok(format!(
        "exec ${{SHELL:-/bin/zsh}} -l -c {}",
        single_quote(&inner)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tmux_commands_are_the_existing_ones() {
        let k = MultiplexerKind::Tmux;
        assert_eq!(
            k.attach_or_create("s1", Some("claude"), Some("~/w"))
                .unwrap(),
            tmux::build_tmux_attach_or_create_command("s1", Some("claude"), Some("~/w")).unwrap()
        );
        assert_eq!(
            k.kill("s1").unwrap(),
            tmux::build_tmux_kill_session_command("s1").unwrap()
        );
        assert_eq!(
            k.detach("s1").unwrap(),
            tmux::build_tmux_detach_command("s1").unwrap()
        );
        assert_eq!(
            k.list_detailed(),
            tmux::build_tmux_list_sessions_detailed_command()
        );
        assert_eq!(MultiplexerKind::default(), MultiplexerKind::Tmux);
    }

    #[test]
    fn windows_commands_carry_no_user_text_outside_hex_and_validated_names() {
        let k = MultiplexerKind::WindowsMux;
        let nasty = "echo \"hi\" & del /q * | %PATH% ^ $(x) `y` '";
        let c = k
            .attach_or_create("claude-1", Some(nasty), Some("C:\\a b"))
            .unwrap();
        let tail = c
            .strip_prefix(&format!("{} mux attach --spec ", win_helper()))
            .unwrap();
        assert!(tail.chars().all(|ch| ch.is_ascii_hexdigit()), "{c}");
        // The hex decodes back to the exact JSON.
        let bytes: Vec<u8> = (0..tail.len() / 2)
            .map(|i| u8::from_str_radix(&tail[2 * i..2 * i + 2], 16).unwrap())
            .collect();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["command"], nasty);
        assert_eq!(v["session"], "claude-1");
        assert_eq!(v["start_dir"], "C:\\a b");
        for bad in ["a b", "a&b", "-x", "", "a\"b", "a%b"] {
            assert!(k.attach_or_create(bad, None, None).is_err(), "{bad}");
            assert!(k.kill(bad).is_err(), "{bad}");
            assert!(k.detach(bad).is_err(), "{bad}");
        }
        assert!(k
            .attach_or_create("s", Some(&"x".repeat(5000)), None)
            .is_err());
        assert_eq!(
            k.kill("s1").unwrap(),
            format!("{} mux kill --session s1", win_helper())
        );
        assert!(k.list_detailed().ends_with("mux list"));
    }

    #[test]
    fn zellij_names_are_refused_not_rewritten() {
        let k = MultiplexerKind::Zellij;
        for bad in [
            "a.b",
            "a:b",
            "a b",
            "-x",
            "",
            &"x".repeat(65),
            "a/b",
            "a;b",
            "a'b",
        ] {
            assert!(k.attach_or_create(bad, None, None).is_err(), "{bad}");
            assert!(k.kill(bad).is_err(), "{bad}");
        }
        assert!(k.attach_or_create("claude-1_x", None, None).is_ok());
    }

    #[test]
    fn zellij_attach_carries_only_validated_text() {
        let k = MultiplexerKind::Zellij;
        let c = k
            .attach_or_create("s1", Some("claude 'it'; rm -rf ~"), Some("~/w x"))
            .unwrap();
        assert!(c.contains("zellij attach --create s1"), "{c}");
        assert!(c.contains("command -v zellij"), "{c}");
        // the start folder is validated like tmux's; a hostile one is refused
        assert!(k.attach_or_create("s1", None, Some("/a\nb")).is_err());
        assert!(k.attach_or_create("s1", None, Some("relative")).is_err());
    }

    #[test]
    fn zellij_detach_is_a_typed_unsupported_error_and_screen_features_are_off() {
        assert!(matches!(
            MultiplexerKind::Zellij.detach("s1"),
            Err(SshCoreError::Unsupported { .. })
        ));
        assert!(MultiplexerKind::Tmux.supports_screen_access());
        assert!(!MultiplexerKind::Zellij.supports_screen_access());
        assert!(!MultiplexerKind::WindowsMux.supports_screen_access());
    }

    #[test]
    fn the_probe_is_parsed_and_the_choice_follows_the_preference() {
        let both = parse_multiplexer_probe("tmux=tmux 3.5a\nzellij=zellij 0.45.1\n");
        assert_eq!(both.tmux.as_deref(), Some("tmux 3.5a"));
        assert_eq!(both.zellij.as_deref(), Some("zellij 0.45.1"));
        let only_z = parse_multiplexer_probe("tmux=\nzellij=zellij 0.45.1\n");
        let only_t = parse_multiplexer_probe("tmux=tmux 3.5a\nzellij=\n");
        let none = parse_multiplexer_probe("garbage");
        use MultiplexerPreference::*;
        assert_eq!(
            choose_multiplexer(Auto, both.clone()),
            MultiplexerKind::Tmux
        );
        assert_eq!(
            choose_multiplexer(PreferTmux, both.clone()),
            MultiplexerKind::Tmux
        );
        assert_eq!(
            choose_multiplexer(PreferZellij, both),
            MultiplexerKind::Zellij
        );
        assert_eq!(
            choose_multiplexer(Auto, only_z.clone()),
            MultiplexerKind::Zellij
        );
        assert_eq!(
            choose_multiplexer(PreferTmux, only_z),
            MultiplexerKind::Zellij
        );
        assert_eq!(
            choose_multiplexer(PreferZellij, only_t),
            MultiplexerKind::Tmux
        );
        assert_eq!(choose_multiplexer(Auto, none), MultiplexerKind::Tmux);
    }

    #[test]
    fn the_probe_command_runs_and_reports_both_programs() {
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(multiplexer_probe_command())
            .env("SHELL", "/bin/sh")
            .env_remove("TMUX")
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            text.contains("tmux=") && text.contains("zellij="),
            "{text:?}"
        );
        let probe = parse_multiplexer_probe(&text);
        if std::process::Command::new("tmux")
            .arg("-V")
            .output()
            .is_ok()
        {
            assert!(probe.tmux.is_some(), "{text:?}");
        }
    }

    /// Against the real `zellij` when the machine has one: a private socket folder, never the
    /// developer's sessions (`ZELLIJ`, `ZELLIJ_SESSION_NAME` and `TMUX` removed).
    #[test]
    fn zellij_list_and_kill_work_against_a_private_server() {
        use std::process::Command;
        if Command::new("zellij").arg("--version").output().is_err() {
            eprintln!("SKIP: zellij is not installed");
            return;
        }
        // short path: the socket path of a Unix socket is limited to about 100 bytes
        let dir = std::path::PathBuf::from(format!("/tmp/zj-{}", rand::random::<u32>()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .unwrap();
        let run = |cmd: &str| {
            Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .env("SHELL", "/bin/sh")
                .env("ZELLIJ_SOCKET_DIR", &dir)
                .env("XDG_RUNTIME_DIR", &dir)
                .env_remove("ZELLIJ")
                .env_remove("ZELLIJ_SESSION_NAME")
                .env_remove("TMUX")
                .env_remove("TMUX_PANE")
                .output()
                .unwrap()
        };
        let k = MultiplexerKind::Zellij;
        let made = run("export PATH=\"/opt/homebrew/bin:/usr/local/bin:$PATH\"; zellij attach --create-background zt-one");
        assert!(
            made.status.success(),
            "{}",
            String::from_utf8_lossy(&made.stderr)
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut names = Vec::new();
        while std::time::Instant::now() < deadline {
            let out = run(&k.list_detailed());
            names = tmux::parse_tmux_sessions_output(&String::from_utf8_lossy(&out.stdout));
            if !names.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        assert_eq!(names.len(), 1, "{names:?}");
        assert_eq!(names[0].name, "zt-one");
        let killed = run(&k.kill("zt-one").unwrap());
        assert!(killed.status.success());
        let out = run(&k.list_detailed());
        assert!(tmux::parse_tmux_sessions_output(&String::from_utf8_lossy(&out.stdout)).is_empty());
        let _ = run(
            "zellij kill-all-sessions -y 2>/dev/null; zellij delete-all-sessions -y -f 2>/dev/null",
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
