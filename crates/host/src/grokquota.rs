//! Grok plan limit. Grok shows it only in the `/usage` dialog ("Weekly limit (SuperGrok Lite)", a bar with
//! the percent used, "Resets: November 2, 11:50"), so the host reads that screen the way `agyquota` reads
//! agy's: a private grok in its own tmux server, cached for the usage report.

use crate::agyquota::{
    age, folder_runs_project_code, now, probe_disabled, run_dir, tmux, wait_for,
};
use regex::Regex;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const FRESH_SECS: f64 = 900.0;
const STALE_SECS: f64 = 86_400.0;
const LOCK_SECS: f64 = 120.0;

/// One window: minutes, percent used, reset time (epoch seconds).
pub type Limit = (u32, f64, Option<i64>);

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// Epoch seconds of the next local `month day hh:mm` after `at` (the dialog prints no year).
fn next_local(month: usize, day: i32, hour: i32, min: i32, at: f64) -> Option<i64> {
    // SAFETY: `tm` is plain data; `localtime_r` and `mktime` only read and write the structs we pass.
    unsafe {
        let t = at as libc::time_t;
        let mut now_tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut now_tm).is_null() {
            return None;
        }
        for year in [now_tm.tm_year, now_tm.tm_year + 1] {
            let mut tm: libc::tm = std::mem::zeroed();
            tm.tm_year = year;
            tm.tm_mon = i32::try_from(month).ok()?;
            tm.tm_mday = day;
            tm.tm_hour = hour;
            tm.tm_min = min;
            tm.tm_isdst = -1;
            let e = libc::mktime(&mut tm);
            if e != -1 && (e as f64) > at {
                return Some(e as i64);
            }
        }
    }
    None
}

fn minutes_of(title: &str) -> Option<u32> {
    let t = title.to_lowercase();
    if !t.contains("limit") {
        None
    } else if t.contains("week") {
        Some(10_080)
    } else if t.contains("day") || t.contains("daily") {
        Some(1_440)
    } else if t.contains("hour") {
        Some(300)
    } else if t.contains("month") {
        Some(43_200)
    } else {
        None
    }
}

/// The limits of the `/usage` dialog text `text`; `at` is the capture time.
pub fn parse(text: &str, at: f64) -> Vec<Limit> {
    let (Ok(pct_rx), Ok(reset_rx)) = (
        Regex::new(r"(\d{1,3}(?:\.\d+)?)\s*%"),
        Regex::new(r"Resets:\s*([A-Za-z]+)\s+(\d{1,2}),\s*(\d{1,2}):(\d{2})"),
    ) else {
        return vec![];
    };
    let mut out: Vec<Limit> = vec![];
    let mut pending: Option<u32> = None;
    for raw in text.lines() {
        let line = raw.trim_matches(|c: char| c.is_whitespace() || "│┃".contains(c));
        if line.is_empty() {
            continue;
        }
        if let Some(m) = minutes_of(line).filter(|_| !pct_rx.is_match(line)) {
            pending = Some(m);
            continue;
        }
        let Some(m) = pending else { continue };
        if let Some(p) = pct_rx.captures(line).and_then(|c| c[1].parse::<f64>().ok()) {
            if (0.0..=100.0).contains(&p) {
                out.push((m, p, None));
            }
        } else if let Some(c) = reset_rx.captures(line) {
            let month = MONTHS
                .iter()
                .position(|n| n.starts_with(&c[1].to_lowercase()) && c[1].len() >= 3);
            let nums: Vec<i32> = (2..=4).filter_map(|i| c[i].parse().ok()).collect();
            if let (Some(month), [d, h, mi]) = (month, nums.as_slice()) {
                if let Some(last) = out.last_mut().filter(|l| l.0 == m) {
                    last.2 = next_local(month, *d, *h, *mi, at);
                }
            }
            pending = None;
        }
    }
    out
}

fn cache_file() -> PathBuf {
    run_dir().join("grok-quota.json")
}

fn lock_file() -> PathBuf {
    run_dir().join("grok-quota.lock")
}

/// The cached limits that have not yet reset (empty when there is no cache or it is a day old).
pub fn cached(at: f64) -> Vec<Limit> {
    let file = cache_file();
    if age(&file).is_none_or(|a| a > STALE_SECS) {
        return vec![];
    }
    let Some(v) = std::fs::read_to_string(&file)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    else {
        return vec![];
    };
    v["limits"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|l| {
                    Some((
                        u32::try_from(l["minutes"].as_u64()?).ok()?,
                        l["used"].as_f64()?,
                        l["resets_epoch"].as_i64(),
                    ))
                })
                .filter(|l| l.2.is_none_or(|r| (r as f64) > at))
                .collect()
        })
        .unwrap_or_default()
}

/// Starts a detached refresh when the cache is older than 15 minutes; called only while a grok tab exists.
pub fn refresh_if_stale(cwd: &str) {
    if probe_disabled()
        || folder_runs_project_code(cwd)
        || age(&cache_file()).is_some_and(|a| a < FRESH_SECS)
        || age(&lock_file()).is_some_and(|a| a < LOCK_SECS)
        || !Path::new(cwd).is_absolute()
        || !Path::new(cwd).is_dir()
    {
        return;
    }
    if std::fs::create_dir_all(run_dir()).is_err() || std::fs::write(lock_file(), b"").is_err() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let _ = Command::new(exe)
        .args(["grok-quota-refresh", cwd])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// Runs grok on its own tmux server, opens `/usage`, reads the dialog and writes the cache. Only keys
/// for the command are sent: the data-retention notice's buttons are never touched.
pub fn refresh(cwd: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    if probe_disabled() || !Path::new(cwd).is_absolute() || !Path::new(cwd).is_dir() {
        return false;
    }
    let dir = run_dir().join("grokq");
    if std::fs::create_dir_all(&dir).is_err()
        || std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).is_err()
    {
        return false;
    }
    let sock = dir.join("sock");
    let ok = (|| {
        tmux(
            &sock,
            &[
                "new-session",
                "-d",
                "-s",
                "q",
                "-x",
                "100",
                "-y",
                "50",
                "-c",
                cwd,
                "grok",
            ],
        );
        wait_for(&sock, "❯", 25)?;
        std::thread::sleep(Duration::from_secs(2));
        tmux(&sock, &["send-keys", "-t", "q", "/usage"]);
        std::thread::sleep(Duration::from_secs(2));
        tmux(&sock, &["send-keys", "-t", "q", "Enter"]);
        let screen = wait_for(&sock, "Usage limit", 12)?;
        let at = now();
        let limits = parse(&screen, at);
        if limits.is_empty() {
            return None;
        }
        let v = json!({"at": at, "limits": limits.iter().map(|l| json!({
            "minutes": l.0, "used": l.1, "resets_epoch": l.2,
        })).collect::<Vec<_>>()});
        let file = cache_file();
        let tmp = file.with_extension("tmp");
        std::fs::write(&tmp, v.to_string()).ok()?;
        std::fs::rename(&tmp, &file).ok()
    })()
    .is_some();
    tmux(&sock, &["kill-server"]);
    let _ = std::fs::remove_file(&sock);
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: &str = "  │  Context usage  Usage limit  Session info                     │\n  │  Weekly limit (SuperGrok Lite)                                │\n  │  ███░░░░░░░░░░░░░░░░░░░░░░░░░░░  10%                          │\n  │  Resets: November 2, 11:50                                    │\n  │  Session usage: no model calls yet in this session.           │\n";

    #[test]
    fn reads_the_weekly_window_with_its_reset() {
        let at = 1_793_000_000.0; // 2026-10-26
        let l = parse(SCREEN, at);
        assert_eq!(l.len(), 1);
        assert_eq!((l[0].0, l[0].1), (10_080, 10.0));
        let r = l[0].2.expect("a reset");
        assert!((r as f64 - at) > 0.0 && (r as f64 - at) < 8.0 * 86_400.0);
    }

    #[test]
    fn the_tab_row_is_not_a_window_and_hostile_text_yields_nothing() {
        assert!(parse("Usage limit\n50%\n", 0.0).is_empty());
        assert!(parse("", 0.0).is_empty());
        assert!(parse("Weekly limit (x)\n999%\n", 0.0).is_empty());
        assert!(parse(&"Weekly limit ".repeat(100_000), 0.0).is_empty());
    }
}
