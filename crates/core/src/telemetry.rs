use crate::error::SshCoreError;
use crate::transcript::strip_ansi_codes;
use serde::{Deserialize, Serialize};

/// Static, zero-interpolation remote shell script that gathers host telemetry in a single round-trip.
pub const TELEMETRY_COLLECTION_SCRIPT: &str = concat!(
    "echo '===HOST==='; hostname 2>/dev/null || echo unknown; uname -srm 2>/dev/null || echo unknown; ",
    "echo '===CORES==='; sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo 1; ",
    "echo '===LOAD==='; sysctl -n vm.loadavg 2>/dev/null || cat /proc/loadavg 2>/dev/null || uptime 2>/dev/null || echo '0.0 0.0 0.0'; ",
    "echo '===MEM==='; sysctl -n hw.memsize 2>/dev/null || true; vm_stat 2>/dev/null || cat /proc/meminfo 2>/dev/null || true; ",
    "echo '===DISK==='; df -k / 2>/dev/null | tail -n 1 || true; ",
    "echo '===BATT==='; pmset -g batt 2>/dev/null || true; ",
    "echo '===PROC==='; ps -Ao pid=,pcpu=,pmem=,etime=,comm= 2>/dev/null | sort -k2 -nr | head -n 35 || true"
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
pub enum ProcessCategory {
    AiAgent,
    LocalLlm,
    BuildTool,
    Runtime,
    System,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct RemoteProcessInfo {
    pub pid: u32,
    pub name: String,
    pub command_line: String,
    pub cpu_percent: f64,
    pub mem_percent: f64,
    pub elapsed: String,
    pub category: ProcessCategory,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct HostTelemetrySnapshot {
    pub hostname: String,
    pub os_name: String,
    pub cpu_cores: u32,
    pub load_1m: f64,
    pub load_5m: f64,
    pub load_15m: f64,
    pub memory_used_mb: u64,
    pub memory_total_mb: u64,
    pub memory_usage_percent: f64,
    pub disk_used_gb: f64,
    pub disk_total_gb: f64,
    pub disk_usage_percent: f64,
    pub battery_percent: Option<u8>,
    pub is_charging: Option<bool>,
    pub power_source: Option<String>,
    pub processes: Vec<RemoteProcessInfo>,
}

/// Validates that a PID is safe to signal (rejects PID 0 and PID 1 `launchd`/`init`).
pub fn validate_kill_pid(pid: u32) -> Result<u32, SshCoreError> {
    if pid <= 1 {
        return Err(SshCoreError::ChannelError {
            reason: format!("Refusing to signal reserved system PID {pid}"),
        });
    }
    Ok(pid)
}

#[uniffi::export]
pub fn classify_process_category(comm: String) -> ProcessCategory {
    let lower = comm.to_lowercase();
    let basename = lower.rsplit('/').next().unwrap_or(&lower);

    if basename.contains("claude")
        || basename.contains("antigravity")
        || basename == "agy"
        || basename.contains("codex")
        || basename.contains("aider")
        || basename.contains("cursor")
    {
        ProcessCategory::AiAgent
    } else if basename.contains("ollama")
        || basename.contains("llama")
        || basename.contains("lmstudio")
        || basename.contains("vllm")
        || basename.contains("mlx")
    {
        ProcessCategory::LocalLlm
    } else if basename == "cargo"
        || basename == "rustc"
        || basename == "rust-analyzer"
        || basename == "xcodebuild"
        || basename == "swift-frontend"
        || basename == "swiftc"
        || basename == "clang"
        || basename == "ld"
        || basename == "gradle"
        || basename == "make"
    {
        ProcessCategory::BuildTool
    } else if basename == "node"
        || basename == "bun"
        || basename == "deno"
        || basename.starts_with("python")
        || basename == "docker"
        || basename == "containerd"
        || basename == "tmux"
        || basename == "zsh"
        || basename == "bash"
    {
        ProcessCategory::Runtime
    } else {
        ProcessCategory::System
    }
}

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
    let processes = parse_process_list(&proc_lines);

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

fn parse_load_averages(lines: &[&str]) -> (f64, f64, f64) {
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

fn parse_memory_metrics(lines: &[&str]) -> (u64, u64, f64) {
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

fn extract_vm_stat_pages(line: &str, prefix: &str) -> Option<u64> {
    let rest = line.strip_prefix(prefix)?;
    rest.trim().trim_end_matches('.').parse::<u64>().ok()
}

fn parse_disk_metrics(lines: &[&str]) -> (f64, f64, f64) {
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

fn parse_battery_metrics(lines: &[&str]) -> (Option<u8>, Option<bool>, Option<String>) {
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

fn parse_process_list(lines: &[&str]) -> Vec<RemoteProcessInfo> {
    let mut results = Vec::new();

    for &line in lines {
        // Expected format: PID %CPU %MEM ELAPSED COMMAND
        let mut parts = line.split_whitespace();
        let Some(pid_str) = parts.next() else {
            continue;
        };
        let Ok(pid) = pid_str.parse::<u32>() else {
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

        results.push(RemoteProcessInfo {
            pid,
            name: short_name,
            command_line: truncated_comm,
            cpu_percent,
            mem_percent,
            elapsed: elapsed_str.to_string(),
            category,
        });
    }

    // Prioritize AI Agents, Local LLMs, and Build Tools at the top, then sort by CPU% descending
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

    results.truncate(25);
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_process_category() {
        assert_eq!(
            classify_process_category("/Users/ivs/.local/bin/claude".to_string()),
            ProcessCategory::AiAgent
        );
        assert_eq!(
            classify_process_category("/Users/ivs/.gemini/antigravity-cli/bin/agy".to_string()),
            ProcessCategory::AiAgent
        );
        assert_eq!(
            classify_process_category("/opt/homebrew/bin/ollama".to_string()),
            ProcessCategory::LocalLlm
        );
        assert_eq!(
            classify_process_category("/Users/ivs/.cargo/bin/cargo".to_string()),
            ProcessCategory::BuildTool
        );
        assert_eq!(
            classify_process_category("/opt/homebrew/bin/tmux".to_string()),
            ProcessCategory::Runtime
        );
        assert_eq!(
            classify_process_category("/sbin/launchd".to_string()),
            ProcessCategory::System
        );
    }

    #[test]
    fn test_validate_kill_pid() {
        assert!(validate_kill_pid(0).is_err());
        assert!(validate_kill_pid(1).is_err());
        assert_eq!(validate_kill_pid(4210).unwrap(), 4210);
    }

    #[test]
    fn test_parse_macos_telemetry_snapshot() {
        let sample = "\
===HOST===
Ivans-MacBook-Pro.local
Darwin 25.0.0 arm64
===CORES===
14
===LOAD===
{ 3.45 2.10 1.88 }
===MEM===
34359738368
Mach Virtual Memory Statistics: (page size of 16384 bytes)
Pages free:                               120000.
Pages active:                             650000.
Pages inactive:                           200000.
Pages speculative:                         10000.
Pages wired down:                         250000.
Pages occupied by compressor:             148576.
===DISK===
/dev/disk3s1s1 971350180 412000000 559350180 43% /
===BATT===
Now drawing from 'Battery Power'
 -InternalBattery-0 (id=1234567)\t84%; discharging; 4:12 remaining present: true
===PROC===
 8412  85.4  4.2    12:04 /opt/homebrew/bin/ollama
 9102  42.0  1.8    03:15 /Users/ivs/.cargo/bin/cargo
 7310  14.5  2.1  1:05:22 /Users/ivs/.local/bin/claude
    1   0.1  0.1 12:00:00 /sbin/launchd
";

        let snap = parse_host_telemetry(sample.to_string());
        assert_eq!(snap.hostname, "Ivans-MacBook-Pro.local");
        assert_eq!(snap.os_name, "Darwin 25.0.0 arm64");
        assert_eq!(snap.cpu_cores, 14);
        assert!((snap.load_1m - 3.45).abs() < 0.01);
        assert!((snap.load_5m - 2.10).abs() < 0.01);
        assert!((snap.load_15m - 1.88).abs() < 0.01);
        assert_eq!(snap.memory_total_mb, 32768);
        assert_eq!(snap.memory_used_mb, 16384);
        assert!((snap.memory_usage_percent - 50.0).abs() < 0.5);
        assert_eq!(snap.battery_percent, Some(84));
        assert_eq!(snap.is_charging, Some(false));
        assert_eq!(snap.power_source.as_deref(), Some("Battery Power"));
        assert_eq!(snap.processes.len(), 4);
        // AI Agent sorted first!
        assert_eq!(snap.processes[0].name, "claude");
        assert_eq!(snap.processes[0].category, ProcessCategory::AiAgent);
        assert_eq!(snap.processes[1].name, "ollama");
        assert_eq!(snap.processes[1].category, ProcessCategory::LocalLlm);
        assert_eq!(snap.processes[2].name, "cargo");
        assert_eq!(snap.processes[2].category, ProcessCategory::BuildTool);
    }

    #[test]
    fn test_parse_linux_telemetry_snapshot() {
        let sample = "\
===HOST===
gpu-workstation
Linux 6.8.0-45-generic x86_64
===CORES===
32
===LOAD===
4.50 3.20 2.10 4/1024 99812
===MEM===
MemTotal:       65536000 kB
MemFree:        12000000 kB
MemAvailable:   32768000 kB
===DISK===
/dev/nvme0n1p2 1048576000 524288000 524288000 50% /
===BATT===
===PROC===
 2048 150.0 12.5 05:10 /usr/local/bin/ollama
";
        let snap = parse_host_telemetry(sample.to_string());
        assert_eq!(snap.hostname, "gpu-workstation");
        assert_eq!(snap.cpu_cores, 32);
        assert_eq!(snap.memory_total_mb, 64000);
        assert_eq!(snap.memory_used_mb, 32000);
        assert!((snap.memory_usage_percent - 50.0).abs() < 0.5);
        assert_eq!(snap.battery_percent, None);
        assert_eq!(snap.processes.len(), 1);
        assert_eq!(snap.processes[0].category, ProcessCategory::LocalLlm);
    }
}
