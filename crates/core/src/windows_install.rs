//! Installing the helper on a native Windows host (feature 34, W4). OpenSSH Server runs the command
//! through `cmd.exe /c`, a command line is limited to about 8000 characters and there is no stdin
//! channel for exec, so the file travels as hex text appended in chunks, then `certutil` (part of
//! Windows) decodes it and hashes the result. The exe name carries the first 12 characters of its
//! digest, so an update never replaces a file that a running multiplexer server still holds, and the
//! app only ever starts a name whose content it has verified.

use crate::error::SshCoreError;

/// Hex characters per `echo` (the whole command line stays far below the 8191 limit of cmd.exe).
const CHUNK_HEX: usize = 6_000;
const DIR: &str = r".ssh-app\bin";

/// Relative path (from the login directory) of the helper with this digest.
pub(crate) fn helper_path(sha256: &str) -> String {
    format!(
        r"{DIR}\ssh-app-host-{}.exe",
        &sha256[..12.min(sha256.len())]
    )
}

fn check_sha(sha256: &str) -> Result<(), SshCoreError> {
    crate::usage::validate_sha256(sha256)
}

/// `certutil -hashfile` of the installed file (prints an error text when it is missing).
pub(crate) fn build_check_command(sha256: &str) -> Result<String, SshCoreError> {
    check_sha(sha256)?;
    Ok(format!(
        r"certutil -hashfile {} SHA256",
        helper_path(sha256)
    ))
}

/// The digest in `certutil -hashfile` output: a line of 64 hex digits, spaces allowed (older
/// versions print pairs separated by spaces).
pub(crate) fn parse_certutil_hash(out: &str) -> Option<String> {
    out.lines().find_map(|l| {
        let compact: String = l.chars().filter(|c| !c.is_whitespace()).collect();
        (compact.len() == 64 && compact.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| compact.to_ascii_lowercase())
    })
}

/// The commands that put `bytes` in place; the last one prints the digest of the result.
pub(crate) fn build_install_commands(
    bytes: &[u8],
    sha256: &str,
) -> Result<Vec<String>, SshCoreError> {
    check_sha(sha256)?;
    if bytes.is_empty() {
        return Err(SshCoreError::InvalidInput {
            reason: "Empty helper".into(),
        });
    }
    let hex_path = format!(r"{DIR}\upload.hex");
    let exe = helper_path(sha256);
    let mut cmds = vec![format!(r"if not exist {DIR} mkdir {DIR}")];
    let hex = hex::encode(bytes);
    for (i, chunk) in hex.as_bytes().chunks(CHUNK_HEX).enumerate() {
        let chunk = std::str::from_utf8(chunk).unwrap_or_default();
        // The space before the redirection stops cmd from reading a trailing digit as a handle.
        let op = if i == 0 { ">" } else { ">>" };
        cmds.push(format!("echo {chunk} {op} {hex_path}"));
    }
    cmds.push(format!(
        "certutil -decodehex -f {hex_path} {exe} & del {hex_path} & certutil -hashfile {exe} SHA256"
    ));
    Ok(cmds)
}

/// `.\<path>` form used to start the helper (see `multiplexer`).
pub(crate) fn run_path(sha256: &str) -> String {
    format!(r".\{}", helper_path(sha256))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn paths_and_commands_are_fixed_text_and_hex() {
        assert_eq!(
            helper_path(SHA),
            r".ssh-app\bin\ssh-app-host-0123456789ab.exe"
        );
        assert_eq!(
            run_path(SHA),
            r".\.ssh-app\bin\ssh-app-host-0123456789ab.exe"
        );
        let bytes: Vec<u8> = (0..20_000u32).map(|i| (i * 7) as u8).collect();
        let cmds = build_install_commands(&bytes, SHA).unwrap();
        assert!(cmds[0].starts_with("if not exist"));
        assert!(cmds[1].ends_with("> .ssh-app\\bin\\upload.hex"));
        assert!(cmds[2].ends_with(">> .ssh-app\\bin\\upload.hex"));
        assert!(
            cmds.iter().all(|c| c.len() < 7_000),
            "command lines stay short"
        );
        let hex: String = cmds[1..cmds.len() - 1]
            .iter()
            .map(|c| {
                c.strip_prefix("echo ")
                    .unwrap()
                    .split(' ')
                    .next()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(hex, hex::encode(&bytes));
        assert!(cmds.last().unwrap().contains("certutil -decodehex"));
        assert!(build_install_commands(&[], SHA).is_err());
        assert!(build_install_commands(&bytes, "x; del *").is_err());
        assert!(build_check_command("..\\evil").is_err());
    }

    #[test]
    fn certutil_output_is_read_in_both_formats() {
        let new = format!("SHA256 hash of f.exe:\r\n{SHA}\r\nCertUtil: -hashfile command completed successfully.\r\n");
        assert_eq!(parse_certutil_hash(&new).as_deref(), Some(SHA));
        let spaced: String = SHA
            .as_bytes()
            .chunks(2)
            .map(|p| std::str::from_utf8(p).unwrap())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            parse_certutil_hash(&format!("SHA256 hash of f:\r\n{spaced}\r\nCertUtil: ok"))
                .as_deref(),
            Some(SHA)
        );
        assert_eq!(
            parse_certutil_hash("CertUtil: -hashfile command FAILED: 0x80070002\r\n"),
            None
        );
        assert_eq!(parse_certutil_hash(""), None);
        assert_eq!(
            parse_certutil_hash(&SHA.to_uppercase()).as_deref(),
            Some(SHA)
        );
    }
}
