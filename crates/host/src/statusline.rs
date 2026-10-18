//! Claude Code `statusLine` wrapper: records `rate_limits`, then prints the user's own status line
//! (when one was wrapped) or a short limits line. It never fails the status line.

use crate::proc::run_with_input;
use crate::queue::{clean, now, private_dir, unlink, write_new, Paths};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::os::unix::fs::DirBuilderExt;
use std::process::Command;
use std::time::Duration;

const MAX_INPUT: u64 = 262_144;

fn limit_window(raw: Option<&Value>) -> Option<Value> {
    let pct = raw?.get("used_percentage")?.as_f64()?;
    let used = if pct.fract() == 0.0 {
        json!((pct as i64).clamp(0, 100))
    } else {
        json!(pct.clamp(0.0, 100.0))
    };
    let mut out = json!({ "used_percentage": used });
    if let Some(reset) = raw?.get("resets_at").and_then(Value::as_f64) {
        out["resets_at"] = json!(reset as i64);
    }
    Some(out)
}

fn expand(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => format!("{}/{rest}", std::env::var("HOME").unwrap_or_default()),
        None => path.to_string(),
    }
}

fn record(p: &Paths, windows: &Map<String, Value>) {
    let cfg = std::env::var("CLAUDE_CONFIG_DIR")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "~/.claude".into());
    let cfg = expand(&cfg);
    let cfg = std::fs::canonicalize(&cfg).map_or(cfg, |c| c.to_string_lossy().into_owned());
    if !(private_dir(&p.home) && private_dir(&p.run)) {
        return;
    }
    let mut dirs = std::fs::DirBuilder::new();
    dirs.mode(0o700).recursive(true);
    if dirs.create(&p.limits).is_err() || !private_dir(&p.limits) {
        return;
    }
    let name = &hex::encode(Sha256::digest(cfg.as_bytes()))[..16];
    let tmp = p.limits.join(format!(".{name}.tmp"));
    unlink(&tmp);
    let mut doc = json!({"dir": cfg, "ts": now()});
    for (k, v) in windows {
        doc[k] = v.clone();
    }
    if write_new(&tmp, doc.to_string().as_bytes()).is_err()
        || std::fs::rename(&tmp, p.limits.join(format!("{name}.json"))).is_err()
    {
        unlink(&tmp);
    }
}

pub fn run(p: &Paths, args: &[String]) -> i32 {
    let mut raw = Vec::new();
    let _ = std::io::stdin()
        .lock()
        .take(MAX_INPUT + 1)
        .read_to_end(&mut raw);
    let data = if raw.len() as u64 <= MAX_INPUT {
        serde_json::from_slice::<Value>(&raw)
            .ok()
            .filter(Value::is_object)
    } else {
        None
    }
    .unwrap_or_else(|| json!({}));
    let limits = data.get("rate_limits");
    let mut windows = Map::new();
    for key in ["five_hour", "seven_day"] {
        if let Some(w) = limit_window(limits.and_then(|l| l.get(key))) {
            windows.insert(key.to_string(), w);
        }
    }
    if !windows.is_empty() {
        record(p, &windows);
    }
    let prev = args
        .first()
        .filter(|h| !h.is_empty())
        .and_then(|h| hex::decode(h).ok())
        .and_then(|b| String::from_utf8(b).ok())
        .filter(|s| !s.is_empty());
    let mut stdout = std::io::stdout().lock();
    if let Some(prev) = prev {
        // The user's own previous status-line command, exactly as before the wrapper.
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(prev);
        if let Some((_, out)) = run_with_input(cmd, &raw, Duration::from_secs(5)) {
            let _ = stdout.write_all(&out);
        }
        return 0;
    }
    let mut parts = Vec::new();
    for (key, label) in [("five_hour", "5h"), ("seven_day", "7d")] {
        if let Some(pct) = windows.get(key).and_then(|w| w["used_percentage"].as_f64()) {
            parts.push(format!("{label} {}%", pct.round_ties_even() as i64));
        }
    }
    if let Some(model) = data
        .get("model")
        .and_then(|m| m.get("display_name"))
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
    {
        parts.insert(0, clean(model, 40));
    }
    let _ = stdout.write_all(parts.join(" · ").as_bytes());
    0
}
