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
}

/// The helper as seen from the SSH login directory of a Windows account: a relative path works in
/// cmd.exe and PowerShell alike and needs no variable expansion or quoting.
const WIN_HELPER: &str = r".\.ssh-app\bin\ssh-app-host.exe";

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
                Ok(format!("{WIN_HELPER} mux attach --spec {spec}"))
            }
        }
    }

    pub fn detach(self, session: &str) -> Result<String, SshCoreError> {
        match self {
            MultiplexerKind::Tmux => tmux::build_tmux_detach_command(session),
            MultiplexerKind::WindowsMux => {
                validate_session_name(session)?;
                Ok(format!("{WIN_HELPER} mux detach --session {session}"))
            }
        }
    }

    pub fn kill(self, session: &str) -> Result<String, SshCoreError> {
        match self {
            MultiplexerKind::Tmux => tmux::build_tmux_kill_session_command(session),
            MultiplexerKind::WindowsMux => {
                validate_session_name(session)?;
                Ok(format!("{WIN_HELPER} mux kill --session {session}"))
            }
        }
    }

    /// Lines `name|windows|attached|created_epoch`, as `parse_tmux_sessions_output` reads them.
    pub fn list_detailed(self) -> String {
        match self {
            MultiplexerKind::Tmux => tmux::build_tmux_list_sessions_detailed_command(),
            MultiplexerKind::WindowsMux => format!("{WIN_HELPER} mux list"),
        }
    }
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
            .strip_prefix(&format!("{WIN_HELPER} mux attach --spec "))
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
            format!("{WIN_HELPER} mux kill --session s1")
        );
        assert!(k.list_detailed().ends_with("mux list"));
    }
}
