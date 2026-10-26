//! `ctl`: what the app calls over SSH to read and answer the approval queue.

use crate::queue::{
    allowed_signers_trusted, is_hex, keygen_path, now, rand_hex, read_small, unlink, write_new,
    Paths, MAX_FILE, MAX_SIG,
};
use crate::sessions::list_sessions;
use serde_json::{json, Value};
use std::io::Read;
use std::time::{Duration, Instant};

const MAX_LIST: usize = 20;
const MAX_DECISIONS_PER_ID: usize = 5;
const WAIT_STEP: Duration = Duration::from_millis(400);

fn wait_max() -> Duration {
    let secs = std::env::var("SSH_APP_WAIT_MAX")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(20.0);
    Duration::from_secs_f64(secs.clamp(0.1, 20.0))
}

fn pending_id(name: &str) -> Option<&str> {
    name.strip_suffix(".json").filter(|id| is_hex(id, 32))
}

/// Most files read from one sandbox's pending folder: the agent in a container can create as many
/// as it likes, so the reader stops early.
const MAX_SANDBOX_ENTRIES: usize = 200;

/// The queues of the live sandboxes (`<home>/sandboxes/<id>/run`): `(id, paths)`, only folders that
/// are private and belong to us. The agent inside a container writes requests there; it can read
/// `pending`, but `decisions` is mounted read-only into the container.
fn sandboxes(p: &Paths) -> Vec<(String, Paths)> {
    let Ok(rd) = std::fs::read_dir(p.home.join("sandboxes")) else {
        return vec![];
    };
    let mut out: Vec<(String, Paths)> = rd
        .flatten()
        .take(64)
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().into_owned();
            let ok = (3..=32).contains(&id.len())
                && id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
            if !ok {
                return None;
            }
            let home = p.home.join("sandboxes").join(&id);
            let run = home.join("run");
            let sp = Paths {
                pending: run.join("pending"),
                decisions: run.join("decisions"),
                sessions: run.join("sessions"),
                limits: run.join("limits"),
                allowed: p.allowed.clone(),
                guard: p.guard.clone(),
                run,
                home,
            };
            sp.queue_trusted().then_some((id, sp))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// A valid, unexpired pending file as a JSON object, else `None`.
fn load_pending(p: &Paths, name: &str, now: i64) -> Option<Value> {
    load_pending_in(p, name, now, false)
}

/// `sandboxed`: the request comes from a container, whose process ids mean nothing here.
fn load_pending_in(p: &Paths, name: &str, now: i64, sandboxed: bool) -> Option<Value> {
    let id = pending_id(name)?;
    let raw = read_small(&p.pending.join(name), MAX_FILE)?;
    let req: Value = serde_json::from_slice(&raw).ok()?;
    let o = req.as_object()?;
    if o.get("id")?.as_str()? != id || !matches!(o.get("kind")?.as_str()?, "approval" | "notify") {
        return None;
    }
    for k in ["ts", "expires"] {
        o.get(k)?.as_i64()?;
    }
    for k in [
        "agent",
        "event",
        "tool",
        "command",
        "cwd",
        "session",
        "nonce",
        "command_sha256",
    ] {
        o.get(k)?.as_str()?;
    }
    if o["expires"].as_i64()? <= now {
        return None;
    }
    // An approval whose hook is gone (the agent was killed or restarted) asks about nothing.
    if o["kind"] == "approval" && !sandboxed {
        if let Some(pid) = o.get("pid").and_then(Value::as_i64).filter(|p| *p > 1) {
            if !process_alive(pid) {
                unlink(&p.pending.join(name));
                return None;
            }
        }
    }
    Some(req)
}

fn process_alive(pid: i64) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return true;
    };
    // SAFETY: signal 0 only checks that the process exists.
    let r = unsafe { libc::kill(pid, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Every valid pending entry, oldest first (not truncated; the agent derives state from it).
pub fn all_pending(p: &Paths, now: i64) -> Vec<Value> {
    let mut items: Vec<Value> = std::fs::read_dir(&p.pending)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| load_pending(p, &e.file_name().to_string_lossy(), now))
                .collect()
        })
        .unwrap_or_default();
    // Requests from sandboxed agents, marked so the phone shows where they come from.
    for (id, sp) in sandboxes(p) {
        if let Ok(rd) = std::fs::read_dir(&sp.pending) {
            for e in rd.flatten().take(MAX_SANDBOX_ENTRIES) {
                if let Some(mut req) =
                    load_pending_in(&sp, &e.file_name().to_string_lossy(), now, true)
                {
                    req["sandbox"] = json!(id);
                    items.push(req);
                }
            }
        }
    }
    items.sort_by(|a, b| {
        (a["ts"].as_i64(), a["id"].as_str()).cmp(&(b["ts"].as_i64(), b["id"].as_str()))
    });
    items
}

fn pending_items(p: &Paths, now: i64) -> Vec<Value> {
    let mut items = all_pending(p, now);
    items.truncate(MAX_LIST);
    items
}

fn read_stdin_json() -> Option<Value> {
    let mut raw = Vec::new();
    std::io::stdin()
        .lock()
        .take(MAX_FILE as u64 + 1)
        .read_to_end(&mut raw)
        .ok()?;
    if raw.len() > MAX_FILE {
        return None;
    }
    let v: Value = serde_json::from_slice(&raw).ok()?;
    Some(if v.is_object() { v } else { json!({}) })
}

fn count_pending(p: &Paths) -> usize {
    std::fs::read_dir(&p.pending).map_or(0, |rd| {
        rd.flatten()
            .filter(|e| pending_id(&e.file_name().to_string_lossy()).is_some())
            .count()
    })
}

fn ids_key(items: &[Value]) -> String {
    let mut ids: Vec<&str> = items.iter().filter_map(|r| r["id"].as_str()).collect();
    ids.sort_unstable();
    ids.join(",")
}

fn error(message: &str) -> Value {
    json!({"ok": false, "error": message})
}

pub fn run(p: &Paths, args: &[String]) -> Value {
    let sub = args.first().map_or("", String::as_str);
    let t = now();
    if sub == "status" {
        let pending = if p.queue_trusted() {
            count_pending(p)
        } else {
            0
        };
        return json!({
            "ok": true, "impl": "rust", "ssh_keygen": keygen_path().is_some(),
            "allowed_signers": allowed_signers_trusted(p), "pending": pending,
        });
    }
    if !p.queue_trusted() {
        return error("approval queue is missing or not private");
    }
    match sub {
        "sessions" => return json!({"host_time": t, "items": list_sessions(p)}),
        "auto-log" => return json!({"host_time": t, "items": crate::auto::log_items(p)}),
        "trusts" => return json!({"host_time": t, "items": crate::trust::items(p)}),
        "revoke-all" => {
            crate::trust::revoke_all(p);
            return json!({"ok": true});
        }
        "list" => return json!({"host_time": t, "items": pending_items(p, t)}),
        "wait" => {
            // Long poll: answers as soon as the set of pending ids differs from the ids the app
            // already shows (sorted, comma-separated), or after the maximum wait.
            let known = args.get(1).map_or("", String::as_str);
            let deadline = Instant::now() + wait_max();
            loop {
                let t = now();
                let items = pending_items(p, t);
                if ids_key(&items) != known || Instant::now() >= deadline {
                    return json!({"host_time": t, "items": items});
                }
                std::thread::sleep(WAIT_STEP);
            }
        }
        _ => {}
    }
    let Some(body) = read_stdin_json() else {
        return error("invalid request");
    };
    let Some(rid) = body
        .get("id")
        .and_then(Value::as_str)
        .filter(|i| is_hex(i, 32))
    else {
        return error("invalid id");
    };
    if sub == "revoke" {
        crate::trust::revoke(p, rid);
        return json!({"ok": true});
    }
    // The request lives in the host queue or in one sandbox's queue; the decision goes to the same one.
    let name = format!("{rid}.json");
    let (home, req) = match load_pending(p, &name, t) {
        Some(r) => (None, Some(r)),
        None => sandboxes(p)
            .into_iter()
            .find_map(|(_, sp)| load_pending_in(&sp, &name, t, true).map(|r| (Some(sp), Some(r))))
            .unwrap_or((None, None)),
    };
    let q = home.as_ref().unwrap_or(p);
    match sub {
        "ack" => {
            if req.is_some_and(|r| r["kind"] == "notify") {
                unlink(&q.pending.join(&name));
            }
            json!({"ok": true})
        }
        "decide" => decide(q, rid, &body, req.as_ref()),
        _ => error("unknown subcommand"),
    }
}

fn decide(p: &Paths, rid: &str, body: &Value, req: Option<&Value>) -> Value {
    if !req.is_some_and(|r| r["kind"] == "approval") {
        return error("request is gone or expired");
    }
    let approved = body.get("approved").and_then(Value::as_bool);
    let dts = body.get("ts").and_then(Value::as_i64);
    let sig = body
        .get("signature")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.chars().count() <= MAX_SIG);
    let (Some(approved), Some(dts), Some(sig)) = (approved, dts, sig) else {
        return error("invalid decision");
    };
    let prefix = format!("{rid}.");
    let existing = match std::fs::read_dir(&p.decisions) {
        Ok(rd) => rd
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
            .count(),
        Err(_) => return error("could not write the decision"),
    };
    if existing >= MAX_DECISIONS_PER_ID {
        return error("too many decisions for this request");
    }
    let mut data = json!({"v": 1, "id": rid, "approved": approved, "ts": dts, "signature": sig});
    // A trust block is verified (and stored) by the waiting hook, not here.
    if let Some(trust) = body
        .get("trust")
        .filter(|t| approved && t.is_object() && t.to_string().len() <= 12000)
    {
        data["trust"] = trust.clone();
    }
    let final_path = p.decisions.join(format!("{rid}.{}.json", rand_hex(8)));
    let tmp = p.decisions.join(format!(".tmp-{}", rand_hex(8)));
    if write_new(&tmp, data.to_string().as_bytes()).is_err()
        || std::fs::rename(&tmp, &final_path).is_err()
    {
        unlink(&tmp);
        return error("could not write the decision");
    }
    json!({"ok": true})
}

#[cfg(test)]
mod stale_tests {
    use super::process_alive;

    #[test]
    fn a_finished_hook_is_not_alive_and_this_process_is() {
        assert!(process_alive(i64::from(std::process::id())));
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = i64::from(child.id());
        child.wait().unwrap();
        assert!(!process_alive(pid));
    }
}

#[cfg(test)]
mod sandbox_tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::path::{Path, PathBuf};

    fn private(p: &Path) {
        std::fs::create_dir_all(p).unwrap();
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    fn host(root: &Path) -> Paths {
        let home = root.join(".ssh-app");
        let run = home.join("run");
        for d in [&home, &run, &run.join("pending"), &run.join("decisions")] {
            private(d);
        }
        Paths {
            pending: run.join("pending"),
            decisions: run.join("decisions"),
            sessions: run.join("sessions"),
            limits: run.join("limits"),
            allowed: home.join("allowed_signers"),
            guard: home.join("guard.json"),
            run,
            home,
        }
    }

    fn sandbox(p: &Paths, id: &str) -> PathBuf {
        let base = p.home.join("sandboxes").join(id);
        for d in [
            &p.home.join("sandboxes"),
            &base,
            &base.join("run"),
            &base.join("run/pending"),
            &base.join("run/decisions"),
        ] {
            private(d);
        }
        base
    }

    const RID: &str = "0123456789abcdef0123456789abcdef";

    fn request(rid: &str, pid: Option<i64>) -> String {
        let mut v = json!({"id": rid, "kind": "approval", "ts": now(), "expires": now() + 100,
            "agent": "claude", "event": "PermissionRequest", "tool": "Bash", "command": "ls",
            "cwd": "/work/repo", "session": "s", "nonce": "n", "command_sha256": "x"});
        if let Some(pid) = pid {
            v["pid"] = json!(pid);
        }
        v.to_string()
    }

    fn put(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[test]
    fn a_sandbox_request_is_listed_with_its_mark_and_its_container_pid_is_ignored() {
        let d = tempfile::tempdir().unwrap();
        let p = host(d.path());
        let base = sandbox(&p, "demo-1");
        // A pid that cannot exist on the host: inside a container it means nothing.
        put(
            &base.join("run/pending").join(format!("{RID}.json")),
            &request(RID, Some(4_000_000)),
        );
        let items = all_pending(&p, now());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["sandbox"], "demo-1");
    }

    #[test]
    fn a_decision_goes_to_the_sandbox_that_asked_and_never_to_the_host_queue() {
        let d = tempfile::tempdir().unwrap();
        let p = host(d.path());
        let base = sandbox(&p, "demo-1");
        put(
            &base.join("run/pending").join(format!("{RID}.json")),
            &request(RID, None),
        );
        let t = now();
        let (home, req) = sandboxes(&p)
            .into_iter()
            .find_map(|(_, sp)| {
                load_pending_in(&sp, &format!("{RID}.json"), t, true).map(|r| (sp, r))
            })
            .unwrap();
        let body = json!({"id": RID, "approved": true, "ts": t, "signature": "sig"});
        assert_eq!(decide(&home, RID, &body, Some(&req))["ok"], true);
        let written = std::fs::read_dir(base.join("run/decisions"))
            .unwrap()
            .count();
        assert_eq!(written, 1);
        assert_eq!(std::fs::read_dir(&p.decisions).unwrap().count(), 0);
    }

    #[test]
    fn symlinks_fifos_loose_folders_and_floods_inside_a_sandbox_cannot_hurt_the_reader() {
        let d = tempfile::tempdir().unwrap();
        let p = host(d.path());
        let base = sandbox(&p, "demo-1");
        let pending = base.join("run/pending");
        // A symlink to a real request elsewhere is not followed.
        let outside = d.path().join("real.json");
        put(&outside, &request(RID, None));
        symlink(&outside, pending.join(format!("{RID}.json"))).unwrap();
        // A FIFO under a valid name must not block the reader.
        let fifo_id = "fedcba9876543210fedcba9876543210";
        let c = std::ffi::CString::new(pending.join(format!("{fifo_id}.json")).to_str().unwrap())
            .unwrap();
        // SAFETY: mkfifo with a valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        // A flood: far more files than the reader looks at.
        for i in 0..400 {
            put(&pending.join(format!("junk-{i}")), "x");
        }
        let t = Instant::now();
        let items = all_pending(&p, now());
        assert!(items.is_empty(), "{items:?}");
        assert!(t.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn a_sandbox_folder_that_is_not_private_or_has_a_bad_name_is_skipped() {
        let d = tempfile::tempdir().unwrap();
        let p = host(d.path());
        let base = sandbox(&p, "demo-1");
        put(
            &base.join("run/pending").join(format!("{RID}.json")),
            &request(RID, None),
        );
        std::fs::set_permissions(
            base.join("run/pending"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        assert!(
            all_pending(&p, now()).is_empty(),
            "a loosened queue is not trusted"
        );
        let bad = sandbox(&p, "Bad_Name");
        put(
            &bad.join("run/pending").join(format!("{RID}.json")),
            &request(RID, None),
        );
        assert!(sandboxes(&p).is_empty());
    }
}
