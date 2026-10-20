//! The Rust readers must answer exactly like `usage_probe.py` (the reference) on one fixture tree.
//! No tmux is involved: the process/pane part is covered by unit tests here and by the loopback
//! test of the core (private tmux socket).

use serde_json::{json, Value};
use ssh_app_host::usage::Probe;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn probe_py() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/src/usage_probe.py")
}

fn iso(epoch: f64) -> String {
    // Civil-from-days (Howard Hinnant), formatted like the agents do.
    let secs = epoch.floor() as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.250Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn lines(items: &[Value]) -> String {
    items.iter().map(|v| v.to_string() + "\n").collect()
}

#[test]
fn rust_readers_match_the_python_probe_on_one_fixture_tree() {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let root = tempfile::tempdir().unwrap();
    let t = root.path().canonicalize().unwrap();
    let home = t.join("home");
    let cfg = t.join("cfg");
    let work = t.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let slug: String = work
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();

    // Claude transcripts: context, limit notice with a reset stamp, a sidechain, duplicates.
    let usage = |i, o, cw, cr| json!({"input_tokens": i, "output_tokens": o, "cache_creation_input_tokens": cw, "cache_read_input_tokens": cr});
    let msg = |id: &str, u: Value| json!({"id": id, "model": "claude-x", "usage": u, "content": [{"type": "text", "text": "ok"}]});
    write(
        &cfg.join("projects").join(&slug).join("a.jsonl"),
        &lines(&[
            json!({"type": "assistant", "timestamp": iso(now - 7200.0), "requestId": "r1", "message": msg("m1", usage(10, 20, 30, 40))}),
            json!({"type": "assistant", "timestamp": iso(now - 7100.0), "requestId": "r1", "message": msg("m1", usage(10, 20, 30, 40))}),
            json!({"type": "assistant", "isSidechain": true, "timestamp": iso(now - 600.0), "requestId": "r2", "message": msg("m2", usage(1, 1, 1, 1))}),
            json!({"type": "assistant", "timestamp": iso(now - 300.0), "requestId": "r3", "message": msg("m3", usage(5, 6, 7, 250_000))}),
            json!({"type": "assistant", "isApiErrorMessage": true, "timestamp": iso(now - 120.0), "message": {"content": [{"type": "text", "text": "Claude AI usage limit reached|1900000000"}]}}),
            json!({"type": "user", "message": {"content": "hello"}}),
        ]),
    );
    write(
        &cfg.join("projects").join("-elsewhere").join("b.jsonl"),
        &lines(&[
            json!({"type": "assistant", "timestamp": iso(now - 86_400.0 * 3.0), "requestId": "r9", "message": msg("m9", usage(100, 200, 300, 400))}),
            json!({"type": "assistant", "timestamp": iso(now - 86_400.0 * 9.0), "requestId": "r10", "message": msg("m10", usage(9, 9, 9, 9))}),
        ]),
    );
    write(
        &cfg.join("projects").join("-elsewhere").join("broken.jsonl"),
        "{not json\n\"usage\" \"assistant\"\n",
    );

    // Limits recorded by the status-line wrapper (one valid, one rolled over).
    write(
        &home.join(".ssh-app/run/limits/x.json"),
        &json!({"dir": cfg.to_string_lossy(), "ts": now as i64, "five_hour": {"used_percentage": 42.4, "resets_at": now as i64 + 3600}, "seven_day": {"used_percentage": 18, "resets_at": now as i64 - 5}}).to_string(),
    );

    // Codex rollout.
    let codex = t.join("codex");
    write(
        &codex.join("sessions/2026/10/18/rollout-1.jsonl"),
        &lines(&[
            json!({"timestamp": iso(now - 30.0), "type": "event_msg", "payload": {"type": "token_count", "rate_limits": {
            "primary": {"used_percent": 12.5, "window_minutes": 300, "resets_at": now as i64 + 900},
            "secondary": {"used_percent": 70, "window_minutes": 10080, "resets_in_seconds": 99_999}}}}),
        ]),
    );

    // Antigravity.
    let agy = home.join(".gemini/antigravity-cli");
    let conv = "9fcd4e98-4e17-453c-99a1-4600828e6d64";
    write(
        &agy.join("cache/last_conversations.json"),
        &json!({work.to_string_lossy(): conv}).to_string(),
    );
    write(
        &agy.join("settings.json"),
        &json!({"model": "Gemini 3.8 Flash (Medium)"}).to_string(),
    );
    write(
        &agy.join(format!(
            "brain/{conv}/.system_generated/logs/transcript.jsonl"
        )),
        &lines(&[
            json!({"type": "PLANNER_RESPONSE", "input_tokens": 3000, "cache_read_tokens": 100_000, "output_tokens": 300, "created_at": iso(now - 600.0)}),
            json!({"status": "ERROR", "created_at": iso(now - 100.0), "content": "RESOURCE_EXHAUSTED: individual quota reached"}),
        ]),
    );

    std::env::set_var("HOME", &home);
    std::env::set_var("SSH_APP_HOME", home.join(".ssh-app"));
    std::env::set_var("CODEX_HOME", &codex);
    let probe = Probe::new(vec![cfg.clone()], now);
    let rust = json!({
        "claude": probe.claude_state(work.to_str().unwrap(), None),
        "codex": probe.codex_limits(),
        "agy": probe.agy_state(work.to_str().unwrap(), None),
        "totals": {"five": probe.window_totals().0, "week": probe.window_totals().1, "partial": probe.window_totals().2},
    });

    let prefix = "import sys\nsrc = open(sys.argv[1]).read().split('\\nsessions = []')[0]\nsys.argv = ['x'] + sys.argv[2:]\ng = {}\nexec(src, g)\nimport json\nfive, week, partial = g['window_totals']()\nprint(json.dumps({'claude': g['claude_state'](CWD), 'codex': g['codex_limits'](), 'agy': g['agy_state'](CWD), 'totals': {'five': five, 'week': week, 'partial': partial}}))\n";
    let code = prefix.replace("CWD", &format!("{:?}", work.to_str().unwrap()));
    let out = Command::new("python3")
        .args(["-I", "-c", &code])
        .arg(probe_py())
        .arg(&cfg)
        .env("HOME", &home)
        .env("SSH_APP_HOME", home.join(".ssh-app"))
        .env("CODEX_HOME", &codex)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let py: Value = serde_json::from_slice(&out.stdout).unwrap();

    for k in ["claude", "codex", "agy", "totals"] {
        assert_eq!(rust[k], py[k], "{k}\nrust: {}\npython: {}", rust[k], py[k]);
    }
    // Spot checks so a parity of two empty answers cannot pass.
    assert_eq!(rust["claude"]["context_used"], json!(5 + 6 + 7 + 250_000));
    assert_eq!(rust["claude"]["context_limit"], json!(1_000_000));
    assert_eq!(rust["claude"]["limit_reset_epoch"], json!(1_900_000_000u64));
    assert_eq!(rust["claude"]["limits"].as_array().unwrap().len(), 1);
    assert_eq!(rust["codex"].as_array().unwrap().len(), 2);
    assert_eq!(rust["agy"]["context_used"], json!(103_300));
    assert_eq!(
        rust["totals"]["week"][4],
        json!(4),
        "m1 once, m2 (sidechain counts), m3, m9"
    );
}
