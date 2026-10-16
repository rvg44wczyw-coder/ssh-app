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
