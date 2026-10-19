//! Which kind of host the app is talking to (feature 33, phase H1): macOS or Linux (also under WSL2),
//! on which CPU. The answer picks which of the helper builds embedded in the app is installed and
//! pinned. It is advice, never trusted for security: the pin is always the app's own digest of its
//! own blob, so a host that lies about its system just gets a build that does not match or run.

use crate::error::SshCoreError;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum HostTarget {
    DarwinArm64,
    DarwinX86_64,
    LinuxX86_64,
    LinuxAarch64,
    WslX86_64,
    WslAarch64,
    Unsupported { reason: String },
}

impl HostTarget {
    /// Target triple of the helper build for this host (`None` when unsupported).
    pub fn triple(&self) -> Option<&'static str> {
        match self {
            HostTarget::DarwinArm64 => Some("aarch64-apple-darwin"),
            HostTarget::DarwinX86_64 => Some("x86_64-apple-darwin"),
            HostTarget::LinuxX86_64 | HostTarget::WslX86_64 => Some("x86_64-unknown-linux-musl"),
            HostTarget::LinuxAarch64 | HostTarget::WslAarch64 => Some("aarch64-unknown-linux-musl"),
            HostTarget::Unsupported { .. } => None,
        }
    }
}

/// `uname -sm` first, then a marker line, then `/proc/version` (empty on macOS).
/// The `TARGET ` prefix marks the answer: whatever a login profile prints before it is ignored.
pub(crate) const HOST_TARGET_SCRIPT: &str = "printf 'TARGET '; uname -sm 2>&1 | head -n 1; echo ===PROC===; cat /proc/version 2>/dev/null | head -n 1";

/// Longest piece of host text echoed back in a message.
const MAX_ECHO: usize = 40;

fn echo(s: &str) -> String {
    crate::approval_queue::display_safe(&s.chars().take(MAX_ECHO).collect::<String>())
}

const WINDOWS_HINT: &str =
    "Windows without WSL2: tmux does not run there; set up WSL2 and connect to it";

/// cmd.exe / PowerShell answering a POSIX command line.
fn windows_shell_error(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "not recognized",
        "not recognised",
        "commandnotfound",
        "parsererror",
    ]
    .iter()
    .any(|m| lower.contains(m))
}

fn unsupported(reason: &str) -> HostTarget {
    HostTarget::Unsupported {
        reason: reason.to_string(),
    }
}

/// Decides the target from the probe's output (`HOST_TARGET_SCRIPT`).
#[uniffi::export]
pub fn parse_host_target(probe_output: String) -> HostTarget {
    let (head, proc_version) = probe_output
        .split_once("===PROC===")
        .unwrap_or((probe_output.as_str(), ""));
    // The last marked line wins; anything before it (profile greetings) is noise.
    let Some(uname) = head
        .lines()
        .rev()
        .find_map(|l| l.trim().strip_prefix("TARGET "))
    else {
        return if windows_shell_error(head) {
            unsupported(WINDOWS_HINT)
        } else {
            unsupported("The host did not answer the system probe")
        };
    };
    let uname = uname.trim();
    let lower = uname.to_ascii_lowercase();
    let mut it = uname.split_whitespace();
    let (os, arch) = (it.next().unwrap_or(""), it.next().unwrap_or(""));
    let wsl = proc_version.to_ascii_lowercase().contains("microsoft");
    let arch64 = matches!(arch, "arm64" | "aarch64");
    let x86 = matches!(arch, "x86_64" | "amd64");
    match os {
        "Darwin" if arch64 => HostTarget::DarwinArm64,
        "Darwin" if x86 => HostTarget::DarwinX86_64,
        "Linux" if arch64 => {
            if wsl {
                HostTarget::WslAarch64
            } else {
                HostTarget::LinuxAarch64
            }
        }
        "Linux" if x86 => {
            if wsl {
                HostTarget::WslX86_64
            } else {
                HostTarget::LinuxX86_64
            }
        }
        "Darwin" | "Linux" => unsupported(&format!("Unsupported CPU: {}", echo(arch))),
        _ if lower.starts_with("mingw")
            || lower.starts_with("msys")
            || lower.starts_with("cygwin") =>
        {
            unsupported(WINDOWS_HINT)
        }
        "" => unsupported("The host did not answer the system probe"),
        other => unsupported(&format!("Unsupported system: {}", echo(other))),
    }
}

/// Which embedded build matches the host, `Err` when the system is not supported.
pub(crate) fn require_supported(target: &HostTarget) -> Result<&'static str, SshCoreError> {
    target.triple().ok_or_else(|| SshCoreError::InvalidInput {
        reason: match target {
            HostTarget::Unsupported { reason } => reason.clone(),
            _ => "Unsupported host".to_string(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(uname: &str, proc: &str) -> HostTarget {
        parse_host_target(format!("TARGET {uname}\n===PROC===\n{proc}\n"))
    }

    #[test]
    fn systems_are_told_apart() {
        assert_eq!(p("Darwin arm64", ""), HostTarget::DarwinArm64);
        assert_eq!(p("Darwin x86_64", ""), HostTarget::DarwinX86_64);
        assert_eq!(
            p("Linux x86_64", "Linux version 6.1.0 (gcc)"),
            HostTarget::LinuxX86_64
        );
        assert_eq!(
            p("Linux aarch64", "Linux version 6.1.0"),
            HostTarget::LinuxAarch64
        );
        assert_eq!(
            p(
                "Linux x86_64",
                "Linux version 5.15.153.1-microsoft-standard-WSL2 (root@x)"
            ),
            HostTarget::WslX86_64
        );
        assert_eq!(
            p(
                "Linux aarch64",
                "Linux version 5.15-microsoft-standard-WSL2"
            ),
            HostTarget::WslAarch64
        );
        assert_eq!(p("Linux arm64", ""), HostTarget::LinuxAarch64);
    }

    #[test]
    fn unsupported_systems_say_why() {
        for (u, needle) in [
            ("MINGW64_NT-10.0-22631 x86_64", "WSL2"),
            ("MSYS_NT-10.0 x86_64", "WSL2"),
            ("CYGWIN_NT-10.0 x86_64", "WSL2"),
            ("FreeBSD amd64", "FreeBSD"),
            ("Linux riscv64", "riscv64"),
            ("", "did not answer"),
        ] {
            match p(u, "") {
                HostTarget::Unsupported { reason } => {
                    assert!(reason.contains(needle), "{u}: {reason}")
                }
                other => panic!("{u} -> {other:?}"),
            }
        }
        // cmd.exe / PowerShell answer without the marker.
        for raw in [
            "'printf' is not recognized as an internal or external command,\noperable program or batch file.\n",
            "printf : The term 'printf' is not recognized as the name of a cmdlet\n",
        ] {
            match parse_host_target(raw.to_string()) {
                HostTarget::Unsupported { reason } => assert!(reason.contains("WSL2"), "{raw}"),
                other => panic!("{raw} -> {other:?}"),
            }
        }
        // Hostile text cannot smuggle control characters into the message.
        match p("Evil\u{1b}[31m x", "") {
            HostTarget::Unsupported { reason } => assert!(!reason.contains('\u{1b}')),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn profile_noise_and_long_text_do_not_matter() {
        let t = |s: &str| parse_host_target(s.to_string());
        assert_eq!(
            t("Welcome back!\nTARGET Darwin arm64\n===PROC===\n"),
            HostTarget::DarwinArm64
        );
        assert_eq!(
            t("Last login: Mon\nnvm loaded\nTARGET Linux x86_64\n===PROC===\nLinux version 6.1\n"),
            HostTarget::LinuxX86_64
        );
        // A noise line that imitates the answer loses against the real, later one.
        assert_eq!(
            t("TARGET Darwin arm64 fake\nTARGET Linux aarch64\n===PROC===\n"),
            HostTarget::LinuxAarch64
        );
        // No marker: no guess.
        assert!(matches!(
            t("Darwin arm64\n===PROC===\n"),
            HostTarget::Unsupported { .. }
        ));
        match t(&format!("TARGET {} x\n===PROC===\n", "A".repeat(5_000_000))) {
            HostTarget::Unsupported { reason } => {
                assert!(reason.chars().count() < 100, "{}", reason.len())
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn triples_and_support() {
        assert_eq!(
            HostTarget::WslAarch64.triple(),
            Some("aarch64-unknown-linux-musl")
        );
        assert_eq!(
            HostTarget::DarwinX86_64.triple(),
            Some("x86_64-apple-darwin")
        );
        assert!(require_supported(&unsupported("nope")).is_err());
        assert_eq!(
            require_supported(&HostTarget::LinuxX86_64).unwrap(),
            "x86_64-unknown-linux-musl"
        );
    }
}
