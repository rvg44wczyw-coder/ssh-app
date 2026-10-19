//! `ssh-app-host telemetry`: one typed JSON snapshot of the host (feature 32, phase T1).
//!
//! Replaces the shell script with its `===SECTION===` text blocks. The numbers still come from the
//! system tools (`sysctl`, `vm_stat`, `df`, `pmset`, `ps`, run with a fixed argv, no shell and a
//! timeout) but are parsed here, once, and handed over as raw values; the app derives percentages
//! and classifies the processes. A source that fails leaves its field out.

use crate::proc::run_with_input;
use serde_json::{json, Value};
use std::process::Command;
use std::time::Duration;

const TOOL_TIMEOUT: Duration = Duration::from_secs(4);
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

fn tool(program: &str, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new(program);
    cmd.args(args).env("LC_ALL", "C");
    let (code, out) = run_with_input(cmd, b"", TOOL_TIMEOUT)?;
    (code == Some(0)).then(|| String::from_utf8_lossy(&out).into_owned())
}

fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_string()
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
        let cpu = cpu.replace(',', ".").parse::<f64>().unwrap_or(0.0);
        let mem = mem.replace(',', ".").parse::<f64>().unwrap_or(0.0);
        let lower = comm.to_lowercase();
        all.push(P {
            cpu,
            keep: ALWAYS_KEEP.iter().any(|k| lower.contains(k)),
            v: json!({"pid": pid, "uid": uid, "user": clean(user), "cpu": cpu, "mem": mem,
                      "etime": clean(etime), "comm": comm}),
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
    let mut o = serde_json::Map::new();
    o.insert("ok".into(), json!(true));
    if let Some(h) = tool("hostname", &[]) {
        o.insert("host".into(), json!(clean(&h)));
    }
    if let Some(u) = tool("uname", &["-srm"]) {
        o.insert("os".into(), json!(clean(&u)));
    }
    if let Some(c) = tool("sysctl", &["-n", "hw.ncpu"]).and_then(|s| s.trim().parse::<u32>().ok()) {
        o.insert("cores".into(), json!(c));
    }
    if let Some(l) = load_average() {
        o.insert("load".into(), json!(l));
    }
    let total = tool("sysctl", &["-n", "hw.memsize"]).and_then(|s| s.trim().parse::<u64>().ok());
    let used = tool("vm_stat", &[]).and_then(|s| parse_vm_stat_used(&s));
    if let (Some(t), Some(u)) = (total, used) {
        o.insert("mem".into(), json!({"total": t, "used": u.min(t)}));
    }
    if let Some((t, a)) = tool("df", &["-k", "/"]).and_then(|s| parse_df(&s)) {
        o.insert("disk".into(), json!({"total_kb": t, "avail_kb": a}));
    }
    if let Some(b) = tool("pmset", &["-g", "batt"]) {
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
    if let Some(p) = tool("ps", &["-Ao", "pid=,uid=,user=,pcpu=,pmem=,etime=,comm="]) {
        o.insert("procs".into(), Value::Array(parse_ps(&p)));
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
}
