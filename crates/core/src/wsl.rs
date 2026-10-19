//! Ready-to-copy setup text for a Windows PC that is used as a host through WSL2 (feature 33, H3).
//! Nothing here runs on the host: the app only shows the snippets and the user pastes them into a
//! Windows PowerShell or into the WSL distro. The only caller-supplied values are a public key and a
//! port, both validated, so no snippet can contain anything but fixed text, digits and a key line.

use crate::error::SshCoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum WslNetworkRoute {
    /// Tailscale installed inside the distro (recommended).
    TailscaleInDistro,
    /// Windows 11 22H2+ mirrored networking.
    MirroredNetworking,
    /// `netsh interface portproxy` (the WSL2 address changes on restart).
    PortProxy,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct WslSnippet {
    pub title: String,
    /// Where to run or save it, e.g. "Windows PowerShell" or "Inside the WSL distro".
    pub where_to_run: String,
    pub body: String,
    pub note: String,
}

fn invalid(reason: &str) -> SshCoreError {
    SshCoreError::InvalidInput {
        reason: reason.to_string(),
    }
}

/// One `ssh-ed25519 <base64> [comment]` line, nothing else.
fn validate_public_key(key: &str) -> Result<String, SshCoreError> {
    let key = key.trim();
    let mut parts = key.split(' ');
    let (kind, blob, comment) = (parts.next(), parts.next(), parts.next());
    let ok = parts.next().is_none()
        && kind == Some("ssh-ed25519")
        && blob.is_some_and(|b| {
            b.len() >= 40
                && b.len() <= 200
                && b.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
        })
        && comment.is_none_or(|c| {
            c.len() <= 64
                && c.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '_' | '-'))
        });
    if ok {
        Ok(key.to_string())
    } else {
        Err(invalid("Not a single ssh-ed25519 public key line"))
    }
}

/// Setup text for the chosen route. `public_key` is the app's key (one `ssh-ed25519` line);
/// `ssh_port` is the port the phone will connect to (22 unless the Windows side already uses it).
#[uniffi::export]
pub fn wsl_setup_snippets(
    route: WslNetworkRoute,
    public_key: String,
    ssh_port: u16,
) -> Result<Vec<WslSnippet>, SshCoreError> {
    let key = validate_public_key(&public_key)?;
    if ssh_port == 0 {
        return Err(invalid("Port must be 1-65535"));
    }
    let mut out = vec![
        WslSnippet {
            title: "Enable systemd in the distro".into(),
            where_to_run: "Inside the WSL distro (then run `wsl --shutdown` in PowerShell)".into(),
            body: "sudo tee /etc/wsl.conf >/dev/null <<'EOF'\n[boot]\nsystemd=true\nEOF".into(),
            note: "Needed so sshd starts with the distro. Restart WSL afterwards.".into(),
        },
        WslSnippet {
            title: "Install sshd and tmux".into(),
            where_to_run: "Inside the WSL distro".into(),
            body: "sudo apt update && sudo apt install -y openssh-server tmux\nsudo systemctl enable --now ssh".into(),
            note: "Debian and Ubuntu. On other distros install the packages openssh and tmux (3.0 or newer) and start the sshd service.".into(),
        },
        WslSnippet {
            title: "Allow this app's key".into(),
            where_to_run: "Inside the WSL distro".into(),
            body: format!(
                "mkdir -p ~/.ssh && chmod 700 ~/.ssh\nprintf '%s\\n' '{key}' >> ~/.ssh/authorized_keys\nchmod 600 ~/.ssh/authorized_keys"
            ),
            note: "Only the public key is shown here; the private key never leaves the phone.".into(),
        },
    ];
    match route {
        WslNetworkRoute::TailscaleInDistro => out.push(WslSnippet {
            title: "Tailscale inside the distro".into(),
            where_to_run: "Inside the WSL distro".into(),
            body: "curl -fsSL https://tailscale.com/install.sh | sh\nsudo tailscale up".into(),
            note: "The distro gets its own tailnet address: connect the phone to that address on port 22. No port forwarding is needed.".into(),
        }),
        WslNetworkRoute::MirroredNetworking => {
            out.push(WslSnippet {
                title: "Mirrored networking".into(),
                where_to_run: "Windows: save as %UserProfile%\\.wslconfig, then run `wsl --shutdown`".into(),
                body: "[wsl2]\nnetworkingMode=mirrored".into(),
                note: "Windows 11 22H2 or newer. The distro then shares the PC's network address.".into(),
            });
            out.push(WslSnippet {
                title: "Open the port in Windows Firewall".into(),
                where_to_run: "Windows PowerShell (as administrator)".into(),
                body: format!(
                    "New-NetFirewallRule -DisplayName \"WSL SSH\" -Direction Inbound -Protocol TCP -LocalPort {ssh_port} -Action Allow"
                ),
                note: "Reach the PC through Tailscale on Windows (or its LAN address) on this port.".into(),
            });
        }
        WslNetworkRoute::PortProxy => out.push(WslSnippet {
            title: "Forward a Windows port to the distro".into(),
            where_to_run: "Windows PowerShell (as administrator)".into(),
            body: format!(
                "$ip = (wsl hostname -I).Trim().Split(' ')[0]\nnetsh interface portproxy add v4tov4 listenport={ssh_port} listenaddress=0.0.0.0 connectport=22 connectaddress=$ip\nNew-NetFirewallRule -DisplayName \"WSL SSH\" -Direction Inbound -Protocol TCP -LocalPort {ssh_port} -Action Allow"
            ),
            note: "The distro's address changes when WSL restarts: run this again after a restart.".into(),
        }),
    }
    out.push(WslSnippet {
        title: "Check".into(),
        where_to_run: "Inside the WSL distro".into(),
        body: "tmux -V\nsystemctl is-active ssh".into(),
        note: "Expect a tmux version of 3.0 or newer and `active`.".into(),
    });
    Ok(out)
}

/// Setup text for a Windows PC used directly (no WSL2): OpenSSH Server with `cmd.exe` as the SSH
/// shell, the app's key in the right file for the account type, Tailscale.
#[uniffi::export]
pub fn windows_setup_snippets(public_key: String) -> Result<Vec<WslSnippet>, SshCoreError> {
    let key = validate_public_key(&public_key)?;
    Ok(vec![
        WslSnippet {
            title: "Install and start OpenSSH Server".into(),
            where_to_run: "Windows PowerShell (as administrator)".into(),
            body: "Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0\nSet-Service -Name sshd -StartupType Automatic\nStart-Service sshd".into(),
            note: "Windows 10 1809 or newer. The firewall rule for port 22 is created by this step.".into(),
        },
        WslSnippet {
            title: "Use cmd.exe as the SSH shell".into(),
            where_to_run: "Windows PowerShell (as administrator)".into(),
            body: "New-ItemProperty -Path \"HKLM:\\SOFTWARE\\OpenSSH\" -Name DefaultShell -Value \"C:\\Windows\\System32\\cmd.exe\" -PropertyType String -Force".into(),
            note: "The app starts its helper through cmd.exe. Your terminal tabs still open PowerShell 7 (or Windows PowerShell).".into(),
        },
        WslSnippet {
            title: "Allow this app's key".into(),
            where_to_run: "Windows PowerShell (as administrator if your account is an administrator)".into(),
            body: format!(
                "$k = '{key}'\n$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)\nif ($admin) {{\n  $f = \"$env:ProgramData\\ssh\\administrators_authorized_keys\"\n  Add-Content -Path $f -Value $k -Encoding ascii\n  icacls $f /inheritance:r /grant \"Administrators:F\" /grant \"SYSTEM:F\"\n}} else {{\n  New-Item -ItemType Directory -Force \"$HOME\\.ssh\" | Out-Null\n  Add-Content -Path \"$HOME\\.ssh\\authorized_keys\" -Value $k -Encoding ascii\n}}"
            ),
            note: "Administrator accounts use a different file on Windows; this picks the right one. Only the public key is shown here.".into(),
        },
        WslSnippet {
            title: "Tailscale".into(),
            where_to_run: "Windows PowerShell".into(),
            body: "winget install Tailscale.Tailscale".into(),
            note: "Sign in, then connect the phone to the PC's tailnet address on port 22.".into(),
        },
        WslSnippet {
            title: "Check".into(),
            where_to_run: "Windows PowerShell".into(),
            body: "Get-Service sshd\ncertutil -v | Select-Object -First 1".into(),
            note: "sshd must be Running. certutil ships with Windows; the app uses it to install and verify its helper.".into(),
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB3k9Yb2Z4n5L0m7X1q8r6tP2uVwQe4yJ3hGfD5sA9cK me@phone";

    #[test]
    fn snippets_contain_the_key_once_and_fixed_text() {
        for r in [
            WslNetworkRoute::TailscaleInDistro,
            WslNetworkRoute::MirroredNetworking,
            WslNetworkRoute::PortProxy,
        ] {
            let s = wsl_setup_snippets(r, KEY.into(), 2222).unwrap();
            assert!(s.len() >= 4);
            let all: String = s.iter().map(|x| x.body.clone()).collect();
            assert_eq!(all.matches(KEY).count(), 1);
            assert!(s[0].body.contains("systemd=true"));
        }
        let p = wsl_setup_snippets(WslNetworkRoute::PortProxy, KEY.into(), 2222).unwrap();
        assert!(p.iter().any(|x| x.body.contains("listenport=2222")));
        let m = wsl_setup_snippets(WslNetworkRoute::MirroredNetworking, KEY.into(), 22).unwrap();
        assert!(m.iter().any(|x| x.body.contains("networkingMode=mirrored")));
    }

    #[test]
    fn windows_snippets_hold_the_key_once_and_reject_bad_keys() {
        let s = windows_setup_snippets(KEY.into()).unwrap();
        let all: String = s.iter().map(|x| x.body.clone()).collect();
        assert_eq!(all.matches(KEY).count(), 1);
        assert!(all.contains("administrators_authorized_keys"));
        assert!(all.contains("DefaultShell"));
        assert!(windows_setup_snippets("ssh-ed25519 AAAA'; calc; '".into()).is_err());
    }

    #[test]
    fn hostile_keys_and_ports_are_refused() {
        for bad in [
            "",
            "ssh-rsa AAAA",
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB3k9Yb2Z4n5L0m7X1q8r6tP2uVwQe4yJ3hGfD5sA9cK me'; rm -rf ~; '",
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB3k9Yb2Z4n5L0m7X1q8r6tP2uVwQe4yJ3hGfD5sA9cK a\nssh-ed25519 BBBB",
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB3k9Yb2Z4n5L0m7X1q8r6tP2uVwQe4yJ3hGfD5sA9cK a b",
            "ssh-ed25519 short x",
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB3k9Yb2Z4n5L0m7X1q8r6tP2uVwQe4yJ3hGfD5sA9$(id) x",
        ] {
            assert!(
                wsl_setup_snippets(WslNetworkRoute::PortProxy, bad.into(), 22).is_err(),
                "{bad:?}"
            );
        }
        assert!(wsl_setup_snippets(WslNetworkRoute::PortProxy, KEY.into(), 0).is_err());
        // no comment is fine
        let nokey = KEY.split(' ').take(2).collect::<Vec<_>>().join(" ");
        assert!(wsl_setup_snippets(WslNetworkRoute::PortProxy, nokey, 22).is_ok());
    }
}
