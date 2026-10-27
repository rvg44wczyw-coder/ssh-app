//! Linux sources for `telemetry` and the process lookups of `kill` (feature 33, phase H2): everything
//! comes from `/proc`, `/sys` and `statvfs`, no `ps`/`vm_stat`/`pmset` (so no procps/busybox
//! differences and no `PATH` dependency). The parsers are plain functions over text and are tested on
//! every OS with captured samples; only the readers at the bottom are Linux-only.

use serde_json::{json, Value};

/// `(total, used)` bytes from `/proc/meminfo`: used is `MemTotal - MemAvailable`.
pub fn parse_meminfo(text: &str) -> Option<(u64, u64)> {
    let kb = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key))
            .and_then(|r| r.split_whitespace().next())
            .and_then(|n| n.parse::<u64>().ok())
    };
    let total = kb("MemTotal:")?.checked_mul(1024)?;
    let avail = kb("MemAvailable:")
        .or_else(|| kb("MemFree:"))?
        .checked_mul(1024)?;
    Some((total, total.saturating_sub(avail).min(total)))
}

/// `/proc/loadavg` -> the three load averages.
pub fn parse_loadavg(text: &str) -> Option<[f64; 3]> {
    let mut it = text.split_whitespace().map(|t| t.parse::<f64>().ok());
    let v = [it.next()??, it.next()??, it.next()??];
    v.iter().all(|x| x.is_finite() && *x >= 0.0).then_some(v)
}

/// Number of `processor` entries in `/proc/cpuinfo` (0 when none is listed).
pub fn count_cpus(cpuinfo: &str) -> u32 {
    cpuinfo
        .lines()
        .filter(|l| l.starts_with("processor") && l.contains(':'))
        .count() as u32
}

/// Fields of `/proc/<pid>/stat` that the helper needs.
#[derive(Debug, PartialEq)]
pub struct Stat {
    pub comm: String,
    pub ppid: u32,
    /// utime + stime in clock ticks.
    pub cpu_ticks: u64,
    /// Start time in clock ticks after boot.
    pub start_ticks: u64,
}

/// The command name sits between the first `(` and the LAST `)` and may contain spaces and
/// parentheses, so the numeric fields are counted from the closing parenthesis.
pub fn parse_stat(text: &str) -> Option<Stat> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    if close < open {
        return None;
    }
    let comm = text[open + 1..close].to_string();
    let rest: Vec<&str> = text[close + 1..].split_whitespace().collect();
    // rest[0] = state (field 3), ppid = field 4 -> rest[1], utime = field 14 -> rest[11],
    // stime = field 15 -> rest[12], starttime = field 22 -> rest[19].
    let num = |i: usize| rest.get(i)?.parse::<u64>().ok();
    Some(Stat {
        comm,
        ppid: u32::try_from(num(1)?).ok()?,
        cpu_ticks: num(11)?.saturating_add(num(12)?),
        start_ticks: num(19)?,
    })
}

/// Real uid from `/proc/<pid>/status`.
pub fn parse_status_uid(text: &str) -> Option<u32> {
    text.lines()
        .find_map(|l| l.strip_prefix("Uid:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// Seconds since boot from `/proc/uptime`.
pub fn parse_uptime(text: &str) -> Option<f64> {
    let v = text.split_whitespace().next()?.parse::<f64>().ok()?;
    (v.is_finite() && v >= 0.0).then_some(v)
}

/// `[[dd-]hh:]mm:ss` like `ps -o etime`.
pub fn format_etime(secs: u64) -> String {
    let (d, h, m, s) = (secs / 86_400, secs / 3600 % 24, secs / 60 % 60, secs % 60);
    if d > 0 {
        format!("{d}-{h:02}:{m:02}:{s:02}")
    } else if h > 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// `name` for `uid` from the text of `/etc/passwd`.
pub fn passwd_name(passwd: &str, uid: u32) -> Option<String> {
    passwd.lines().find_map(|l| {
        let mut f = l.split(':');
        let name = f.next()?;
        let _pw = f.next()?;
        (f.next()?.parse::<u32>().ok()? == uid).then(|| name.to_string())
    })
}

/// A command line from `/proc/<pid>/cmdline` (NUL separated): argument 0 as the name, the whole
/// line (spaces) for pattern matching. `None` for kernel threads (empty).
pub fn parse_cmdline(raw: &[u8]) -> Option<(String, String)> {
    let args: Vec<String> = raw
        .split(|b| *b == 0)
        .filter(|a| !a.is_empty())
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect();
    let first = args.first()?.clone();
    Some((first, args.join(" ")))
}

/// Battery state from the files of the `power_supply` class: `(percent, charging, source)`.
/// `supplies` is `(type, capacity, status, online)` per directory.
pub fn parse_power_supplies(
    supplies: &[(String, String, String, String)],
) -> (Option<u8>, Option<bool>, Option<String>) {
    let mut pct = None;
    let mut charging = None;
    let mut ac = false;
    let mut have_battery = false;
    for (kind, capacity, status, online) in supplies {
        match kind.trim() {
            "Battery" => {
                have_battery = true;
                pct = pct.or_else(|| capacity.trim().parse::<u8>().ok().map(|p| p.min(100)));
                let st = status.trim().to_ascii_lowercase();
                charging = charging.or(match st.as_str() {
                    "charging" | "full" => Some(true),
                    "discharging" | "not charging" => Some(false),
                    _ => None,
                });
            }
            "Mains" | "USB" | "USB_PD" if online.trim() == "1" => ac = true,
            _ => {}
        }
    }
    let source = have_battery.then(|| if ac { "AC Power" } else { "Battery Power" }.to_string());
    (pct, charging, source)
}

/// One process row in the shape `telemetry` sends, from already-read pieces.
#[allow(clippy::too_many_arguments)]
pub fn process_row(
    pid: u32,
    uid: u32,
    user: &str,
    stat: &Stat,
    cmdline: Option<(String, String)>,
    rss_pages: u64,
    page_size: u64,
    mem_total: u64,
    uptime: f64,
    hz: f64,
    now: u64,
) -> Value {
    let elapsed = (uptime - stat.start_ticks as f64 / hz).max(0.0);
    let cpu = if elapsed > 0.0 {
        (stat.cpu_ticks as f64 / hz / elapsed * 100.0).clamp(0.0, 100_000.0)
    } else {
        0.0
    };
    let mem = if mem_total > 0 {
        (rss_pages.saturating_mul(page_size) as f64 / mem_total as f64 * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };
    let comm = match &cmdline {
        Some((first, _)) => first.clone(),
        None => format!("[{}]", stat.comm),
    };
    json!({"pid": pid, "uid": uid, "user": user, "cpu": cpu, "mem": mem,
           "etime": format_etime(elapsed as u64), "comm": comm,
           "started": now.saturating_sub(elapsed as u64),
           "line": cmdline.map(|(_, l)| l)})
}

#[cfg(target_os = "linux")]
pub use reader::*;

#[cfg(target_os = "linux")]
mod reader {
    use super::*;
    use std::fs;

    fn read(path: &str) -> Option<String> {
        fs::read_to_string(path).ok()
    }

    /// A command line, at most 64 KiB (an argument list can be megabytes).
    fn read_cmdline(pid: u32) -> Option<Vec<u8>> {
        use std::io::Read;
        let mut buf = Vec::new();
        fs::File::open(format!("/proc/{pid}/cmdline"))
            .ok()?
            .take(65_536)
            .read_to_end(&mut buf)
            .ok()?;
        Some(buf)
    }

    fn hz() -> f64 {
        // SAFETY: sysconf has no preconditions.
        let v = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        if v > 0 {
            v as f64
        } else {
            100.0
        }
    }

    fn page_size() -> u64 {
        // SAFETY: sysconf has no preconditions.
        let v = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if v > 0 {
            v as u64
        } else {
            4096
        }
    }

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }

    fn pids() -> Vec<u32> {
        fs::read_dir("/proc")
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `(ppid, uid, start epoch, command)` of one process.
    pub fn proc_info(pid: u32) -> Option<(u32, u32, u64, String)> {
        let stat = parse_stat(&read(&format!("/proc/{pid}/stat"))?)?;
        let uid = parse_status_uid(&read(&format!("/proc/{pid}/status"))?)?;
        let uptime = parse_uptime(&read("/proc/uptime")?)?;
        let elapsed = (uptime - stat.start_ticks as f64 / hz()).max(0.0) as u64;
        let comm = read_cmdline(pid)
            .and_then(|raw| parse_cmdline(&raw))
            .map_or_else(|| stat.comm.clone(), |(first, _)| first);
        Some((
            stat.ppid,
            uid,
            now().saturating_sub(elapsed),
            comm.chars().filter(|c| !c.is_control()).collect(),
        ))
    }

    pub fn has_children(pid: u32) -> Option<bool> {
        let mut any = false;
        for p in pids() {
            if let Some(s) = read(&format!("/proc/{p}/stat")).and_then(|t| parse_stat(&t)) {
                any |= s.ppid == pid;
            }
        }
        Some(any)
    }

    pub fn processes(keep: &dyn Fn(&str) -> bool, top: usize, max: usize) -> Vec<Value> {
        let passwd = read("/etc/passwd").unwrap_or_default();
        let mem_total = read("/proc/meminfo")
            .and_then(|t| parse_meminfo(&t))
            .map_or(0, |m| m.0);
        let uptime = read("/proc/uptime")
            .and_then(|t| parse_uptime(&t))
            .unwrap_or(0.0);
        let (hz, ps, now) = (hz(), page_size(), now());
        let mut rows: Vec<(f64, bool, Value)> = Vec::new();
        for pid in pids() {
            let Some(stat) = read(&format!("/proc/{pid}/stat")).and_then(|t| parse_stat(&t)) else {
                continue;
            };
            let Some(uid) = read(&format!("/proc/{pid}/status")).and_then(|t| parse_status_uid(&t))
            else {
                continue;
            };
            let cmdline = read_cmdline(pid).and_then(|r| parse_cmdline(&r));
            let rss = read(&format!("/proc/{pid}/statm"))
                .and_then(|t| t.split_whitespace().nth(1)?.parse::<u64>().ok())
                .unwrap_or(0);
            let user = passwd_name(&passwd, uid).unwrap_or_else(|| uid.to_string());
            let mut row = process_row(
                pid,
                uid,
                &user,
                &stat,
                cmdline.clone(),
                rss,
                ps,
                mem_total,
                uptime,
                hz,
                now,
            );
            let line = row
                .as_object_mut()
                .and_then(|m| m.remove("line"))
                .unwrap_or(Value::Null);
            let matches = keep(&cmdline.map_or_else(|| stat.comm.clone(), |(_, l)| l))
                || keep(line.as_str().unwrap_or(""));
            let cpu = row["cpu"].as_f64().unwrap_or(0.0);
            rows.push((cpu, matches, row));
        }
        rows.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        rows.into_iter()
            .enumerate()
            .filter(|(i, r)| *i < top || r.1)
            .map(|(_, r)| r.2)
            .take(max)
            .collect()
    }

    pub fn battery() -> (Option<u8>, Option<bool>, Option<String>) {
        let mut supplies = Vec::new();
        if let Ok(rd) = fs::read_dir("/sys/class/power_supply") {
            for e in rd.flatten() {
                let d = e.path();
                let f = |n: &str| fs::read_to_string(d.join(n)).unwrap_or_default();
                supplies.push((f("type"), f("capacity"), f("status"), f("online")));
            }
        }
        parse_power_supplies(&supplies)
    }

    /// `(total KiB, available KiB)` of `/`.
    pub fn disk() -> Option<(u64, u64)> {
        // SAFETY: a zeroed statvfs is a valid out-buffer; the path is a NUL-terminated literal.
        let mut sv: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c"/".as_ptr(), &mut sv) } != 0 {
            return None;
        }
        let unit = sv.f_frsize as u64;
        Some((
            sv.f_blocks as u64 * unit / 1024,
            sv.f_bavail as u64 * unit / 1024,
        ))
    }

    pub fn host_and_os() -> (Option<String>, Option<String>) {
        let host = read("/proc/sys/kernel/hostname").map(|h| h.trim().to_string());
        // SAFETY: a zeroed utsname is a valid out-buffer.
        let mut u: libc::utsname = unsafe { std::mem::zeroed() };
        let os = (unsafe { libc::uname(&mut u) } == 0).then(|| {
            let field = |f: &[libc::c_char]| {
                f.iter()
                    .take_while(|c| **c != 0)
                    .map(|c| *c as u8 as char)
                    .collect::<String>()
            };
            format!(
                "{} {} {}",
                field(&u.sysname),
                field(&u.release),
                field(&u.machine)
            )
        });
        (host, os)
    }

    pub fn cores() -> Option<u32> {
        let n = read("/proc/cpuinfo").map_or(0, |t| count_cpus(&t));
        (n > 0).then_some(n)
    }

    pub fn memory() -> Option<(u64, u64)> {
        parse_meminfo(&read("/proc/meminfo")?)
    }

    pub fn load() -> Option<[f64; 3]> {
        parse_loadavg(&read("/proc/loadavg")?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEMINFO: &str = "MemTotal:       16384000 kB\nMemFree:          200000 kB\nMemAvailable:    8192000 kB\nBuffers:           1000 kB\n";
    const STAT: &str = "1234 (node (worker) x) S 1 1234 1234 0 -1 4194304 1000 0 0 0 150 50 0 0 20 0 8 0 5000 123456789 2000 18446744073709551615 0 0 0 0 0 0 0 0 0 0 0 0 17 3 0 0 0 0 0\n";

    #[test]
    fn meminfo_loadavg_cpus() {
        assert_eq!(
            parse_meminfo(MEMINFO),
            Some((16_384_000 * 1024, 8_192_000 * 1024))
        );
        assert_eq!(
            parse_meminfo("MemTotal: 100 kB\nMemFree: 40 kB\n"),
            Some((102_400, 61_440))
        );
        assert_eq!(parse_meminfo("nothing"), None);
        assert_eq!(
            parse_loadavg("0.52 0.58 0.59 2/933 12345\n"),
            Some([0.52, 0.58, 0.59])
        );
        assert_eq!(parse_loadavg("nan 1 2"), None);
        assert_eq!(parse_loadavg(""), None);
        assert_eq!(
            count_cpus("processor\t: 0\nmodel name\t: x\nprocessor\t: 1\n"),
            2
        );
        assert_eq!(count_cpus("Processor : ARMv7\n"), 0);
    }

    #[test]
    fn stat_with_spaces_and_parentheses_in_the_name() {
        let s = parse_stat(STAT).unwrap();
        assert_eq!(
            s,
            Stat {
                comm: "node (worker) x".into(),
                ppid: 1,
                cpu_ticks: 200,
                start_ticks: 5000
            }
        );
        assert!(parse_stat("garbage").is_none());
        assert!(parse_stat("1 (a) S").is_none());
        assert_eq!(
            parse_status_uid("Name:\tx\nUid:\t1000\t1000\t1000\t1000\n"),
            Some(1000)
        );
        assert_eq!(parse_status_uid("Name:\tx\n"), None);
    }

    #[test]
    fn times_names_and_command_lines() {
        assert_eq!(format_etime(5), "00:05");
        assert_eq!(format_etime(3723), "01:02:03");
        assert_eq!(format_etime(2 * 86_400 + 3600), "2-01:00:00");
        assert_eq!(parse_uptime("1234.56 5678.9\n"), Some(1234.56));
        assert_eq!(
            passwd_name(
                "root:x:0:0::/root:/bin/sh\ndev:x:1000:1000::/home/dev:/bin/zsh\n",
                1000
            )
            .as_deref(),
            Some("dev")
        );
        assert_eq!(passwd_name("root:x:0:0\n", 5), None);
        assert_eq!(
            parse_cmdline(b"/usr/bin/node\0/opt/claude\0--x\0"),
            Some((
                "/usr/bin/node".into(),
                "/usr/bin/node /opt/claude --x".into()
            ))
        );
        assert_eq!(parse_cmdline(b""), None);
    }

    #[test]
    fn rows_and_battery() {
        let stat = parse_stat(STAT).unwrap();
        // uptime 100 s, process started at tick 5000 / 100 Hz = 50 s -> 50 s old; 200 ticks = 2 s cpu.
        let row = process_row(
            1234,
            1000,
            "dev",
            &stat,
            Some(("/usr/bin/node".into(), "node x".into())),
            2560,
            4096,
            16_384_000 * 1024,
            100.0,
            100.0,
            1_000_000,
        );
        assert_eq!(row["etime"], "00:50");
        assert_eq!(row["started"], 1_000_000 - 50);
        assert!((row["cpu"].as_f64().unwrap() - 4.0).abs() < 1e-9);
        assert!(row["mem"].as_f64().unwrap() > 0.0);
        let kernel = process_row(2, 0, "root", &stat, None, 0, 4096, 0, 100.0, 100.0, 10);
        assert_eq!(kernel["comm"], "[node (worker) x]");
        let bat = [
            (
                "Battery".to_string(),
                "87\n".to_string(),
                "Discharging\n".to_string(),
                String::new(),
            ),
            (
                "Mains".to_string(),
                String::new(),
                String::new(),
                "0\n".to_string(),
            ),
        ];
        assert_eq!(
            parse_power_supplies(&bat),
            (Some(87), Some(false), Some("Battery Power".into()))
        );
        let ac = [
            (
                "Battery".to_string(),
                "100".to_string(),
                "Full".to_string(),
                String::new(),
            ),
            (
                "Mains".to_string(),
                String::new(),
                String::new(),
                "1".to_string(),
            ),
        ];
        assert_eq!(
            parse_power_supplies(&ac),
            (Some(100), Some(true), Some("AC Power".into()))
        );
        assert_eq!(parse_power_supplies(&[]), (None, None, None));
    }
}
