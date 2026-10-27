#[test]
fn kill_replies() {
    assert!(matches!(
        super::parse_helper_kill(r#"{"ok":true,"signalled":true}"#),
        Some(Ok(()))
    ));
    assert!(
        matches!(super::parse_helper_kill(r#"{"ok":false,"error":"That is the tmux server"}"#), Some(Err(e)) if e.contains("tmux"))
    );
    for none in ["HELPER_ABSENT", "", r#"{"error":"usage"}"#] {
        assert!(super::parse_helper_kill(none).is_none(), "{none}");
    }
    let c = super::build_helper_kill_command(&"c".repeat(64), 123, true, Some(1700000000));
    assert!(
        c.contains("kill --pid 123 --force --started 1700000000") && c.contains("HELPER_ABSENT")
    );
}

#[test]
fn helper_reply_becomes_a_snapshot() {
    let reply = r#"noise
{"ok":true,"host":"mac","os":"Darwin 25 arm64","cores":8,"load":[1.5,1.0,0.5],"mem":{"total":17179869184,"used":8589934592},"disk":{"total_kb":1048576,"avail_kb":524288},"battery":{"percent":80,"charging":false,"source":"Battery Power"},"me_uid":501,"procs":[{"pid":10,"uid":501,"user":"u","cpu":1.0,"mem":2.0,"etime":"01:00","comm":"/usr/bin/zsh"},{"pid":11,"uid":501,"user":"u","cpu":0.0,"mem":1.0,"etime":"01:00","comm":"/opt/ollama"},{"pid":12,"uid":0,"user":"root","cpu":9.0,"mem":0.1,"etime":"01:00","comm":"/usr/libexec/x"}]}"#;
    let s = super::parse_helper_telemetry(reply).expect("snapshot");
    assert_eq!((s.hostname.as_str(), s.cpu_cores), ("mac", 8));
    assert_eq!(
        (s.memory_total_mb, s.memory_used_mb, s.memory_usage_percent),
        (16384, 8192, 50.0)
    );
    assert_eq!(
        (s.disk_total_gb, s.disk_used_gb, s.disk_usage_percent),
        (1.0, 0.5, 50.0)
    );
    assert_eq!((s.battery_percent, s.is_charging), (Some(80), Some(false)));
    assert_eq!(s.processes[0].name, "ollama");
    assert_eq!(s.processes[0].category, ProcessCategory::LocalLlm);
    assert_eq!(
        s.processes.last().map(|p| p.group),
        Some(ProcessGroup::System)
    );
}

#[test]
fn a_reply_without_the_process_list_or_with_null_numbers_is_handled() {
    // No `procs`: ask the shell script (None); a null cpu in one row must not drop the reply.
    assert!(super::parse_helper_telemetry(r#"{"ok":true,"host":"m"}"#).is_none());
    let reply = r#"{"ok":true,"procs":[{"pid":3,"uid":501,"user":"u","cpu":null,"mem":-4.0,"etime":"0:1","comm":"/bin/x\u202egnp.txt"}]}"#;
    let s = super::parse_helper_telemetry(reply).expect("snapshot");
    assert_eq!(
        (s.processes[0].cpu_percent, s.processes[0].mem_percent),
        (0.0, 0.0)
    );
    assert!(
        s.processes[0].command_line.contains("U+202E"),
        "{}",
        s.processes[0].command_line
    );
}

#[test]
fn unusable_helper_replies_fall_back() {
    assert!(super::parse_helper_telemetry("HELPER_ABSENT").is_none());
    assert!(super::parse_helper_telemetry("{\"error\":\"usage\"}").is_none());
    assert!(super::parse_helper_telemetry("{\"ok\":false}").is_none());
    assert!(super::parse_helper_telemetry("").is_none());
}

#[test]
fn helper_command_is_pinned_and_fixed() {
    let c = super::build_helper_telemetry_command(&"a".repeat(64));
    assert!(c.contains("HELPER_ABSENT") && c.contains("telemetry") && c.contains(&"a".repeat(64)));
}

#[test]
fn idle_agents_and_llm_servers_survive_the_process_cut() {
    // 400 busy processes, then an idle ollama and an idle claude at the very end.
    let mut input = String::new();
    for i in 0..400 {
        input.push_str(&format!(
            "{} 501 u 5.0 0.1 01:00 /usr/bin/busy{}\n",
            1000 + i,
            i
        ));
    }
    input.push_str("9001 501 u 0.0 0.2 04:54:07 ollama\n");
    input.push_str("9002 501 u 0.0 0.9 01:10:00 /usr/local/bin/claude\n");
    input.push_str("9003 501 u 0.0 0.1 01:10:00 /usr/bin/idle-other\n");
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(proc_filter!())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child.stdin.take().unwrap().write_all(input.as_bytes())?;
            child.wait_with_output()
        })
        .expect("awk filter runs");
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(text.lines().count(), 152, "150 busiest + ollama + claude");
    assert!(text.contains("ollama"));
    assert!(text.contains("claude"));
    assert!(!text.contains("idle-other"));
}

use super::*;

#[test]
fn test_classify_process_category() {
    assert_eq!(
        classify_process_category("/Users/dev/.local/bin/claude".to_string()),
        ProcessCategory::AiAgent
    );
    assert_eq!(
        classify_process_category("/Users/dev/.gemini/antigravity-cli/bin/agy".to_string()),
        ProcessCategory::AiAgent
    );
    assert_eq!(
        classify_process_category("/opt/homebrew/bin/ollama".to_string()),
        ProcessCategory::LocalLlm
    );
    assert_eq!(
        classify_process_category("/Users/dev/.cargo/bin/cargo".to_string()),
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
    assert!(validate_kill_pid(2_147_483_648).is_err() && validate_kill_pid(u32::MAX).is_err());
    assert!(validate_kill_pid(2_147_483_647).is_ok());
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
===ME===
dev
501
===PROC===
 8412  501 dev     85.4  4.2    12:04 /opt/homebrew/bin/ollama
 9102  501 dev     42.0  1.8    03:15 /Users/dev/.cargo/bin/cargo
 7310  501 dev     14.5  2.1  1:05:22 /Users/dev/.local/bin/claude
    1    0 root     0.1  0.1 12:00:00 /sbin/launchd
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
    assert_eq!(snap.processes[0].owner, "dev");
    assert_eq!(snap.processes[0].group, ProcessGroup::User);
    assert_eq!(snap.processes[3].group, ProcessGroup::System);
}

#[test]
fn process_groups_split_system_user_and_other() {
    let g =
        |uid, owner: &str, cmd: &str, me| classify_process_group(uid, owner.into(), cmd.into(), me);
    // The login account's own tools.
    assert_eq!(
        g(501, "dev", "/Users/dev/.local/bin/claude", Some(501)),
        ProcessGroup::User
    );
    assert_eq!(g(501, "dev", "/bin/zsh", Some(501)), ProcessGroup::User);
    // Root, service accounts and OS locations are system, even when the login user runs them.
    assert_eq!(
        g(0, "root", "/sbin/launchd", Some(501)),
        ProcessGroup::System
    );
    assert_eq!(
        g(241, "_windowserver", "/System/Library/x", Some(501)),
        ProcessGroup::System
    );
    assert_eq!(
        g(501, "dev", "/usr/libexec/cfprefsd", Some(501)),
        ProcessGroup::System
    );
    assert_eq!(
        g(1000, "dev", "/usr/lib/systemd/systemd", Some(1000)),
        ProcessGroup::System
    );
    // Someone else's ordinary process, or ownership we cannot tell.
    assert_eq!(
        g(502, "guest", "/usr/bin/vim", Some(501)),
        ProcessGroup::Other
    );
    assert_eq!(g(502, "guest", "/usr/bin/vim", None), ProcessGroup::User);
    // A login account with a low uid (some Linux boxes) is still the user, not the system.
    assert_eq!(
        g(100, "me", "/home/me/bin/tool", Some(100)),
        ProcessGroup::User
    );
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
===ME===
dev
1000
===PROC===
 2048 1000 dev 150.0 12.5 05:10 /usr/local/bin/ollama
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
