//! Temporary trust (feature 50 §4.6): the phone signs "allow these rules in this project for this
//! agent session until T" together with an approval; the hook verifies the signature, stores the
//! signed blob and re-verifies it on every use. A file written by hand gives nothing.

use crate::auto::{
    bash_stays_inside, edit_target_ok, inside, tool_paths, word_stays_inside, AutoDecision,
};
use crate::guard::{first_str, guard_allows, realpath, tool_kind};
use crate::queue::{
    clean, is_hex, now, private_dir, rand_hex, read_small, ssh_verify, unlink, write_new, Paths,
    ANTIGRAVITY_CMD_KEYS, MAX_SIG,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;

const TRUST_MAX: usize = 50;
const HARD_CAP: i64 = 86_400;

pub struct Trust {
    pub id: String,
    nonce: String,
    ts: i64,
    expires: i64,
    agent: String,
    session: String,
    root: String,
    edits: bool,
    rules: Vec<String>,
    signature: String,
}

fn dir(p: &Paths) -> PathBuf {
    p.run.join("trust")
}

fn cap(guard: &Value) -> i64 {
    guard
        .get("trust_max_secs")
        .and_then(Value::as_i64)
        .filter(|n| (60..=HARD_CAP).contains(n))
        .unwrap_or(HARD_CAP)
}

/// `SSH_APP_TRUST_V1:` and the fields, each as `<utf8 length>:<bytes>\n`.
fn message(t: &Trust) -> Vec<u8> {
    let rules_hash = hex::encode(Sha256::digest(t.rules.join("\n").as_bytes()));
    let fields = [
        t.id.as_str(),
        t.nonce.as_str(),
        &t.ts.to_string(),
        &t.expires.to_string(),
        t.agent.as_str(),
        t.session.as_str(),
        t.root.as_str(),
        if t.edits { "true" } else { "false" },
        &rules_hash,
    ];
    let mut out = b"SSH_APP_TRUST_V1:".to_vec();
    for f in fields {
        out.extend_from_slice(format!("{}:", f.len()).as_bytes());
        out.extend_from_slice(f.as_bytes());
        out.push(b'\n');
    }
    out
}

fn rules_ok(rules: &[String]) -> bool {
    if rules.is_empty() || rules.len() > 60 {
        return false;
    }
    rules.iter().all(|r| {
        let r = r.trim();
        if r.is_empty() || r.chars().count() > 400 {
            return false;
        }
        let (name, spec) = match r.split_once('(') {
            Some((n, rest)) => (n, rest.strip_suffix(')').unwrap_or(rest)),
            None => (r, ""),
        };
        let name_ok = !name.is_empty()
            && name
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        name_ok && !(tool_kind(name) == "Bash" && spec.is_empty())
    })
}

fn from_value(v: &Value) -> Option<Trust> {
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let rules: Vec<String> = v
        .get("rules")?
        .as_array()?
        .iter()
        .map(|x| x.as_str().map(str::to_string))
        .collect::<Option<_>>()?;
    let t = Trust {
        id: s("id")?,
        nonce: s("nonce")?,
        ts: v.get("ts")?.as_i64()?,
        expires: v.get("expires")?.as_i64()?,
        agent: s("agent")?,
        session: s("session")?,
        root: s("root")?,
        edits: v.get("edits")?.as_bool()?,
        rules,
        signature: s("signature")?,
    };
    (is_hex(&t.id, 32) && t.root.starts_with('/') && !t.session.is_empty() && rules_ok(&t.rules))
        .then_some(t)
}

fn blob(t: &Trust) -> Value {
    json!({"v": 1, "id": t.id, "nonce": t.nonce, "ts": t.ts, "expires": t.expires,
        "agent": t.agent, "session": t.session, "root": t.root, "edits": t.edits,
        "rules": t.rules, "signature": t.signature})
}

fn read_blob(path: &std::path::Path) -> Option<Trust> {
    let raw = read_small(path, 20000)?;
    from_value(&serde_json::from_slice::<Value>(&raw).ok()?)
}

fn valid(p: &Paths, guard: &Value, t: &Trust) -> bool {
    let n = now();
    t.ts < t.expires
        && n < t.expires
        && t.expires - t.ts <= cap(guard)
        && t.signature.chars().count() <= MAX_SIG
        && ssh_verify(p, &message(t), &t.signature)
}

/// Deletes expired and unreadable files; returns the rest.
fn sweep(p: &Paths) -> Vec<Trust> {
    let Ok(rd) = std::fs::read_dir(dir(p)) else {
        return vec![];
    };
    let n = now();
    let mut out = vec![];
    for entry in rd.flatten().take(TRUST_MAX * 2) {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".json") || name.starts_with('.') {
            continue;
        }
        match read_blob(&entry.path()) {
            Some(t) if t.expires > n && name == format!("{}.json", t.id) => out.push(t),
            _ => unlink(&entry.path()),
        }
    }
    out
}

/// Called with the decision of a verified approval: stores the trust block if its own signature
/// holds. Bound to the request the phone saw (id, nonce, agent, session).
pub fn store_trust(p: &Paths, req: &Value, t: &Value, dts: i64) {
    let guard = crate::guard::load_guard(p);
    let (Some(agent), Some(session)) = (req["agent"].as_str(), req["session"].as_str()) else {
        return;
    };
    if session.is_empty()
        || t.get("agent").and_then(Value::as_str) != Some(agent)
        || t.get("session").and_then(Value::as_str) != Some(session)
    {
        return;
    }
    let (Some(id), Some(nonce)) = (req["id"].as_str(), req["nonce"].as_str()) else {
        return;
    };
    let mut v = t.clone();
    v["id"] = json!(id);
    v["nonce"] = json!(nonce);
    v["ts"] = json!(dts);
    let Some(trust) = from_value(&v) else {
        return;
    };
    if !valid(p, &guard, &trust) {
        return;
    }
    let d = dir(p);
    let _ = std::fs::DirBuilder::new().mode(0o700).create(&d);
    if !private_dir(&d) {
        return;
    }
    if sweep(p).len() >= TRUST_MAX {
        return;
    }
    let tmp = d.join(format!(".tmp-{}", rand_hex(8)));
    if write_new(&tmp, blob(&trust).to_string().as_bytes()).is_err()
        || std::fs::rename(&tmp, d.join(format!("{}.json", trust.id))).is_err()
    {
        unlink(&tmp);
    }
}

/// A trust ends with the agent session it was given for.
pub fn drop_for_session(p: &Paths, agent: &str, session: &str) {
    for t in sweep(p) {
        if t.agent == agent && t.session == session {
            unlink(&dir(p).join(format!("{}.json", t.id)));
        }
    }
}

pub fn revoke(p: &Paths, id: &str) {
    unlink(&dir(p).join(format!("{id}.json")));
}

pub fn revoke_all(p: &Paths) {
    for t in sweep(p) {
        revoke(p, &t.id);
    }
}

/// `ctl trusts`: the valid trusts, soonest to expire first.
pub fn items(p: &Paths) -> Vec<Value> {
    let guard = crate::guard::load_guard(p);
    let mut ts: Vec<Trust> = sweep(p)
        .into_iter()
        .filter(|t| valid(p, &guard, t))
        .collect();
    ts.sort_by_key(|t| t.expires);
    ts.iter()
        .take(TRUST_MAX)
        .map(|t| {
            json!({"id": t.id, "agent": clean(&t.agent, 40), "session": clean(&t.session, 80),
                "root": clean(&t.root, 300), "expires": t.expires, "edits": t.edits,
                "rules": t.rules.iter().take(12).map(|r| clean(r, 120)).collect::<Vec<_>>()})
        })
        .collect()
}

fn project_listed(guard: &Value, root: &str) -> bool {
    let want = realpath(root);
    guard
        .get("auto")
        .and_then(|a| a.get("projects"))
        .and_then(Value::as_array)
        .is_some_and(|ps| {
            ps.iter().take(60).any(|x| {
                x.get("root")
                    .and_then(Value::as_str)
                    .is_some_and(|r| realpath(r) == want)
            })
        })
}

/// Whether a signed, unexpired trust of this agent session covers the call.
pub fn decision(
    p: &Paths,
    guard: &Value,
    agent: &str,
    session: &str,
    tool: &str,
    input: &Value,
    cwd: &str,
) -> Option<AutoDecision> {
    if session.is_empty() || cwd.is_empty() || !private_dir(&dir(p)) {
        return None;
    }
    let here = realpath(cwd);
    let kind = tool_kind(tool);
    for t in sweep(p) {
        if t.agent != agent || t.session != session {
            continue;
        }
        let root = realpath(&t.root);
        if !inside(&root, &here) || !project_listed(guard, &t.root) {
            continue;
        }
        let (ok, hit) = if kind == "Edit" {
            (
                t.edits && edit_target_ok(&root, cwd, &tool_paths(input)),
                "project file edits".to_string(),
            )
        } else if crate::auto::touches_sensitive(&crate::auto::call_text(kind, input)) {
            continue;
        } else {
            let hit = guard_allows(&t.rules, tool, input, cwd);
            let stays = if kind == "Bash" {
                let keys: Vec<&str> = std::iter::once("command")
                    .chain(ANTIGRAVITY_CMD_KEYS[..4].iter().copied())
                    .collect();
                let empty = serde_json::Map::new();
                bash_stays_inside(
                    &root,
                    cwd,
                    first_str(input.as_object().unwrap_or(&empty), &keys),
                )
            } else {
                tool_paths(input)
                    .iter()
                    .all(|x| word_stays_inside(&root, cwd, x))
            };
            (hit.is_some() && stays, hit.unwrap_or_default())
        };
        if ok && valid(p, guard, &t) {
            return Some(AutoDecision::Allow(
                format!("trusted {}: {hit}", &t.id[..8]),
                Some(root),
                true,
            ));
        }
    }
    None
}
