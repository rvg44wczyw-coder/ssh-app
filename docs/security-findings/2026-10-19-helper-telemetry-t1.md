# Security findings: feature 32 T1, helper `telemetry` (2026-10-19)

## 1. Scope

Commit `dc98918` on `main` (clean tree at the start). Reviewed:

- `crates/host/src/telemetry.rs` (subcommand `telemetry`, `tool`, `parse_vm_stat_used`, `parse_df`,
  `parse_battery`, `parse_ps`, `snapshot`), `crates/host/src/main.rs` (dispatch, `hello` caps),
  `crates/host/src/proc.rs` (`run_with_input`, 1 MiB cap, timeout).
- `crates/core/src/telemetry.rs` (`build_helper_telemetry_command`, `parse_helper_telemetry`, the
  `Helper*` serde structs, `sort_processes`), `host_api.rs::fetch_host_telemetry`,
  `session.rs::fetch_host_telemetry_once`, `usage.rs::validate_sha256`.

Attack surface that applies: subprocess execution (fixed-argv tools), parsing of untrusted
process-table text (any local user or process controls its own `comm`), a host-to-app JSON reply
that is shown in the UI, pin check of the helper, resource exhaustion.

Ruled out: crypto / keys / replay / downgrade-negotiation / timing (the call is a read-only,
unauthenticated-by-itself exec inside the already authenticated SSH session; the pin is a SHA-256
equality on the host, no secret compared); path traversal (no path input exists in T1); writes (the
helper writes nothing).

Live tests (all on my own local subprocesses, helper binary built from the tree,
`target/debug/ssh-app-host`; hostile tools placed first in `PATH` in a private scratch folder; no
tmux, no sshd, no network): hostile `ps` output (`nan`, `inf`, `1e999`, negative CPU, uid
4294967295, pid overflow, a JSON-looking `comm`, RLO/ZWSP characters, C1 and ESC sequences); every
tool replaced by `sleep 30` (timing); a `ps` writing 200 000 rows plus one 5 MB row (output cap).
Code analysis only: pin and quoting path in the core, serde behaviour on `null`, TOCTOU between the
digest check and the exec.

## 2. Findings

### 1. [InputValidation] Low: a non-finite number in one `ps` row makes the app drop the whole helper reply (silent fallback), negative values pass

Where: `crates/host/src/telemetry.rs` `parse_ps` (`cpu.replace(',', ".").parse::<f64>()`),
`crates/core/src/telemetry.rs` `HelperProc { cpu: f64, mem: f64 }`.

Scenario: Rust's `f64::from_str` accepts `nan`, `inf`, `1e999`. `serde_json` writes non-finite
numbers as `null`. The core then fails to deserialize `null` into `f64`, `parse_helper_telemetry`
returns `None` for the whole reply, and the app silently runs the shell script instead. Negative
values (`-5`) are accepted and shown. Real `ps` never prints these; it takes a replaced `ps` ahead
of the system one in the user's `PATH` (same account), so this is robustness, not a boundary
crossing.

Verified live: fake `ps` with rows `nan`, `inf`, `1e999`, `-5` produced `"cpu": null`,
`"mem": null`, `"cpu": -5.0` in the helper's JSON. The consequence in the core is by code reading
(serde rejects `null` for `f64`), not run.

Fix direction: in the helper keep only finite, non-negative values (`filter(f64::is_finite)`, clamp
to 0..=100·cores); in the core make the numeric fields tolerant (`Option<f64>` with a default) so
one bad row cannot discard the reply.

### 2. [InputValidation] Low: bidirectional and zero-width characters survive in process names (spec §5 says they are removed)

Where: `crates/host/src/telemetry.rs` `clean()` removes only `char::is_control`; the core adds
nothing (`command_line` / `name` / `owner` are shown as-is).

Scenario: any local account or process can start an executable whose path contains U+202E (RLO) or
U+200B. The monitor then shows `/bin/e<RLO>gnp.txt` reordered (reads as `…txt.png`-style
spoofing), so a malicious process can look like an innocent one right where the user decides what
to `kill`. ESC and C1 controls are removed correctly (live: `\x1b]0;x\x07` and U+0085 became plain
text, no terminal sequence reaches the app).

Verified live: the helper emitted `"comm": "/bin/e‮​gnp.txt"` unchanged.

Fix direction: drop Unicode format characters (category Cf: U+200B–U+200F, U+202A–U+202E,
U+2066–U+2069, U+FEFF) in `clean()` and again in the core, or reuse the core's `display_safe`.

### 3. [DoS] Low: the helper's own time budget (7 tools × 4 s, sequential) exceeds the host watchdog (20 s), so one hung tool costs all fields

Where: `crates/host/src/telemetry.rs` `TOOL_TIMEOUT` and `snapshot()`;
`crates/core/src/telemetry.rs` `build_helper_telemetry_command` (`login_sh_script(…, 20)`).

Scenario: if several tools hang (a wedged `pmset` or `ps` is a known macOS behaviour under load, a
shadowed tool in `PATH` does the same on purpose), the helper needs up to 7 × 4 s = 28 s while the
login-shell watchdog kills it at 20 s. The reply is then empty, the app falls back to the shell
script (which has no per-tool timeout of its own and may hang on the same tool), and the user gets
no partial data although load, cores and memory were available.

Verified live: all seven tools replaced by `sleep 30`: the helper returned
`{"load":[…],"me_uid":501,"ok":true}` after 32 s wall time. (Side effect seen only because the fake
tools were shell scripts: the killed script's `sleep` grandchild outlived it; the real tools are
single processes and are killed directly.)

Fix direction: one overall deadline in `snapshot()` (for example 12 s) shared by all tools, and/or
run the tools in parallel threads; print what has been collected when the deadline passes.

### 4. [DoS] Low: more than 1 MiB of `ps` output removes the whole process list while the reply still says `ok`

Where: `crates/host/src/proc.rs` (`take(MAX_OUT)` then the reader stops, the child gets SIGPIPE and
exits non-zero), `crates/host/src/telemetry.rs` `tool()` (`code == Some(0)` required).

Scenario: `kern.maxprocperuid` is 4000 here, `kern.maxproc` 6000. An account that runs a few
thousand processes with long executable paths (≈ 300 bytes per row) pushes `ps` past 1 MiB. `ps`
dies from SIGPIPE, `tool()` returns `None`, `procs` is left out, `ok` stays `true`, so the core does
not fall back and the monitor shows an empty process list: the flood hides itself and every other
process from the monitor (and from the Kill action).

Verified live: a fake `ps` writing 200 000 rows plus one 5 MB row gave a reply with no `procs` key
(0.27 s, 19 MB RSS, so memory and CPU stay bounded; the problem is the silent loss, not a crash).

Fix direction: accept output read up to the cap even when the child exits non-zero, cut it at the
last complete line, and add `"procs_truncated": true`; the app should show that flag. Also have the
core treat a missing `procs` on a macOS host as "unknown", not as "none".

### 5. [Informational] Hardening notes (no boundary is crossed by these)

- `Command::new("ps")`, `sysctl`, `vm_stat`, `df`, `pmset`, `uname`, `hostname` are resolved through
  `PATH`, so a same-account program earlier in `PATH` replaces them (this is how findings 1, 3 and 4
  were driven). The pin protects the helper binary, not what the helper runs. Using absolute paths
  (`/bin/ps`, `/usr/sbin/sysctl`, `/usr/bin/vm_stat`, `/bin/df`, `/usr/bin/pmset`, `/usr/bin/uname`,
  `/bin/hostname`) removes the dependency on the user's `PATH`.
- The digest check and the exec in `build_helper_telemetry_command` are two steps (TOCTOU). Only the
  same account can swap the file (`~/.ssh-app` and `bin` are `drwx------`), and that account already
  owns everything the helper can do; same caveat as feature 24 §7.
- Process category is a substring match on the whole path, and `ALWAYS_KEEP` uses the same match, so
  any local process whose path contains `claude`, `cursor`, `llama`, `mlx` etc. is labelled an AI
  agent and bypasses the 150-row cut (up to 250 extra rows, total cap 400). Display-only, and the
  same rule as the old shell script's `awk` filter; stated here so the UI never treats the label as
  trust.

## 3. Checked and held

- Pin injection: `validate_sha256` (exactly 64 lowercase hex characters) runs in
  `fetch_host_telemetry` and `fetch_host_telemetry_once` before `build_helper_telemetry_command`
  (which is `pub(crate)`) puts the digest between single quotes; no other input reaches the script.
  The wrong-pin case falls back to the shell script (loopback test from the T1 commit).
- Reply spoofing from the process table: a `comm` containing `{"ok":true,"host":"evil","procs":[]}`
  stays a JSON string value (live), newlines cannot be produced because control characters are
  removed, so a process cannot add its own line to the reply. Only a shell profile of the same
  account could print a line before the helper's, which is the same account again.
- Numeric conversions: memory `used` is clamped to `total`, `saturating_*` on the page sums,
  disk `avail > total` saturates to 0 used, percentages are clamped, `uid` 4294967295 parses, a
  pid beyond `u32` drops that row (live).
- Resource use: process list capped at 400 rows and 240 characters per name, 1 MiB per tool, 4 s per
  tool; the helper reads no files, opens no sockets, writes nothing.

## 4. Verdict

Findings (highest severity: Low): no Critical/High; four Low robustness / display-safety items and
three informational notes. None blocks T1; findings 2 and 4 should be fixed before T2 because they
contradict spec §5 (display-safe output) and §6 (the app should not silently lose data).

## 5. Resolution (2026-10-19, same day)

All four findings and the PATH note are fixed; re-verified by tests and by re-running the live
attack with the hostile tools first in `PATH`.

1. Non-finite / negative numbers: the helper keeps only finite values clamped to 0..=100000 (CPU)
   and 0..=100 (memory); the core takes `Option<f64>` and clamps again, so a `null` cannot discard
   the reply. Tests: `non_finite_and_negative_numbers_are_made_safe` (helper),
   `a_reply_without_the_process_list_or_with_null_numbers_is_handled` (core).
2. Bidi / zero-width / separators: the helper drops them (`spoofy`), the core shows any that still
   arrive as a visible `⟦U+XXXX⟧` marker (`display_safe`) on name, command line, owner, elapsed,
   host and OS. Test: `bidi_and_zero_width_are_dropped`, core test above.
3. Time budget: one shared 12 s deadline for all tools (each still at most 4 s), partial answer when
   it runs out. Live re-run with every tool replaced by `sleep 30` in `PATH`: the answer is now the
   real data in about 1 s because tools are called by absolute path (see 5); unit test
   `an_expired_budget_skips_the_tool_and_a_slow_tool_is_cut` covers the deadline itself.
4. Output cap: a tool killed after writing more than 1 MiB keeps its output, cut at the last whole
   line, and the reply carries `procs_truncated`. A reply without `procs` now makes the core fall
   back to the shell script instead of showing an empty list. Test:
   `output_over_the_cap_is_cut_at_a_whole_line_and_flagged` (uses `yes`).
5. Tools are started by absolute path (`/bin/ps`, `/usr/sbin/sysctl`, ...) with the bare name only
   as a fallback when the path does not exist. The TOCTOU between digest check and exec and the
   substring category rule stay as documented (same-account only / display-only).

Verdict after fixes: clean for T1.
