//! `ssh-app-host telemetry`: one typed JSON snapshot of the host (feature 32, phase T1).
//!
//! Replaces the shell script with its `===SECTION===` text blocks. The numbers still come from the
//! system tools (`sysctl`, `vm_stat`, `df`, `pmset`, `ps`, run with a fixed argv, no shell and a
//! timeout) but are parsed here, once, and handed over as raw values; the app derives percentages
//! and classifies the processes. A source that fails leaves its field out.

use crate::proc::{run_with_input, MAX_OUT};
use serde_json::{json, Value};
use std::process::Command;
use std::time::{Duration, Instant};

const TOOL_TIMEOUT: Duration = Duration::from_secs(4);
/// All tools together; the app's host-side watchdog is 20 s, so a partial answer beats none.
const TOTAL_BUDGET: Duration = Duration::from_secs(12);
const TOP_BY_CPU: usize = 150;
const MAX_PROCS: usize = 400;
const MAX_COMM: usize = 240;
/// Idle agents and local-LLM servers sit far below the CPU cut-off on a busy machine; keep them.
const ALWAYS_KEEP: [&str; 11] = [
    "claude",
    "antigravity",
    "agy",
    "codex",
    "aider",
    "cursor",
    "ollama",
    "llama",
    "lmstudio",
    "vllm",
    "mlx",
];

/// System tools by absolute path, so a program earlier in the user's `PATH` cannot stand in for
/// them; the bare name is the fallback where the absolute path does not exist.
fn resolve(program: &str) -> String {
    ["/bin", "/usr/bin", "/usr/sbin", "/sbin"]
        .iter()
        .map(|d| format!("{d}/{program}"))
        .find(|p| std::path::Path::new(p).is_file())
        .unwrap_or_else(|| program.to_string())
}

/// Output of a tool that finished before `deadline` (and within `TOOL_TIMEOUT`). A tool that was
/// cut off because it wrote more than `MAX_OUT` (it dies from SIGPIPE) still counts: the text is
/// shortened to its last complete line and the flag says so.
fn run_tool(program: &str, args: &[&str], deadline: Instant) -> Option<(String, bool)> {
    let left = deadline.checked_duration_since(Instant::now())?;
    let mut cmd = Command::new(resolve(program));
    cmd.args(args).env("LC_ALL", "C");
    let (code, out) = run_with_input(cmd, b"", left.min(TOOL_TIMEOUT))?;
    let capped = out.len() as u64 >= MAX_OUT;
    if code != Some(0) && !capped {
        return None;
    }
    let mut text = String::from_utf8_lossy(&out).into_owned();
    if capped {
        text.truncate(text.rfind('\n').map_or(0, |i| i + 1));
    }
    Some((text, capped))
}

/// `[[dd-]hh:]mm:ss` as seconds.
pub fn etime_secs(e: &str) -> Option<u64> {
    let (days, rest) = match e.split_once('-') {
        Some((d, r)) => (d.parse::<u64>().ok()?, r),
        None => (0, e),
    };
    let mut secs = 0u64;
    let parts: Vec<&str> = rest.split(':').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    for p in parts {
        secs = secs.checked_mul(60)?.checked_add(p.parse::<u64>().ok()?)?;
    }
    days.checked_mul(86_400)?.checked_add(secs)
}

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// One process as `ps` sees it: (parent pid, uid, start epoch, command).
pub fn proc_info(pid: u32) -> Option<(u32, u32, u64, String)> {
    let deadline = Instant::now() + TOOL_TIMEOUT;
    let out = tool(
        "ps",
        &["-o", "ppid=,uid=,etime=,comm=", "-p", &pid.to_string()],
        deadline,
    )?;
    let line = out.lines().next()?;
    let mut it = line.split_whitespace();
    let ppid = it.next()?.parse().ok()?;
    let uid = it.next()?.parse().ok()?;
    let started = now_epoch().saturating_sub(etime_secs(it.next()?)?);
    Some((ppid, uid, started, clean(&it.collect::<Vec<_>>().join(" "))))
}

fn tool(program: &str, args: &[&str], deadline: Instant) -> Option<String> {
    run_tool(program, args, deadline).map(|(t, _)| t)
}

/// Characters that reorder or hide text (controls, bidi, zero-width, line separators): dropped,
/// because a process name is shown where the user decides what to stop.
fn spoofy(c: char) -> bool {
    c.is_control()
        || matches!(c,
            '\u{ad}' | '\u{61c}' | '\u{180e}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}' | '\u{feff}' | '\u{fff9}'..='\u{fffb}')
}

fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| !spoofy(*c))
        .collect::<String>()
        .trim()
        .to_string()
}

fn finite(v: f64, max: f64) -> f64 {
    if v.is_finite() {
        v.clamp(0.0, max)
    } else {
        0.0
    }
}

/// Used bytes on macOS: active + wired + compressor pages times the page size `vm_stat` reports.
pub fn parse_vm_stat_used(text: &str) -> Option<u64> {
    let mut page: u64 = 16384;
    let (mut active, mut wired, mut comp, mut seen) = (0u64, 0u64, 0u64, false);
    for line in text.lines() {
        let pages = |prefix: &str| {
            line.strip_prefix(prefix)
                .and_then(|r| r.trim().trim_end_matches('.').parse::<u64>().ok())
        };
        if line.contains("page size of") {
            if let Some(p) = line.split_whitespace().find_map(|t| t.parse::<u64>().ok()) {
                page = p.max(1);
            }
        } else if let Some(v) = pages("Pages active:") {
            (active, seen) = (v, true);
        } else if let Some(v) = pages("Pages wired down:") {
            wired = v;
        } else if let Some(v) = pages("Pages occupied by compressor:") {
            comp = v;
        }
    }
    seen.then(|| {
        active
            .saturating_add(wired)
            .saturating_add(comp)
            .saturating_mul(page)
    })
}

/// `df -k /` last line: `fs 1K-blocks used avail ...` -> (total KiB, available KiB).
pub fn parse_df(text: &str) -> Option<(u64, u64)> {
    let cols: Vec<&str> = text.lines().last()?.split_whitespace().collect();
    if cols.len() < 4 {
        return None;
    }
    Some((cols[1].parse().ok()?, cols[3].parse().ok()?))
}

/// `pmset -g batt`: (percent, charging, source). All `None` on a desktop.
pub fn parse_battery(text: &str) -> (Option<u8>, Option<bool>, Option<String>) {
    let (mut pct, mut charging, mut source) = (None, None, None);
    for line in text.lines() {
        if line.contains("Now drawing from") {
            if line.contains("AC Power") {
                source = Some("AC Power".to_string());
            } else if line.contains("Battery Power") {
                source = Some("Battery Power".to_string());
            }
        }
        if line.contains('%') {
            if let Some(p) = line.split_whitespace().find_map(|t| {
                t.trim_end_matches(';')
                    .strip_suffix('%')?
                    .parse::<u8>()
                    .ok()
            }) {
                pct = Some(p.min(100));
            }
            let lower = line.to_lowercase();
            if lower.contains("discharging") {
                charging = Some(false);
            } else if lower.contains("charging") || lower.contains("charged") {
                charging = Some(true);
            }
        }
    }
    (pct, charging, source)
}

/// `ps -Ao pid=,uid=,user=,pcpu=,pmem=,etime=,comm=` lines -> process objects, the 150 busiest by
/// CPU plus every agent / local-LLM process, at most 400.
pub fn parse_ps(text: &str) -> Vec<Value> {
    struct P {
        cpu: f64,
        keep: bool,
        v: Value,
    }
    let mut all: Vec<P> = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(pid), Some(uid), Some(user), Some(cpu), Some(mem), Some(etime)) = (
            it.next().and_then(|s| s.parse::<u32>().ok()),
            it.next().and_then(|s| s.parse::<u32>().ok()),
            it.next(),
            it.next(),
            it.next(),
            it.next(),
        ) else {
            continue;
        };
        let comm: String = clean(&it.collect::<Vec<_>>().join(" "))
            .chars()
            .take(MAX_COMM)
            .collect();
        if comm.is_empty() {
            continue;
        }
        let cpu = finite(
            cpu.replace(',', ".").parse::<f64>().unwrap_or(0.0),
            100_000.0,
        );
        let mem = finite(mem.replace(',', ".").parse::<f64>().unwrap_or(0.0), 100.0);
        let lower = comm.to_lowercase();
        all.push(P {
            cpu,
            keep: ALWAYS_KEEP.iter().any(|k| lower.contains(k)),
            v: json!({"pid": pid, "uid": uid, "user": clean(user), "cpu": cpu, "mem": mem,
                      "etime": clean(etime), "comm": comm,
                      "started": etime_secs(etime).map(|e| now_epoch().saturating_sub(e))}),
        });
    }
    all.sort_by(|a, b| {
        b.cpu
            .partial_cmp(&a.cpu)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    all.into_iter()
        .enumerate()
        .filter(|(i, p)| *i < TOP_BY_CPU || p.keep)
        .map(|(_, p)| p.v)
        .take(MAX_PROCS)
        .collect()
}

fn load_average() -> Option<[f64; 3]> {
    let mut l = [0f64; 3];
    // SAFETY: `l` has room for the 3 samples requested.
    let n = unsafe { libc::getloadavg(l.as_mut_ptr(), 3) };
    (n == 3).then_some(l)
}

pub fn snapshot() -> Value {
    snapshot_within(TOTAL_BUDGET)
}

fn snapshot_within(budget: Duration) -> Value {
    let deadline = Instant::now() + budget;
    let mut o = serde_json::Map::new();
    o.insert("ok".into(), json!(true));
    if let Some(h) = tool("hostname", &[], deadline) {
        o.insert("host".into(), json!(clean(&h)));
    }
    if let Some(u) = tool("uname", &["-srm"], deadline) {
        o.insert("os".into(), json!(clean(&u)));
    }
    if let Some(c) =
        tool("sysctl", &["-n", "hw.ncpu"], deadline).and_then(|s| s.trim().parse::<u32>().ok())
    {
        o.insert("cores".into(), json!(c));
    }
    if let Some(l) = load_average() {
        o.insert("load".into(), json!(l));
    }
    let total =
        tool("sysctl", &["-n", "hw.memsize"], deadline).and_then(|s| s.trim().parse::<u64>().ok());
    let used = tool("vm_stat", &[], deadline).and_then(|s| parse_vm_stat_used(&s));
    if let (Some(t), Some(u)) = (total, used) {
        o.insert("mem".into(), json!({"total": t, "used": u.min(t)}));
    }
    if let Some((t, a)) = tool("df", &["-k", "/"], deadline).and_then(|s| parse_df(&s)) {
        o.insert("disk".into(), json!({"total_kb": t, "avail_kb": a}));
    }
    if let Some(b) = tool("pmset", &["-g", "batt"], deadline) {
        let (pct, charging, source) = parse_battery(&b);
        if pct.is_some() || source.is_some() {
            o.insert(
                "battery".into(),
                json!({"percent": pct, "charging": charging, "source": source}),
            );
        }
    }
    // SAFETY: getuid has no preconditions.
    o.insert("me_uid".into(), json!(unsafe { libc::getuid() }));
    if let Some((p, capped)) = run_tool(
        "ps",
        &["-Ao", "pid=,uid=,user=,pcpu=,pmem=,etime=,comm="],
        deadline,
    ) {
        o.insert("procs".into(), Value::Array(parse_ps(&p)));
        o.insert("procs_truncated".into(), json!(capped));
    }
    Value::Object(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VM: &str = "Mach Virtual Memory Statistics: (page size of 16384 bytes)\nPages free: 100.\nPages active: 1000.\nPages wired down: 500.\nPages occupied by compressor: 250.\n";

    #[test]
    fn vm_stat_used_is_active_wired_compressed() {
        assert_eq!(parse_vm_stat_used(VM), Some(1750 * 16384));
        assert_eq!(parse_vm_stat_used("nothing"), None);
        let small = VM.replace("16384", "4096");
        assert_eq!(parse_vm_stat_used(&small), Some(1750 * 4096));
    }

    #[test]
    fn df_and_battery() {
        assert_eq!(
            parse_df(
                "Filesystem 1024-blocks Used Available Capacity\n/dev/disk3s1 1000 400 600 40% /\n"
            ),
            Some((1000, 600))
        );
        assert_eq!(parse_df(""), None);
        let b = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=1)\t87%; charging; 1:10 remaining present: true\n";
        assert_eq!(
            parse_battery(b),
            (Some(87), Some(true), Some("AC Power".into()))
        );
        let d = "Now drawing from 'Battery Power'\n -InternalBattery-0\t55%; discharging; 3:00 remaining\n";
        assert_eq!(
            parse_battery(d),
            (Some(55), Some(false), Some("Battery Power".into()))
        );
        assert_eq!(parse_battery("Now drawing from 'AC Power'\n").0, None);
    }

    #[test]
    fn ps_keeps_idle_agents_and_caps() {
        let mut text = String::new();
        for i in 0..400 {
            text.push_str(&format!(
                "{} 501 u 5.0 0.1 01:00 /usr/bin/busy{i}\n",
                1000 + i
            ));
        }
        text.push_str("9001 501 u 0.0 0.1 02:00 /opt/ollama\n9002 501 u 0,0 0.1 1-02:00:00 /Users/u/.local/bin/claude\n");
        text.push_str("garbage line\n7 x u 1 1 1 bad\n");
        let procs = parse_ps(&text);
        assert_eq!(procs.len(), 152);
        assert!(procs.iter().any(|p| p["pid"] == 9001));
        assert!(procs
            .iter()
            .any(|p| p["pid"] == 9002 && p["etime"] == "1-02:00:00"));
    }

    #[test]
    fn ps_comm_with_spaces_and_controls() {
        let procs =
            parse_ps("12 501 u 1.0 2.0 00:05 /Applications/My App.app/Contents/x\u{1b}[31m\n");
        assert_eq!(procs[0]["comm"], "/Applications/My App.app/Contents/x[31m");
    }

    #[test]
    fn snapshot_has_core_fields_here() {
        let s = snapshot();
        assert_eq!(s["ok"], true);
        assert!(s["load"].is_array());
        assert!(s["procs"].as_array().is_some_and(|p| !p.is_empty()));
    }

    #[test]
    fn bidi_and_zero_width_are_dropped() {
        let procs = parse_ps("12 501 u 1.0 2.0 00:05 /bin/e\u{202e}\u{200b}gnp.txt\u{2066}\n");
        assert_eq!(procs[0]["comm"], "/bin/egnp.txt");
        assert_eq!(clean("a\u{feff}b\u{85}c"), "abc");
    }

    #[test]
    fn non_finite_and_negative_numbers_are_made_safe() {
        let procs =
            parse_ps("1 501 u nan 1e999 0:1 /a\n2 501 u inf -3 0:1 /b\n3 501 u -5 1.0 0:1 /c\n");
        assert_eq!(procs.len(), 3);
        for p in &procs {
            assert!(
                p["cpu"]
                    .as_f64()
                    .is_some_and(|c| (0.0..=100_000.0).contains(&c)),
                "{p}"
            );
            assert!(
                p["mem"]
                    .as_f64()
                    .is_some_and(|m| (0.0..=100.0).contains(&m)),
                "{p}"
            );
        }
    }

    #[test]
    fn an_expired_budget_skips_the_tool_and_a_slow_tool_is_cut() {
        assert!(run_tool("sleep", &["5"], Instant::now()).is_none());
        let t = Instant::now();
        assert!(run_tool("sleep", &["5"], Instant::now() + Duration::from_millis(300)).is_none());
        assert!(t.elapsed() < Duration::from_secs(2));
        let s = snapshot_within(Duration::ZERO);
        assert_eq!(s["ok"], true);
        assert!(s.get("procs").is_none() && s.get("host").is_none());
    }

    #[test]
    fn output_over_the_cap_is_cut_at_a_whole_line_and_flagged() {
        // `yes` writes forever and dies from SIGPIPE once the cap is read: partial output, not none.
        let (text, capped) = run_tool(
            "yes",
            &["1 501 u 1.0 1.0 0:1 /opt/x"],
            Instant::now() + Duration::from_secs(5),
        )
        .expect("capped output");
        assert!(capped && text.ends_with('\n') && text.len() as u64 <= MAX_OUT);
        assert!(!parse_ps(&text).is_empty());
    }
}
