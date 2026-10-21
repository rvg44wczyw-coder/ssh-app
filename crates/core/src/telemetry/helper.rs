use super::*;

/// The pinned helper's `telemetry` call (feature 32): a fixed script, no user text. Prints
/// `HELPER_ABSENT` when the helper is missing, a symlink or not exactly the pinned build.
pub(crate) fn build_helper_telemetry_command(sha256: &str) -> String {
    let script = format!(
        "h={path}; if [ ! -f \"$h\" ] || [ -L \"$h\" ] || \
         [ \"{SHA256_OF_H}\" != '{sha256}' ]; then echo HELPER_ABSENT; \
         else \"$h\" telemetry 2>/dev/null; fi",
        path = crate::usage::HELPER_PATH_EXPR
    );
    crate::hostprobe::login_sh_script(&script, &[], 20)
}

#[derive(Deserialize)]
pub(super) struct HelperProc {
    pid: u32,
    uid: u32,
    user: String,
    #[serde(default)]
    cpu: Option<f64>,
    #[serde(default)]
    mem: Option<f64>,
    etime: String,
    comm: String,
    #[serde(default)]
    started: Option<u64>,
}

#[derive(Deserialize)]
pub(super) struct HelperBattery {
    percent: Option<u8>,
    charging: Option<bool>,
    source: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct HelperMem {
    total: u64,
    used: u64,
}

#[derive(Deserialize)]
pub(super) struct HelperDisk {
    total_kb: u64,
    avail_kb: u64,
}

#[derive(Deserialize)]
pub(super) struct HelperTelemetry {
    ok: bool,
    host: Option<String>,
    os: Option<String>,
    cores: Option<u32>,
    load: Option<[f64; 3]>,
    mem: Option<HelperMem>,
    disk: Option<HelperDisk>,
    battery: Option<HelperBattery>,
    me_uid: Option<u32>,
    /// Missing means the helper could not list processes: the shell script is asked instead.
    procs: Option<Vec<HelperProc>>,
}

/// Text from the host that is shown in the app: bidi / zero-width / control characters become a
/// visible marker, whatever the helper did (an older or replaced build may not clean them).
pub(super) fn safe(s: &str) -> String {
    crate::approval_queue::display_safe(s)
}

pub(super) fn finite_in(v: Option<f64>, max: f64) -> f64 {
    v.filter(|x| x.is_finite()).unwrap_or(0.0).clamp(0.0, max)
}

/// Builds the snapshot from the helper's reply; `None` when there is no usable reply (absent
/// helper, older helper without `telemetry`, malformed output), so the caller falls back to the
/// shell script.
pub(crate) fn parse_helper_telemetry(out: &str) -> Option<HostTelemetrySnapshot> {
    let h: HelperTelemetry = out.lines().find_map(|l| {
        let l = l.trim();
        if !l.starts_with('{') {
            return None;
        }
        serde_json::from_str::<HelperTelemetry>(l)
            .ok()
            .filter(|t| t.ok)
    })?;
    let (used_b, total_b) = h.mem.as_ref().map_or((0, 0), |m| (m.used, m.total));
    let total_mb = total_b / (1024 * 1024);
    let used_mb = (used_b / (1024 * 1024)).min(if total_mb > 0 { total_mb } else { u64::MAX });
    let pct = |used: f64, total: f64| {
        if total > 0.0 {
            (((used / total) * 100.0).clamp(0.0, 100.0) * 10.0).round() / 10.0
        } else {
            0.0
        }
    };
    let (avail_kb, total_kb) = h.disk.as_ref().map_or((0, 0), |d| (d.avail_kb, d.total_kb));
    let used_kb = total_kb.saturating_sub(avail_kb);
    let gb = |kb: u64| ((kb as f64) / (1024.0 * 1024.0) * 10.0).round() / 10.0;
    let [load_1m, load_5m, load_15m] = h.load.unwrap_or([0.0; 3]);
    let me_uid = h.me_uid;
    let mut processes: Vec<RemoteProcessInfo> = h
        .procs?
        .into_iter()
        .map(|p| {
            let command_line: String = p.comm.chars().take(240).collect();
            let name = command_line
                .rsplit('/')
                .next()
                .unwrap_or(&command_line)
                .to_string();
            RemoteProcessInfo {
                pid: p.pid,
                group: classify_process_group(p.uid, p.user.clone(), command_line.clone(), me_uid),
                category: classify_process_category(command_line.clone()),
                owner: safe(&p.user),
                name: safe(&name),
                command_line: safe(&command_line),
                cpu_percent: finite_in(p.cpu, 100_000.0),
                mem_percent: finite_in(p.mem, 100.0),
                elapsed: safe(&p.etime),
                started_epoch: p.started,
            }
        })
        .collect();
    sort_processes(&mut processes);
    Some(HostTelemetrySnapshot {
        hostname: h.host.map_or_else(|| "unknown".to_string(), |v| safe(&v)),
        os_name: h.os.map_or_else(|| "Unix".to_string(), |v| safe(&v)),
        cpu_cores: h.cores.filter(|&c| c > 0).unwrap_or(1),
        load_1m,
        load_5m,
        load_15m,
        memory_used_mb: used_mb,
        memory_total_mb: total_mb,
        memory_usage_percent: pct(used_mb as f64, total_mb as f64),
        disk_used_gb: gb(used_kb),
        disk_total_gb: gb(total_kb),
        disk_usage_percent: pct(used_kb as f64, total_kb as f64),
        battery_percent: h.battery.as_ref().and_then(|b| b.percent),
        is_charging: h.battery.as_ref().and_then(|b| b.charging),
        power_source: h.battery.and_then(|b| b.source),
        processes,
    })
}

/// The pinned helper's `kill` call: only numbers reach the script.
pub(crate) fn build_helper_kill_command(
    sha256: &str,
    pid: u32,
    force: bool,
    started: Option<u64>,
) -> String {
    let force = if force { " --force" } else { "" };
    let started = started.map_or(String::new(), |s| format!(" --started {s}"));
    let script = format!(
        "h={path}; if [ ! -f \"$h\" ] || [ -L \"$h\" ] || \
         [ \"{SHA256_OF_H}\" != '{sha256}' ]; then echo HELPER_ABSENT; \
         else \"$h\" kill --pid {pid}{force}{started} 2>/dev/null; fi",
        path = crate::usage::HELPER_PATH_EXPR
    );
    crate::hostprobe::login_sh_script(&script, &[], 20)
}

/// `Some(Ok)` the process was signalled, `Some(Err(why))` the helper refused (shown, not retried
/// through the shell: the shell would not know the rules), `None` no usable reply.
pub(crate) fn parse_helper_kill(out: &str) -> Option<Result<(), String>> {
    for line in out.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        match v.get("ok").and_then(|o| o.as_bool()) {
            Some(true) => return Some(Ok(())),
            Some(false) => {
                let why = v.get("error").and_then(|e| e.as_str()).unwrap_or("refused");
                return Some(Err(crate::approval_queue::display_safe(
                    &why.chars().take(300).collect::<String>(),
                )));
            }
            None => {}
        }
    }
    None
}
