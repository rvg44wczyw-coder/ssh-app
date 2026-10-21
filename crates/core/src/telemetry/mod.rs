use crate::error::SshCoreError;
use crate::hostprobe::SHA256_OF_H;
use crate::transcript::strip_ansi_codes;
use serde::{Deserialize, Serialize};

/// The process list is cut to the 150 busiest, but agents and local-LLM servers are always kept:
/// an idle `ollama` sits far below the CPU cut-off on a busy machine and would never show up.
macro_rules! proc_filter {
    () => {
        "awk 'NR<=150 || tolower($0) ~ /claude|antigravity|agy|codex|aider|cursor|ollama|llama|lmstudio|vllm|mlx/'"
    };
}

/// Static, zero-interpolation remote shell script that gathers host telemetry in a single round-trip.
pub const TELEMETRY_COLLECTION_SCRIPT: &str = concat!(
    "echo '===HOST==='; hostname 2>/dev/null || echo unknown; uname -srm 2>/dev/null || echo unknown; ",
    "echo '===CORES==='; sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo 1; ",
    "echo '===LOAD==='; sysctl -n vm.loadavg 2>/dev/null || cat /proc/loadavg 2>/dev/null || uptime 2>/dev/null || echo '0.0 0.0 0.0'; ",
    "echo '===MEM==='; sysctl -n hw.memsize 2>/dev/null || true; vm_stat 2>/dev/null || cat /proc/meminfo 2>/dev/null || true; ",
    "echo '===DISK==='; df -k / 2>/dev/null | tail -n 1 || true; ",
    "echo '===BATT==='; pmset -g batt 2>/dev/null || true; ",
    "echo '===ME==='; id -un 2>/dev/null; id -u 2>/dev/null; ",
    "echo '===PROC==='; ps -Ao pid=,uid=,user=,pcpu=,pmem=,etime=,comm= 2>/dev/null | sort -k4 -nr | ", proc_filter!(), " || true"
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
pub enum ProcessCategory {
    AiAgent,
    LocalLlm,
    BuildTool,
    Runtime,
    System,
}

/// Who a process belongs to, for the grouped full list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
pub enum ProcessGroup {
    /// Root, service accounts (`_name`, low uids) and OS locations (`/System`, `/usr/libexec`, ...).
    System,
    /// Started by the account the app logs in with.
    User,
    /// Another regular account, or ownership that could not be determined.
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct RemoteProcessInfo {
    pub pid: u32,
    /// Account that owns the process (as `ps` prints it, may be truncated).
    pub owner: String,
    pub group: ProcessGroup,
    pub name: String,
    pub command_line: String,
    pub cpu_percent: f64,
    pub mem_percent: f64,
    pub elapsed: String,
    pub category: ProcessCategory,
    /// Start time (epoch seconds) when the host reported it; sent back with a kill request so a
    /// reused pid is refused.
    pub started_epoch: Option<u64>,
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
    if pid <= 1 || i32::try_from(pid).is_err() {
        return Err(SshCoreError::ChannelError {
            reason: format!("Refusing to signal reserved or invalid PID {pid}"),
        });
    }
    Ok(pid)
}

const SYSTEM_PATHS: [&str; 6] = [
    "/System/",
    "/usr/libexec/",
    "/usr/sbin/",
    "/sbin/",
    "/Library/Apple/",
    "/usr/lib/systemd/",
];

/// `me_uid` is the login account's uid (`None` when the host did not report it: every
/// non-system process then counts as the user's).
#[uniffi::export]
pub fn classify_process_group(
    uid: u32,
    owner: String,
    command: String,
    me_uid: Option<u32>,
) -> ProcessGroup {
    let system_account = uid == 0 || owner.starts_with('_') || (uid < 500 && Some(uid) != me_uid);
    if system_account || SYSTEM_PATHS.iter().any(|p| command.starts_with(p)) {
        ProcessGroup::System
    } else if me_uid.is_none_or(|m| m == uid) {
        ProcessGroup::User
    } else {
        ProcessGroup::Other
    }
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

mod parse;
pub use parse::*;
mod helper;
pub(crate) use helper::*;
#[cfg(test)]
mod tests;
