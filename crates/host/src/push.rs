//! Push for the host agent (feature 35, phase T2): the phone registers a device token and the
//! relay's address over SSH, the agent's event sink sends one of four constant categories to the
//! relay. No text about the work ever travels: the request has no field for it.
//!
//! The helper has no TLS stack; the request goes through `curl`, whose configuration (URL, bearer
//! secret, body) is written to its stdin so nothing sensitive appears in the process list.

use crate::agent::{Event, EventSink};
use crate::queue::{now, private_dir, Paths};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const MAX_INPUT: u64 = 2048;
/// At most one push per session and category in this many seconds.
const MIN_GAP: i64 = 30;
const MAX_IN_FLIGHT: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    pub token: String,
    pub env: String,
    pub url: String,
    pub secret: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Sent,
    /// The relay says the token is gone: the registration is removed.
    Unregistered,
    Failed,
}

pub trait Sender: Send + Sync {
    /// `sid`: opaque id of the tmux session (see `session_id`), so the phone can open that tab.
    fn send(&self, reg: &Registration, category: &str, sid: Option<&str>) -> Outcome;
}

pub fn push_file(p: &Paths) -> PathBuf {
    p.home.join("push.json")
}

fn hex_len(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `https://host[:port]/push` with a plain host name and nothing else.
fn valid_url(u: &str) -> bool {
    let Some(rest) = u.strip_prefix("https://") else {
        return false;
    };
    let Some(host) = rest.strip_suffix("/push") else {
        return false;
    };
    let (name, port) = match host.split_once(':') {
        Some((n, p)) => (n, Some(p)),
        None => (host, None),
    };
    u.len() <= 200
        && !name.is_empty()
        && name.contains('.')
        && !name.starts_with(['.', '-'])
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && port
            .is_none_or(|p| !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()))
}

pub fn validate(r: &Registration) -> Result<(), &'static str> {
    if !hex_len(&r.token, 64) {
        return Err("bad token");
    }
    if r.env != "production" && r.env != "sandbox" {
        return Err("bad env");
    }
    if !valid_url(&r.url) {
        return Err("bad url");
    }
    let s = &r.secret;
    if !(32..=128).contains(&s.len())
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("bad secret");
    }
    Ok(())
}

fn parse(v: &Value) -> Option<Registration> {
    let t = |k: &str| v[k].as_str().map(str::to_string);
    let r = Registration {
        token: t("token")?.to_ascii_lowercase(),
        env: t("env")?,
        url: t("url")?,
        secret: t("secret")?,
    };
    validate(&r).ok().map(|_| r)
}

/// The stored registration, or `None` when missing, malformed, or not private to this user.
pub fn load(p: &Paths) -> Option<Registration> {
    use std::os::unix::fs::MetadataExt;
    let path = push_file(p);
    let meta = std::fs::symlink_metadata(&path).ok()?;
    // SAFETY: geteuid has no preconditions.
    if !meta.is_file() || meta.mode() & 0o077 != 0 || meta.uid() != unsafe { libc::geteuid() } {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    parse(&serde_json::from_str(&text).ok()?)
}

fn save(p: &Paths, r: &Registration) -> std::io::Result<()> {
    std::fs::create_dir_all(&p.home)?;
    std::fs::set_permissions(&p.home, std::fs::Permissions::from_mode(0o700))?;
    if !private_dir(&p.home) {
        return Err(std::io::Error::other("state folder is not private"));
    }
    let tmp = p.home.join("push.json.tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    let body = json!({"token": r.token, "env": r.env, "url": r.url, "secret": r.secret});
    f.write_all(body.to_string().as_bytes())?;
    f.sync_all()?;
    std::fs::rename(tmp, push_file(p))
}

/// `push-register`: one JSON object on stdin (never argv, the secret is in it).
pub fn register(p: &Paths, input: &mut dyn Read) -> Value {
    let mut text = String::new();
    if input.take(MAX_INPUT + 1).read_to_string(&mut text).is_err() || text.len() as u64 > MAX_INPUT
    {
        return json!({"ok": false, "error": "bad input"});
    }
    let Some(reg) = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| parse(&v))
    else {
        return json!({"ok": false, "error": "bad registration"});
    };
    match save(p, &reg) {
        Ok(()) => json!({"ok": true}),
        Err(_) => json!({"ok": false, "error": "could not save"}),
    }
}

pub fn unregister(p: &Paths) -> Value {
    match std::fs::remove_file(push_file(p)) {
        Ok(()) => json!({"ok": true}),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({"ok": true}),
        Err(_) => json!({"ok": false, "error": "could not remove"}),
    }
}

/// Never includes the secret or the token.
pub fn status(p: &Paths) -> Value {
    match load(p) {
        Some(r) => {
            let host = r
                .url
                .trim_start_matches("https://")
                .trim_end_matches("/push");
            json!({"ok": true, "registered": true, "env": r.env, "relay": host,
                   "token_tail": &r.token[r.token.len() - 6..]})
        }
        None => json!({"ok": true, "registered": false}),
    }
}

/// Escapes a value for a double-quoted curl config string.
fn conf(v: &str) -> String {
    v.replace('\\', "\\\\").replace('"', "\\\"")
}

fn request_body(reg: &Registration, category: &str, sid: Option<&str>) -> String {
    match sid {
        Some(sid) => json!({"token": reg.token, "category": category, "env": reg.env, "sid": sid}),
        None => json!({"token": reg.token, "category": category, "env": reg.env}),
    }
    .to_string()
}

fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(msg);
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

/// Opaque id of a tmux session for the push payload: the first 4 bytes (8 hex) of
/// HMAC-SHA256(relay secret, "sid:" + session name). The phone computes the same from its own
/// copy of the secret and its tab names; Apple and the relay see only 8 hex digits that say
/// nothing about the name without the secret.
pub fn session_id(secret: &str, tmux: &str) -> String {
    hex::encode(&hmac_sha256(secret.as_bytes(), format!("sid:{tmux}").as_bytes())[..4])
}

fn curl_config(reg: &Registration, category: &str, sid: Option<&str>) -> String {
    format!(
        "url = \"{}\"\nrequest = \"POST\"\nheader = \"Authorization: Bearer {}\"\n\
         header = \"Content-Type: application/json\"\ndata = \"{}\"\nmax-time = 10\n\
         proto = \"=https\"\nsilent\noutput = \"/dev/null\"\nwrite-out = \"%{{http_code}}\"\n",
        conf(&reg.url),
        conf(&reg.secret),
        conf(&request_body(reg, category, sid))
    )
}

fn curl_path() -> Option<&'static str> {
    ["/usr/bin/curl", "/bin/curl", "/usr/local/bin/curl"]
        .into_iter()
        .find(|p| std::path::Path::new(p).exists())
}

pub struct CurlSender;

impl Sender for CurlSender {
    fn send(&self, reg: &Registration, category: &str, sid: Option<&str>) -> Outcome {
        let Some(curl) = curl_path() else {
            return Outcome::Failed;
        };
        let Ok(mut child) = Command::new(curl)
            .args(["--config", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        else {
            return Outcome::Failed;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(curl_config(reg, category, sid).as_bytes());
        }
        let Ok(out) = child.wait_with_output() else {
            return Outcome::Failed;
        };
        match String::from_utf8_lossy(&out.stdout).trim() {
            "200" => Outcome::Sent,
            "410" => Outcome::Unregistered,
            _ => Outcome::Failed,
        }
    }
}

/// `push-test`: sends the `alert` category once and reports the outcome.
pub fn test(p: &Paths, sender: &dyn Sender) -> Value {
    let Some(reg) = load(p) else {
        return json!({"ok": false, "error": "not registered"});
    };
    match sender.send(&reg, "alert", None) {
        Outcome::Sent => json!({"ok": true}),
        Outcome::Unregistered => {
            let _ = unregister(p);
            json!({"ok": false, "error": "the relay no longer knows this device"})
        }
        Outcome::Failed => json!({"ok": false, "error": "relay failed"}),
    }
}

struct Inner {
    paths: Paths,
    sender: Arc<dyn Sender>,
    last: Mutex<HashMap<(String, &'static str), i64>>,
    in_flight: AtomicUsize,
}

pub struct PushSink {
    inner: Arc<Inner>,
}

fn category(to: &str) -> Option<&'static str> {
    match to {
        "approval" => Some("approval"),
        "finished" => Some("finished"),
        "waiting" => Some("waiting"),
        _ => None,
    }
}

impl Inner {
    /// Decides and sends synchronously; returns what was sent (for tests).
    fn dispatch(&self, ev: &Event) -> Option<&'static str> {
        let category = category(&ev.to)?;
        let reg = load(&self.paths)?;
        {
            let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
            let key = (format!("{}\0{}", ev.agent, ev.session), category);
            let t = now();
            if last.get(&key).is_some_and(|&p| t - p < MIN_GAP) {
                return None;
            }
            last.insert(key, t);
        }
        let sid = ev.tmux.as_deref().map(|t| session_id(&reg.secret, t));
        if self.sender.send(&reg, category, sid.as_deref()) == Outcome::Unregistered {
            let _ = unregister(&self.paths);
        }
        Some(category)
    }
}

impl PushSink {
    pub fn new(paths: Paths, sender: Arc<dyn Sender>) -> PushSink {
        PushSink {
            inner: Arc::new(Inner {
                paths,
                sender,
                last: Mutex::new(HashMap::new()),
                in_flight: AtomicUsize::new(0),
            }),
        }
    }

    /// Synchronous form of `on_event`, used by tests.
    pub fn dispatch(&self, ev: &Event) -> Option<&'static str> {
        self.inner.dispatch(ev)
    }
}

impl EventSink for PushSink {
    fn on_event(&self, event: &Event) {
        if category(&event.to).is_none() || load(&self.inner.paths).is_none() {
            return;
        }
        if self.inner.in_flight.fetch_add(1, Ordering::SeqCst) >= MAX_IN_FLIGHT {
            self.inner.in_flight.fetch_sub(1, Ordering::SeqCst);
            return;
        }
        // A slow relay must never hold the watcher: the send runs on its own thread.
        let (inner, ev) = (self.inner.clone(), event.clone());
        std::thread::spawn(move || {
            inner.dispatch(&ev);
            inner.in_flight.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12";
    const SECRET: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn reg_json(url: &str) -> String {
        json!({"token": TOKEN, "env": "sandbox", "url": url, "secret": SECRET}).to_string()
    }

    fn paths(dir: &std::path::Path) -> Paths {
        let home = dir.join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
        let run = home.join("run");
        Paths {
            pending: run.join("pending"),
            decisions: run.join("decisions"),
            sessions: run.join("sessions"),
            limits: run.join("limits"),
            allowed: home.join("allowed_signers"),
            guard: home.join("guard"),
            run,
            home,
        }
    }

    #[derive(Default)]
    struct Recorder {
        sent: Mutex<Vec<String>>,
        answer: Mutex<Option<Outcome>>,
    }
    impl Sender for Recorder {
        fn send(&self, _: &Registration, category: &str, sid: Option<&str>) -> Outcome {
            self.sent.lock().unwrap().push(format!(
                "{category}{}",
                sid.map(|s| format!("@{s}")).unwrap_or_default()
            ));
            self.answer.lock().unwrap().unwrap_or(Outcome::Sent)
        }
    }

    fn ev(to: &str, session: &str) -> Event {
        Event {
            agent: "claude".into(),
            session: session.into(),
            from: "running".into(),
            to: to.into(),
            tmux: None,
        }
    }

    #[test]
    fn urls_are_checked() {
        for ok in [
            "https://relay.example.workers.dev/push",
            "https://a.b:8443/push",
        ] {
            assert!(valid_url(ok), "{ok}");
        }
        for bad in [
            "http://a.b/push",
            "https://a.b/other",
            "https://a.b/push?x=1",
            "https://a.b/push\"x",
            "https://user@a.b/push",
            "https://localhost/push",
            "https://a.b:/push",
            "https://-a.b/push",
            "https://a b.c/push",
            "https:///push",
        ] {
            assert!(!valid_url(bad), "{bad}");
        }
    }

    #[test]
    fn registration_is_stored_privately_and_status_hides_secrets() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        let url = "https://relay.example.workers.dev/push";
        let r = register(&p, &mut reg_json(url).as_bytes());
        assert_eq!(r["ok"], true, "{r}");
        let mode = std::fs::metadata(push_file(&p))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        let st = status(&p).to_string();
        assert!(st.contains("\"registered\":true"));
        assert!(!st.contains(SECRET) && !st.contains(TOKEN));
        assert_eq!(unregister(&p)["ok"], true);
        assert_eq!(status(&p)["registered"], false);
    }

    #[test]
    fn bad_registrations_are_refused() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        for body in [
            reg_json("http://relay.example.com/push"),
            json!({"token": "zz", "env": "sandbox", "url": "https://a.b/push", "secret": SECRET}).to_string(),
            json!({"token": TOKEN, "env": "dev", "url": "https://a.b/push", "secret": SECRET}).to_string(),
            json!({"token": TOKEN, "env": "sandbox", "url": "https://a.b/push", "secret": "short"}).to_string(),
            json!({"token": TOKEN, "env": "sandbox", "url": "https://a.b/push", "secret": format!("{SECRET}\n")}).to_string(),
            "not json".into(),
            "x".repeat(3000),
        ] {
            assert_eq!(register(&p, &mut body.as_bytes())["ok"], false, "{body}");
        }
        assert!(!push_file(&p).exists());
    }

    #[test]
    fn a_world_readable_or_symlinked_file_is_not_used() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        register(&p, &mut reg_json("https://a.b/push").as_bytes());
        assert!(load(&p).is_some());
        std::fs::set_permissions(push_file(&p), std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load(&p).is_none());
        std::fs::remove_file(push_file(&p)).unwrap();
        let other = d.path().join("other.json");
        std::fs::write(&other, reg_json("https://a.b/push")).unwrap();
        std::fs::set_permissions(&other, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink(&other, push_file(&p)).unwrap();
        assert!(load(&p).is_none());
    }

    #[test]
    fn only_the_three_categories_are_sent_and_repeats_are_held_back() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        let rec = Arc::new(Recorder::default());
        let sink = PushSink::new(p.clone(), rec.clone());
        assert_eq!(sink.dispatch(&ev("approval", "s1")), None, "not registered");
        register(&p, &mut reg_json("https://a.b/push").as_bytes());
        assert_eq!(sink.dispatch(&ev("approval", "s1")), Some("approval"));
        assert_eq!(
            sink.dispatch(&ev("approval", "s1")),
            None,
            "same category within 30 s"
        );
        assert_eq!(sink.dispatch(&ev("approval", "s2")), Some("approval"));
        assert_eq!(sink.dispatch(&ev("finished", "s1")), Some("finished"));
        assert_eq!(sink.dispatch(&ev("running", "s1")), None);
        assert_eq!(sink.dispatch(&ev("gone", "s1")), None);
        assert_eq!(sink.dispatch(&ev("rm -rf /", "s1")), None);
        assert_eq!(rec.sent.lock().unwrap().len(), 3);
    }

    #[test]
    fn a_gone_device_drops_the_registration() {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        let rec = Arc::new(Recorder::default());
        *rec.answer.lock().unwrap() = Some(Outcome::Unregistered);
        let sink = PushSink::new(p.clone(), rec);
        register(&p, &mut reg_json("https://a.b/push").as_bytes());
        sink.dispatch(&ev("waiting", "s1"));
        assert!(!push_file(&p).exists());
    }

    #[test]
    fn curl_config_has_the_fixed_body_and_escapes_quotes() {
        let reg = parse(&serde_json::from_str(&reg_json("https://a.b/push")).unwrap()).unwrap();
        let c = curl_config(&reg, "approval", None);
        assert!(c.contains(&format!("Authorization: Bearer {SECRET}")));
        assert!(c.contains("proto = \"=https\""));
        assert!(c.contains("\\\"category\\\":\\\"approval\\\""));
        assert_eq!(c.lines().filter(|l| l.starts_with("data")).count(), 1);
        // The only fields in the body are token, category and env.
        let body: Value = serde_json::from_str(&request_body(&reg, "approval", None)).unwrap();
        let mut keys: Vec<_> = body.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["category", "env", "token"]);
    }

    #[test]
    fn hmac_matches_the_rfc_4231_vector_and_the_sid_is_eight_hex() {
        // RFC 4231 test case 2
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            hex::encode(mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        let sid = session_id(SECRET, "agy-ssh-app-1");
        assert_eq!(sid.len(), 8);
        assert!(sid.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(sid, session_id(SECRET, "agy-ssh-app-2"));
        assert_ne!(sid, session_id("other-secret", "agy-ssh-app-1"));
        assert!(!sid.contains("agy"));
    }

    #[test]
    fn the_body_carries_sid_only_when_given() {
        let reg = parse(&serde_json::from_str(&reg_json("https://a.b/push")).unwrap()).unwrap();
        let with: Value =
            serde_json::from_str(&request_body(&reg, "finished", Some("0a1b2c3d"))).unwrap();
        let mut keys: Vec<&str> = with
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort();
        assert_eq!(keys, ["category", "env", "sid", "token"]);
    }
}
