//! Small read-only probes run on the host over an exec channel (binary lookup, Ollama check).
//!
//! A probe may start user-controlled programs (a shell with the user's rc files, `ollama
//! --version`), so every probe runs under a host-side watchdog: if it does not finish in time
//! the command and its descendants are killed. Without it each timed-out check would leave a stuck
//! process behind on the host.

use crate::tmux::shell_quote;

/// SHA-256 of the file `$h` (or `$t`) as a shell expression: `shasum` (macOS, most Linux), else
/// `sha256sum` (coreutils, busybox), else `openssl`; the first token of the output is the digest.
pub(crate) const SHA256_OF_H: &str = "$( { shasum -a 256 || sha256sum || openssl dgst -sha256 -r; } < \"$h\" 2>/dev/null | cut -d' ' -f1)";
pub(crate) const SHA256_OF_T: &str = "$( { shasum -a 256 || sha256sum || openssl dgst -sha256 -r; } < \"$t\" 2>/dev/null | cut -d' ' -f1)";

/// Wraps `command` (a single shell command that writes to stdout) in a POSIX `sh -c` that kills
/// the command and all its descendants after `secs` seconds. All stderr is discarded; merge it
/// into stdout inside `command` if it is wanted.
///
/// The tree is walked with `ps` (children before parents) instead of using job control: an exec
/// channel has no tty, so `set -m` fails under dash and `kill -- -PGID` is not portable. A
/// descendant that already left the tree (`setsid` + double fork) is out of reach by design.
pub(crate) fn watchdog_sh(command: &str, secs: u32) -> String {
    let script = format!(
        "exec 2>/dev/null; \
         kt() {{ for c in $(ps -A -o pid= -o ppid= | awk -v p=\"$1\" '$2==p{{print $1}}'); do kt $c; done; kill -9 $1; }}; \
         {command} & p=$!; \
         ( sleep {secs}; kt $p ) >/dev/null 2>&1 & w=$!; \
         wait $p; kill $w >/dev/null 2>&1; true"
    );
    format!("sh -c {}", shell_quote(&script))
}

/// Wraps `command` so it stops when the SSH channel closes: a watcher reads the channel's stdin,
/// which reaches EOF when the client closes the channel (Stop pressed, connection lost), and then
/// kills the command. Without it a cancelled Ollama request would keep the model generating on the
/// host until it finished. The command's own exit status is passed through.
pub(crate) fn cancellable_sh(command: &str) -> String {
    let script = format!(
        "exec 3<&0; {command} & c=$!; \
         ( cat <&3 >/dev/null; kill $c ) >/dev/null 2>&1 & w=$!; \
         wait $c; r=$?; kill $w >/dev/null 2>&1; exit $r"
    );
    format!("sh -c {}", shell_quote(&script))
}

/// Runs the POSIX `script` through the user's login shell (so `PATH` from `.zprofile` is set;
/// never a hard-coded shell), then through `sh` (the login shell may be fish), under a host-side
/// watchdog. `args` become `$1...` and must already be shell-quoted (or an expression such as
/// `"$HOME"`, which the login shell expands). stderr is merged into stdout.
pub(crate) fn login_sh_script(script: &str, args: &[String], secs: u32) -> String {
    let mut via_sh = format!("exec sh -c {} sh", shell_quote(script));
    for a in args {
        via_sh.push(' ');
        via_sh.push_str(a);
    }
    let inner = format!(
        "\"${{SHELL:-/bin/zsh}}\" -l -c {} </dev/null 2>&1",
        shell_quote(&via_sh)
    );
    watchdog_sh(&inner, secs)
}

#[cfg(test)]
mod sha_tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn the_digest_expression_matches_sha256_on_this_machine() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("blob");
        std::fs::write(&f, b"hello digest").unwrap();
        let want = hex::encode(Sha256::digest(b"hello digest"));
        for (var, expr) in [("h", SHA256_OF_H), ("t", SHA256_OF_T)] {
            let script = format!(
                "{var}={}; printf %s \"{expr}\"",
                shell_quote(&f.to_string_lossy())
            );
            let out = std::process::Command::new("sh")
                .args(["-c", &script])
                .output()
                .unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout), want, "{var}");
        }
        // With no `shasum` on PATH the next tool answers.
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        for tool in ["sh", "cut", "sha256sum", "openssl", "printf"] {
            if let Ok(p) = std::process::Command::new("sh")
                .args(["-c", &format!("command -v {tool}")])
                .output()
            {
                let path = String::from_utf8_lossy(&p.stdout).trim().to_string();
                if path.starts_with('/') {
                    let _ = std::os::unix::fs::symlink(&path, bin.join(tool));
                }
            }
        }
        if bin.join("sha256sum").exists() || bin.join("openssl").exists() {
            let script = format!(
                "h={}; printf %s \"{SHA256_OF_H}\"",
                shell_quote(&f.to_string_lossy())
            );
            let out = std::process::Command::new(bin.join("sh"))
                .env("PATH", &bin)
                .args(["-c", &script])
                .output()
                .unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout), want, "without shasum");
        }
    }
}
