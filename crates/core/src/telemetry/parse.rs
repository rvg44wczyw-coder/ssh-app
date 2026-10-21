use super::*;

#[uniffi::export]
pub fn parse_host_telemetry(raw_output: String) -> HostTelemetrySnapshot {
    let cleaned = strip_ansi_codes(raw_output);
    let mut current_section = "";

    let mut host_lines: Vec<&str> = Vec::new();
    let mut cores_lines: Vec<&str> = Vec::new();
    let mut load_lines: Vec<&str> = Vec::new();
    let mut mem_lines: Vec<&str> = Vec::new();
    let mut disk_lines: Vec<&str> = Vec::new();
    let mut batt_lines: Vec<&str> = Vec::new();
    let mut proc_lines: Vec<&str> = Vec::new();
    let mut me_lines: Vec<&str> = Vec::new();

    for line in cleaned.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match trimmed {
            "===HOST===" => {
                current_section = "HOST";
                continue;
            }
            "===CORES===" => {
                current_section = "CORES";
                continue;
            }
            "===LOAD===" => {
                current_section = "LOAD";
                continue;
            }
            "===MEM===" => {
                current_section = "MEM";
                continue;
            }
            "===DISK===" => {
                current_section = "DISK";
                continue;
            }
            "===BATT===" => {
                current_section = "BATT";
                continue;
            }
            "===ME===" => {
                current_section = "ME";
                continue;
            }
            "===PROC===" => {
                current_section = "PROC";
                continue;
            }
            _ => {}
        }

        match current_section {
            "HOST" => host_lines.push(trimmed),
            "CORES" => cores_lines.push(trimmed),
            "LOAD" => load_lines.push(trimmed),
            "MEM" => mem_lines.push(trimmed),
            "DISK" => disk_lines.push(trimmed),
            "BATT" => batt_lines.push(trimmed),
            "ME" => me_lines.push(trimmed),
            "PROC" => proc_lines.push(trimmed),
            _ => {}
        }
    }

    let hostname = host_lines.first().copied().unwrap_or("unknown").to_string();
    let os_name = host_lines.get(1).copied().unwrap_or("Unix").to_string();

    let cpu_cores = cores_lines
        .first()
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|&c| c > 0)
        .unwrap_or(1);

    let (load_1m, load_5m, load_15m) = parse_load_averages(&load_lines);
    let (memory_used_mb, memory_total_mb, memory_usage_percent) = parse_memory_metrics(&mem_lines);
    let (disk_used_gb, disk_total_gb, disk_usage_percent) = parse_disk_metrics(&disk_lines);
    let (battery_percent, is_charging, power_source) = parse_battery_metrics(&batt_lines);
    let me_uid = me_lines.get(1).and_then(|s| s.parse::<u32>().ok());
    let processes = parse_process_list(&proc_lines, me_uid);

    HostTelemetrySnapshot {
        hostname,
        os_name,
        cpu_cores,
        load_1m,
        load_5m,
        load_15m,
        memory_used_mb,
        memory_total_mb,
        memory_usage_percent,
        disk_used_gb,
        disk_total_gb,
        disk_usage_percent,
        battery_percent,
        is_charging,
        power_source,
        processes,
    }
}

pub(super) fn parse_load_averages(lines: &[&str]) -> (f64, f64, f64) {
    let Some(first) = lines.first() else {
        return (0.0, 0.0, 0.0);
    };
    // Handles macOS `{ 1.85 1.42 1.19 }` and Linux `1.85 1.42 1.19 2/540 12345`
    let cleaned = first.replace(['{', '}', ','], " ");
    let nums: Vec<f64> = cleaned
        .split_whitespace()
        .filter_map(|tok| tok.parse::<f64>().ok())
        .collect();
    let l1 = nums.first().copied().unwrap_or(0.0);
    let l5 = nums.get(1).copied().unwrap_or(0.0);
    let l15 = nums.get(2).copied().unwrap_or(0.0);
    (l1, l5, l15)
}

pub(super) fn parse_memory_metrics(lines: &[&str]) -> (u64, u64, f64) {
    if lines.is_empty() {
        return (0, 0, 0.0);
    }

    // Check Linux /proc/meminfo format first
    let mut mem_total_kb: Option<u64> = None;
    let mut mem_avail_kb: Option<u64> = None;
    for &line in lines {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            mem_total_kb = rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse::<u64>().ok());
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            mem_avail_kb = rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse::<u64>().ok());
        }
    }

    if let (Some(total_kb), Some(avail_kb)) = (mem_total_kb, mem_avail_kb) {
        let total_mb = total_kb / 1024;
        let used_mb = total_kb.saturating_sub(avail_kb) / 1024;
        let pct = if total_mb > 0 {
            ((used_mb as f64 / total_mb as f64) * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };
        return (used_mb, total_mb, (pct * 10.0).round() / 10.0);
    }

    // macOS: first line is hw.memsize in bytes, followed by vm_stat lines
    let total_bytes = lines
        .first()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let total_mb = total_bytes / (1024 * 1024);

    let mut page_size: u64 = 16384; // Default Apple Silicon page size
    let mut pages_active: u64 = 0;
    let mut pages_wired: u64 = 0;
    let mut pages_compressed: u64 = 0;

    for &line in lines {
        if line.contains("page size of") {
            for tok in line.split_whitespace() {
                if let Ok(ps) = tok.parse::<u64>() {
                    page_size = ps;
                    break;
                }
            }
        } else if let Some(val) = extract_vm_stat_pages(line, "Pages active:") {
            pages_active = val;
        } else if let Some(val) = extract_vm_stat_pages(line, "Pages wired down:") {
            pages_wired = val;
        } else if let Some(val) = extract_vm_stat_pages(line, "Pages occupied by compressor:") {
            pages_compressed = val;
        }
    }

    let used_bytes = (pages_active + pages_wired + pages_compressed) * page_size;
    let used_mb = (used_bytes / (1024 * 1024)).min(if total_mb > 0 { total_mb } else { u64::MAX });
    let pct = if total_mb > 0 {
        ((used_mb as f64 / total_mb as f64) * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };

    (used_mb, total_mb, (pct * 10.0).round() / 10.0)
}

pub(super) fn extract_vm_stat_pages(line: &str, prefix: &str) -> Option<u64> {
    let rest = line.strip_prefix(prefix)?;
    rest.trim().trim_end_matches('.').parse::<u64>().ok()
}

pub(super) fn parse_disk_metrics(lines: &[&str]) -> (f64, f64, f64) {
    let Some(line) = lines.first() else {
        return (0.0, 0.0, 0.0);
    };
    let cols: Vec<&str> = line.split_whitespace().collect();
    if cols.len() < 4 {
        return (0.0, 0.0, 0.0);
    }

    let total_kb = cols.get(1).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
    let avail_kb = cols.get(3).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
    let used_kb = total_kb.saturating_sub(avail_kb);

    let total_gb = ((total_kb as f64) / (1024.0 * 1024.0) * 10.0).round() / 10.0;
    let used_gb = ((used_kb as f64) / (1024.0 * 1024.0) * 10.0).round() / 10.0;
    let pct = if total_kb > 0 {
        (((used_kb as f64) / (total_kb as f64) * 100.0) * 10.0).round() / 10.0
    } else {
        0.0
    };

    (used_gb, total_gb, pct.clamp(0.0, 100.0))
}

pub(super) fn parse_battery_metrics(lines: &[&str]) -> (Option<u8>, Option<bool>, Option<String>) {
    if lines.is_empty() {
        return (None, None, None);
    }

    let mut power_source: Option<String> = None;
    let mut battery_pct: Option<u8> = None;
    let mut is_charging: Option<bool> = None;

    for &line in lines {
        if line.contains("Now drawing from") {
            if line.contains("AC Power") {
                power_source = Some("AC Power".to_string());
            } else if line.contains("Battery Power") {
                power_source = Some("Battery Power".to_string());
            }
        }
        if line.contains('%') {
            for tok in line.split_whitespace() {
                let clean = tok.trim_end_matches(';');
                if let Some(num_str) = clean.strip_suffix('%') {
                    if let Ok(pct) = num_str.parse::<u8>() {
                        battery_pct = Some(pct.min(100));
                        break;
                    }
                }
            }
            let lower = line.to_lowercase();
            if lower.contains("discharging") {
                is_charging = Some(false);
            } else if lower.contains("charging") || lower.contains("charged") {
                is_charging = Some(true);
            }
        }
    }

    (battery_pct, is_charging, power_source)
}

pub(super) fn parse_process_list(lines: &[&str], me_uid: Option<u32>) -> Vec<RemoteProcessInfo> {
    let mut results = Vec::new();

    for &line in lines {
        // Expected format: PID UID USER %CPU %MEM ELAPSED COMMAND
        let mut parts = line.split_whitespace();
        let Some(pid_str) = parts.next() else {
            continue;
        };
        let Ok(pid) = pid_str.parse::<u32>() else {
            continue;
        };
        let Some(uid) = parts.next().and_then(|u| u.parse::<u32>().ok()) else {
            continue;
        };
        let Some(owner) = parts.next().map(str::to_string) else {
            continue;
        };
        let Some(cpu_str) = parts.next() else {
            continue;
        };
        let cpu_percent = cpu_str.replace(',', ".").parse::<f64>().unwrap_or(0.0);

        let Some(mem_str) = parts.next() else {
            continue;
        };
        let mem_percent = mem_str.replace(',', ".").parse::<f64>().unwrap_or(0.0);

        let Some(elapsed_str) = parts.next() else {
            continue;
        };
        let comm_rest: Vec<&str> = parts.collect();
        if comm_rest.is_empty() {
            continue;
        }

        let full_comm = comm_rest.join(" ");
        let truncated_comm: String = full_comm.chars().take(240).collect();
        let short_name = truncated_comm
            .rsplit('/')
            .next()
            .unwrap_or(&truncated_comm)
            .to_string();
        let category = classify_process_category(truncated_comm.clone());

        let group = classify_process_group(uid, owner.clone(), truncated_comm.clone(), me_uid);
        results.push(RemoteProcessInfo {
            pid,
            owner,
            group,
            name: short_name,
            command_line: truncated_comm,
            cpu_percent,
            mem_percent,
            elapsed: elapsed_str.to_string(),
            category,
            started_epoch: None,
        });
    }

    sort_processes(&mut results);
    results
}

/// Agents, local LLMs and build tools first, then by CPU; at most 120 rows.
pub(super) fn sort_processes(results: &mut Vec<RemoteProcessInfo>) {
    results.sort_by(|a, b| {
        let priority = |cat: ProcessCategory| -> u8 {
            match cat {
                ProcessCategory::AiAgent => 0,
                ProcessCategory::LocalLlm => 1,
                ProcessCategory::BuildTool => 2,
                ProcessCategory::Runtime => 3,
                ProcessCategory::System => 4,
            }
        };
        priority(a.category)
            .cmp(&priority(b.category))
            .then_with(|| {
                b.cpu_percent
                    .partial_cmp(&a.cpu_percent)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    results.truncate(120);
}
