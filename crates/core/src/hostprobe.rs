//! Small read-only probes run on the host over an exec channel (binary lookup, Ollama check).
//!
//! A probe may start user-controlled programs (a shell with the user's rc files, `ollama
//! --version`), so every probe runs under a host-side watchdog: if it does not finish in time
//! the command and its descendants are killed. Without it each timed-out check would leave a stuck
//! process behind on the host.

use crate::tmux::shell_quote;

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
