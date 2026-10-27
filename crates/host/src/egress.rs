//! A push when a sandbox's proxy refuses a host (spec 62). The daemon reads the proxy's recent log
//! (`<engine> logs --since 30s ssh-app-sbx-<id>-proxy`) while a phone session is live, and sends the
//! constant `blocked` push once per newly refused host set. The push says nothing about the host:
//! the name came from the agent, and a push payload carries no text about the work. The phone's
//! sandbox screen shows the name and the Allow button.

use crate::agent::Event;
use crate::ctl::sandboxes;
use crate::queue::Paths;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Most refused hosts remembered per sandbox: a flood of names from the agent cannot grow it.
const MAX_SEEN: usize = 100;
const MAX_LOG: u64 = 1 << 20;

/// What fetches a proxy's recent log: the real one runs the engine, tests give canned text.
pub trait LogSource: Send + Sync {
    fn recent(&self, engine: &str, container: &str) -> Option<String>;
}

pub struct EngineLogs;

fn engine_path(engine: &str) -> Option<&'static str> {
    let list: &[&str] = match engine {
        "docker" => &[
            "/usr/local/bin/docker",
            "/opt/homebrew/bin/docker",
            "/Applications/Docker.app/Contents/Resources/bin/docker",
            "/usr/bin/docker",
        ],
        "podman" => &[
            "/opt/homebrew/bin/podman",
            "/usr/local/bin/podman",
            "/usr/bin/podman",
        ],
        _ => return None,
    };
    list.iter()
        .copied()
        .find(|p| std::path::Path::new(p).exists())
}

/// Longest an engine call may run before it is killed: a stuck engine must not hold the watcher.
const ENGINE_SECS: u64 = 10;

/// Reads a pipe on its own thread: keeps the first `MAX_LOG` bytes and drains the rest, so a noisy
/// log can never fill the pipe and block the engine process.
fn drain<R: Read + Send + 'static>(mut r: R) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        let _ = (&mut r).take(MAX_LOG).read_to_end(&mut kept);
        let _ = std::io::copy(&mut r, &mut std::io::sink());
        kept
    })
}

impl LogSource for EngineLogs {
    fn recent(&self, engine: &str, container: &str) -> Option<String> {
        let mut child = Command::new(engine_path(engine)?)
            .args(["logs", "--since", "30s", container])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .ok()?;
        let out = drain(child.stdout.take()?);
        let err = drain(child.stderr.take()?);
        let deadline = Instant::now() + Duration::from_secs(ENGINE_SECS);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
            }
        }
        let mut text = String::from_utf8_lossy(&out.join().ok()?).into_owned();
        text.push_str(&String::from_utf8_lossy(&err.join().ok()?));
        Some(text)
    }
}

/// The engine a sandbox was set up with, from its `engine` file (exactly `docker` or `podman`).
fn engine_of(p: &Paths, id: &str) -> Option<&'static str> {
    let text = std::fs::read_to_string(p.home.join("sandboxes").join(id).join("engine")).ok()?;
    match text.trim() {
        "docker" => Some("docker"),
        "podman" => Some("podman"),
        _ => None,
    }
}

/// Plain DNS names the proxy refused: letters, digits, dots and hyphens only, with a letter in the
/// last label (no IP literal), at most 253 characters.
pub fn refused_hosts(log: &str) -> Vec<String> {
    const KEY: &str = "refused on filtered domain \"";
    let mut out: Vec<String> = Vec::new();
    for line in log.lines() {
        let Some(at) = line.find(KEY) else { continue };
        let rest = &line[at + KEY.len()..];
        let Some(end) = rest.find('"') else { continue };
        let host = rest[..end].to_ascii_lowercase();
        let named = host
            .rsplit('.')
            .next()
            .is_some_and(|tld| tld.bytes().any(|b| b.is_ascii_alphabetic()));
        let plain = !host.is_empty()
            && host.len() <= 253
            && !host.starts_with(['.', '-'])
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
        if named && plain && !out.contains(&host) {
            out.push(host);
        }
    }
    out
}

/// Remembers which refused hosts were already announced, per sandbox.
pub struct Egress {
    seen: HashMap<String, HashSet<String>>,
    /// When each sandbox last caused a push, and when the last pushes of all sandboxes went out.
    last: HashMap<String, Instant>,
    recent: Vec<Instant>,
    gap: Duration,
}

/// The relay allows few pushes an hour per install and approvals share that budget, so an agent
/// inside a sandbox that invents refused names must not be able to use it up: one `blocked` push
/// per sandbox per gap, and a few per hour in all. A suppressed refusal still shows on the phone.
const PUSH_GAP: Duration = Duration::from_secs(300);
const PUSHES_PER_HOUR: usize = 4;

impl Default for Egress {
    fn default() -> Self {
        Egress {
            seen: HashMap::new(),
            last: HashMap::new(),
            recent: Vec::new(),
            gap: PUSH_GAP,
        }
    }
}

impl Egress {
    fn may_push(&mut self, id: &str) -> bool {
        let now = Instant::now();
        self.recent
            .retain(|t| now.duration_since(*t) < Duration::from_secs(3600));
        if self.recent.len() >= PUSHES_PER_HOUR
            || self
                .last
                .get(id)
                .is_some_and(|t| now.duration_since(*t) < self.gap)
        {
            return false;
        }
        self.last.insert(id.to_string(), now);
        self.recent.push(now);
        true
    }

    /// One pass over the live sandboxes. The first pass for a sandbox only records what is already
    /// in its log (an old refusal is not news); later passes return the ids with a new refusal.
    pub fn scan(&mut self, p: &Paths, logs: &dyn LogSource) -> Vec<String> {
        let live = sandboxes(p);
        self.seen.retain(|id, _| live.iter().any(|(l, _)| l == id));
        let mut news = Vec::new();
        for (id, _) in live {
            let Some(engine) = engine_of(p, &id) else {
                continue;
            };
            let Some(log) = logs.recent(engine, &format!("ssh-app-sbx-{id}-proxy")) else {
                continue;
            };
            let first = !self.seen.contains_key(&id);
            let mut fresh = false;
            {
                let seen = self.seen.entry(id.clone()).or_default();
                for h in refused_hosts(&log) {
                    if seen.len() < MAX_SEEN && seen.insert(h) {
                        fresh = true;
                    }
                }
            }
            if fresh && !first && self.may_push(&id) {
                news.push(id);
            }
        }
        news
    }
}

/// Runs a pass on its own thread (a stuck engine must not hold the daemon loop); the daemon takes
/// the ids with a new refusal through [`EgressWatch::take`] and gives each to the sink as a
/// `blocked` event of the pseudo agent `sandbox`.
pub struct EgressWatch {
    state: Arc<Mutex<Egress>>,
    busy: Arc<AtomicBool>,
    news: Arc<Mutex<Vec<String>>>,
}

impl Default for EgressWatch {
    fn default() -> Self {
        EgressWatch {
            state: Arc::new(Mutex::new(Egress::default())),
            busy: Arc::new(AtomicBool::new(false)),
            news: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl EgressWatch {
    pub fn poll(&self, paths: &Arc<Paths>, logs: Arc<dyn LogSource>) {
        if self.busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let (state, busy, news, paths) = (
            self.state.clone(),
            self.busy.clone(),
            self.news.clone(),
            paths.clone(),
        );
        std::thread::spawn(move || {
            let found = state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .scan(&paths, logs.as_ref());
            news.lock().unwrap_or_else(|e| e.into_inner()).extend(found);
            busy.store(false, Ordering::SeqCst);
        });
    }

    /// Events for the refusals found since the last call.
    pub fn take(&self) -> Vec<Event> {
        let ids = std::mem::take(&mut *self.news.lock().unwrap_or_else(|e| e.into_inner()));
        ids.into_iter()
            .map(|id| Event {
                agent: "sandbox".into(),
                tmux: Some(format!("sandbox:{id}")),
                session: id,
                from: "running".into(),
                to: "blocked".into(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct Fixed(Mutex<String>);
    impl LogSource for Fixed {
        fn recent(&self, engine: &str, container: &str) -> Option<String> {
            assert_eq!(engine, "docker");
            assert_eq!(container, "ssh-app-sbx-abc-proxy");
            Some(self.0.lock().unwrap().clone())
        }
    }

    fn line(h: &str) -> String {
        format!("NOTICE Mar 1 Proxying refused on filtered domain \"{h}\"\n")
    }

    fn setup(dir: &std::path::Path) -> Paths {
        let home = dir.join("home");
        let run = home.join("sandboxes/abc/run");
        for d in ["pending", "decisions", "sessions", "limits"] {
            std::fs::create_dir_all(run.join(d)).unwrap();
        }
        for d in [
            &home,
            &home.join("sandboxes"),
            &home.join("sandboxes/abc"),
            &run,
        ] {
            std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        for d in ["pending", "decisions", "sessions", "limits"] {
            std::fs::set_permissions(run.join(d), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        std::fs::write(home.join("sandboxes/abc/engine"), "docker").unwrap();
        Paths {
            pending: home.join("run/pending"),
            decisions: home.join("run/decisions"),
            sessions: home.join("run/sessions"),
            limits: home.join("run/limits"),
            allowed: home.join("allowed_signers"),
            guard: home.join("guard"),
            run: home.join("run"),
            home,
        }
    }

    #[test]
    fn a_noisy_process_cannot_block_on_a_full_pipe() {
        // 8 MB on stdout and on stderr: more than the cap and far more than a pipe holds.
        let mut child = Command::new("sh")
            .args([
                "-c",
                "head -c 8000000 /dev/zero | tr '\\0' a; head -c 8000000 /dev/zero | tr '\\0' b >&2",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let out = drain(child.stdout.take().unwrap());
        let err = drain(child.stderr.take().unwrap());
        assert!(child.wait().unwrap().success());
        assert_eq!(out.join().unwrap().len() as u64, MAX_LOG);
        assert_eq!(err.join().unwrap().len() as u64, MAX_LOG);
    }

    #[test]
    fn only_plain_dns_names_are_taken_from_the_log() {
        let log = [
            line("Evil.Example.com"),
            line("10.0.0.1"),
            line("*.x.com"),
            line("a b.com"),
            line("-bad.com"),
            line("evil.example.com"),
            "garbage \"x\"\n".to_string(),
        ]
        .concat();
        assert_eq!(refused_hosts(&log), vec!["evil.example.com".to_string()]);
    }

    #[test]
    fn a_new_refusal_is_news_once_and_the_first_look_is_silent() {
        let t = tempfile::tempdir().unwrap();
        let p = setup(t.path());
        let logs = Fixed(Mutex::new(line("old.example.com")));
        let mut e = Egress {
            gap: Duration::ZERO,
            ..Egress::default()
        };
        assert!(
            e.scan(&p, &logs).is_empty(),
            "already in the log at the start"
        );
        assert!(e.scan(&p, &logs).is_empty(), "same host again");
        *logs.0.lock().unwrap() = line("old.example.com") + &line("new.example.com");
        assert_eq!(e.scan(&p, &logs), vec!["abc".to_string()]);
        assert!(e.scan(&p, &logs).is_empty(), "announced once");
    }

    #[test]
    fn refusals_cannot_use_up_the_push_budget() {
        let t = tempfile::tempdir().unwrap();
        let p = setup(t.path());
        let logs = Fixed(Mutex::new(String::new()));
        let mut e = Egress::default();
        e.scan(&p, &logs);
        *logs.0.lock().unwrap() = line("a.example.com");
        assert_eq!(e.scan(&p, &logs).len(), 1);
        *logs.0.lock().unwrap() = line("a.example.com") + &line("b.example.com");
        assert!(e.scan(&p, &logs).is_empty(), "same sandbox inside the gap");
        let mut many = Egress {
            gap: Duration::ZERO,
            ..Egress::default()
        };
        many.scan(&p, &logs);
        let sent = (0..20)
            .filter(|i| {
                *logs.0.lock().unwrap() = line(&format!("h{i}.example.com"));
                !many.scan(&p, &logs).is_empty()
            })
            .count();
        assert_eq!(sent, PUSHES_PER_HOUR);
    }

    #[test]
    fn a_flood_of_names_stays_bounded() {
        let t = tempfile::tempdir().unwrap();
        let p = setup(t.path());
        let logs = Fixed(Mutex::new(String::new()));
        let mut e = Egress::default();
        e.scan(&p, &logs);
        *logs.0.lock().unwrap() = (0..500)
            .map(|i| line(&format!("h{i}.example.com")))
            .collect();
        e.scan(&p, &logs);
        assert!(e.seen["abc"].len() <= MAX_SEEN);
    }

    #[test]
    fn an_unknown_engine_file_is_ignored() {
        let t = tempfile::tempdir().unwrap();
        let p = setup(t.path());
        std::fs::write(p.home.join("sandboxes/abc/engine"), "../../bin/sh").unwrap();
        let logs = Fixed(Mutex::new(line("x.example.com")));
        let mut e = Egress::default();
        assert!(e.scan(&p, &logs).is_empty());
        assert!(engine_of(&p, "abc").is_none());
    }
}
