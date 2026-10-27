# Security findings: feature 33 H2, Linux helper (2026-10-19)

## 1. Scope

Working tree on top of `0142825`: `crates/host/src/linux.rs` (`/proc`, `/sys`, `statvfs` readers and
parsers), the Linux branches in `telemetry.rs` (`snapshot_linux`, `proc_info`, `has_children`),
`agent.rs` `peer_is_us` (`SO_PEERCRED`), the extended deny list (`files.rs`, core `file_browser.rs`,
shared table `fs_policy_cases.tsv`), the `/proc/<pid>/cwd` branch in the core transcript script.

Applies: parsing of kernel/host text, subprocess-free process inspection, process signalling
(authorisation), file-system policy, resource exhaustion. Ruled out: crypto, replay, downgrade, timing.

Live tests (own containers: `debian:stable-slim`, linux/arm64, helper from `cargo zigbuild`, no
volume except the read-only helper directory; the developer's tmux was never involved, pid 35350 and
`HOME=/Users/dev` before and after): telemetry on a real `/proc`; process named with ESC and U+202E via
`prctl(PR_SET_NAME)`; a 100 KB argument; 600 extra processes; `ls --pane` through a container tmux;
`read`; `kill` of pid 1, of the tmux server (daemonised and foreground `-D` under a supervisor, parent
not 1), of the helper's parent/grandparent, of a root process as an unprivileged user, of a missing pid,
with a stale and a correct `--started`. Code analysis only: `hidepid=2` /proc, pid reuse, x86_64 build
(compiled, not run).

## 2. Findings

### 1. [DoS] Low: unbounded `cmdline` read per process
`linux.rs` `processes` read the whole `/proc/<pid>/cmdline` of every process (an argument list may be
megabytes, there may be thousands of processes). Not exploited to a failure here (100 KB arguments
were fine, 18 KB reply), but memory scales with the host's argument sizes. Fix: read at most 64 KiB.

### 2. [Informational] A process can choose its own name
A process may rename itself (`prctl`, rewriting `argv`): it can look like `tmux: server` (then it is
refused by `kill`, harmless) or contain `claude` (then it stays in the list, harmless). Control and
bidi characters in names are dropped (verified: no such character in 150 listed processes, including
the one named with ESC and U+202E).

### 3. [Informational] Visibility and pid races
With `hidepid` another account's processes are not listed (the list shrinks, `kill` says "No such
process"); a pid can be reused between the lookup and `kill(2)` (the start-time check narrows it to
3 s). Same-account only, as in T3.

## 3. Checked and held (live)

- `kill`: pid 1 refused; the daemonised tmux server and a foreground server whose parent is a shell
  (pid ≠ 1, has children) refused; the helper's parent and grandparent refused ("carries this SSH
  connection"); unprivileged user against a root process refused ("another account"); stale
  `--started` refused; correct one stops the process; missing pid reported.
- Telemetry: valid JSON, same keys as macOS, at most 150 processes for 600 extra ones, control/bidi
  characters absent; `me_uid` correct.
- `ls`/`read` on Linux paths work; the new deny rows (gcloud, gh, docker, password-store, keyrings,
  sudoers, gshadow) are in the shared table that both implementations are tested against.

## 4. Verdict

Findings (highest severity: Low); finding 1 fixed the same day (`read_cmdline`, 64 KiB); 2 and 3
stay as documented. Clean for H2.
