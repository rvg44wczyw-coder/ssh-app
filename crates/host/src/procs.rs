//! `ssh-app-host kill`: signal a process, with the refusals that keep the phone connected and the
//! host's sessions alive (feature 32, phase T3).
//!
//! Refused: pid <= 1, this helper and every ancestor of it (the SSH session's `sshd` and shell),
//! processes of another account, the tmux server (it would end every session), and a pid whose
//! start time no longer matches the one the app saw in the list (the pid was reused).

use crate::telemetry::proc_info;
use serde_json::{json, Value};
use std::time::Duration;

const MAX_CHAIN: usize = 64;
/// Start times come from `ps` elapsed time (whole seconds, measured at two moments).
const START_TOLERANCE: u64 = 3;

fn ancestors(mut pid: u32) -> Vec<u32> {
    let mut chain = vec![pid];
    while chain.len() < MAX_CHAIN {
        match proc_info(pid) {
            Some((ppid, ..)) if ppid > 1 && !chain.contains(&ppid) => {
                chain.push(ppid);
                pid = ppid;
            }
            _ => break,
        }
    }
    chain
}

pub fn is_tmux_server(comm: &str, ppid: u32) -> bool {
    let base = comm.rsplit('/').next().unwrap_or(comm);
    (base == "tmux" || base.starts_with("tmux:")) && ppid == 1
}

pub fn refusal(
    pid: u32,
    me: u32,
    my_chain: &[u32],
    info: Option<(u32, u32, u64, String)>,
    my_uid: u32,
    started: Option<u64>,
) -> Option<String> {
    if pid <= 1 {
        return Some(format!("Refusing to signal reserved system PID {pid}"));
    }
    if pid == me || my_chain.contains(&pid) {
        return Some(
            "That process carries this SSH connection; stopping it would drop the phone".into(),
        );
    }
    let Some((ppid, uid, start, comm)) = info else {
        return Some("No such process".into());
    };
    if uid != my_uid {
        return Some("That process belongs to another account".into());
    }
    if is_tmux_server(&comm, ppid) {
        return Some("That is the tmux server; stopping it ends every session (use the terminal to do that on purpose)".into());
    }
    if let Some(seen) = started {
        if seen.abs_diff(start) > START_TOLERANCE {
            return Some("The process list is out of date (that pid now belongs to another process); refresh it".into());
        }
    }
    None
}

pub fn kill(pid: u32, force: bool, started: Option<u64>) -> Value {
    // SAFETY: getuid has no preconditions.
    let my_uid = unsafe { libc::getuid() };
    let me = std::process::id();
    let my_chain = ancestors(me);
    if let Some(why) = refusal(pid, me, &my_chain, proc_info(pid), my_uid, started) {
        return json!({"ok": false, "error": why});
    }
    let sig = if force { libc::SIGKILL } else { libc::SIGTERM };
    // SAFETY: plain kill(2) on a pid that passed the checks above.
    if unsafe { libc::kill(pid as i32, sig) } != 0 {
        let e = std::io::Error::last_os_error();
        return json!({"ok": false, "error": format!("kill failed: {e}")});
    }
    std::thread::sleep(Duration::from_millis(200));
    // SAFETY: signal 0 only checks that the process exists.
    let still = unsafe { libc::kill(pid as i32, 0) } == 0;
    json!({"ok": true, "signalled": true, "still_running": still})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(ppid: u32, uid: u32, start: u64, comm: &str) -> Option<(u32, u32, u64, String)> {
        Some((ppid, uid, start, comm.to_string()))
    }

    #[test]
    fn refusals() {
        let chain = [500, 400, 300];
        let r = |pid, i, st| refusal(pid, 500, &chain, i, 501, st);
        assert!(r(0, None, None).unwrap().contains("reserved"));
        assert!(r(1, None, None).unwrap().contains("reserved"));
        assert!(r(500, info(400, 501, 0, "helper"), None)
            .unwrap()
            .contains("SSH connection"));
        assert!(r(300, info(1, 501, 0, "sshd"), None)
            .unwrap()
            .contains("SSH connection"));
        assert!(r(77, None, None).unwrap().contains("No such"));
        assert!(r(77, info(5, 0, 0, "root-thing"), None)
            .unwrap()
            .contains("another account"));
        assert!(r(77, info(1, 501, 0, "/opt/homebrew/bin/tmux"), None)
            .unwrap()
            .contains("tmux server"));
        assert!(r(77, info(1, 501, 0, "tmux: server"), None)
            .unwrap()
            .contains("tmux server"));
        assert!(
            r(77, info(5, 501, 0, "tmux"), None).is_none(),
            "a tmux client is allowed"
        );
        assert!(r(77, info(5, 501, 1000, "node"), Some(1002)).is_none());
        assert!(r(77, info(5, 501, 1000, "node"), Some(900))
            .unwrap()
            .contains("out of date"));
        assert!(r(77, info(5, 501, 1000, "node"), None).is_none());
    }

    #[test]
    fn etime_parsing() {
        use crate::telemetry::etime_secs;
        assert_eq!(etime_secs("00:05"), Some(5));
        assert_eq!(etime_secs("1:02:03"), Some(3723));
        assert_eq!(etime_secs("2-01:00:00"), Some(2 * 86400 + 3600));
        assert_eq!(etime_secs("x"), None);
        assert_eq!(etime_secs("1:2:3:4"), None);
    }

    #[test]
    fn kills_a_spawned_sleep_and_refuses_ancestors_and_stale_pids() {
        let mut child = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .unwrap();
        let pid = child.id();
        let (_, _, start, _) = proc_info(pid).expect("info");
        // Out-of-date start time: refused, still alive.
        let stale = kill(pid, false, Some(start.saturating_sub(100)));
        assert_eq!(stale["ok"], false, "{stale}");
        assert!(child.try_wait().unwrap().is_none());
        // The test process and its parent are ancestors of the helper in a real run; here the
        // helper IS the test process.
        let own = kill(std::process::id(), true, None);
        assert_eq!(own["ok"], false, "{own}");
        assert_eq!(kill(1, true, None)["ok"], false);
        let ok = kill(pid, false, Some(start));
        assert_eq!(ok["ok"], true, "{ok}");
        let _ = child.wait();
        assert_eq!(kill(pid, false, None)["ok"], false, "gone now");
    }
}
