# Security audit: M16 — host Ollama probe, watchdog, host-controlled output in the UI

- **Date**: 2026-10-16
- **Target**: branch `main`, uncommitted working tree on top of `b800ac6`
- **Auditor**: `hacker` (own loopback sshd, local subprocesses, temp dirs, and a throwaway fake Ollama HTTP server I bound to 127.0.0.1:11434 myself)

## 1. Scope

Reviewed:
- (A) `crates/core/src/hostprobe.rs::watchdog_sh`, `ai.rs` (`build_ollama_probe_command`, `parse_ollama_probe`), `session.rs::check_host_ollama`, `agents.rs::build_binary_probe_command`.
- (B) iOS consumption of host-controlled data: `AiAssistantSheetView` (model chips → `queryHostOllama`), `ServerPickerPopup`, `ActionPanelView` (code reading), plus the Rust `query_host_ollama` path the model name flows into.

Attack classes applied: InputValidation, SandboxEscape (process containment), DoS, MetadataLeak. Ruled out: MITM/Replay/Downgrade/KeyConfusion/Timing/WeakRandomness (no handshake, keys, comparisons or randomness in scope).

Live tests run (harness in the session scratchpad, not in the repo):
1. **Parser bombs** (12 inputs): 2M-deep arrays, 500k-deep objects, 400k-model list (7 MB), duplicate keys, 100 KB number, CR/NUL/bidi/marker-in-name, 1M marker lines, 4 MB single line. No panic; slowest 0.58 s.
2. **Live over a loopback sshd** with `SetEnv SHELL` pointing at a fake shell that puts a fake `ollama` first on `PATH`; fake `ollama` modes: hang, ignore SIGTERM/HUP/INT, endless `version is` lines, 300 MB without newline, `setsid` background child, hostile version line (`$(touch …)`, CR). Measured duration, host RSS (sampled `ps`), client RSS and surviving PIDs.
3. **Fake Ollama server on 127.0.0.1:11434**: hostile model list, a 300 MB body, a 2 MB deep-nested body, and a POST capture to see what a listener on that port receives.
4. **`query_host_ollama` with a hostile model name** (`a'; touch X; echo '$(touch X)`touch X``): canary file check.
5. **Local subprocess runs**: hostile `$SHELL` path (spaces, quotes, `;`, `$()`, backticks), hostile `PATH` entry with a newline, and the watchdog under **`dash`** (the `/bin/sh` of Debian/Ubuntu).
6. **Exec flood** through the plain exec path (4 MB cap).

All started processes were checked afterwards; none left running.

## 2. Findings

### 1. SandboxEscape / DoS — Medium — the watchdog does not contain the probe on `dash` hosts (confirmed)
- **Location**: `hostprobe.rs::watchdog_sh` (`set -m` + `kill -9 -- -$p`).
- **Scenario**: any host whose `/bin/sh` is dash (Debian, Ubuntu, most Linux images). An exec channel has no tty, so dash prints `set: can't access tty; job control turned off`, never creates a separate process group, and dash's `kill` rejects `--` (`kill: Illegal number: -`); `kill -9 -$p` finds no such group. Only the top shell dies; its children (`ollama --version`, a hung rc helper) keep running and keep the probe's stdout pipe open.
- **Verified**: running the real `build_ollama_probe_command()` through dash with a fake `ollama` that sleeps 120 s: **the call returned after 120.6 s** (it should end at 8 s) and the client would wait on the open channel until its own 10 s timeout, leaving the hung program on the host. On macOS (bash as `sh`) the same test is killed correctly.
- **Fix direction**: do not rely on job control. Kill the descendants explicitly (walk `ps -A -o pid= -o ppid=` depth-first, children before parents), which works in dash, bash and BSD/GNU userland.

### 2. InputValidation — Low — marker injection through `echo` escape interpretation (confirmed)
- **Location**: probe script `echo "OLLAMA_TAGS=$t"` (and the other `echo`s). macOS `/bin/sh` is bash in POSIX mode and its `echo` interprets backslash escapes.
- **Scenario**: whatever answers on 127.0.0.1:11434 (or a hostile `ollama`) returns JSON containing the two characters `\n`; `echo` turns them into a real newline, so the text after it starts a new output line. My fake server's body `{"name":"a\nOLLAMA_PATH=/evil"}` produced a parsed `binary_path = "/evil\"},{\"name\":…"`, and the broken `OLLAMA_TAGS` line made `running` false. A newline in a `PATH` directory name does the same (verified: `binary_path = "/evil/ollama"`).
- **Impact**: spoofed `installed`/`binary_path`/`running`; no code execution (every value is validated and none is executed; `binary_path` is not even shown).
- **Fix direction**: print with `printf '%s\n'`; skip a resolved path containing control characters.

### 3. DoS — Low — unbounded host memory in the probe (confirmed)
- **Location**: `ollama --version 2>&1 | grep … | tail -n 1` and `t=$(curl … | tr …)`.
- **Verified**: a fake `ollama` streaming 300 MB without newlines made `grep` reach **778 MB RSS** in 8 s (killed by the watchdog afterwards); a server on 11434 returning 300 MB made the probe shell hold **142 MB** in the `$()` capture within curl's 2 s limit; both scale with the speed of the sender. The client is unaffected (4 MB exec cap held: client RSS 62 → 62 MB under `yes` flood; 10 → 58 MB under the 300 MB body).
- **Fix direction**: bound both with `head -c` (4 KB for the version line, 256 KB for the tags body) before `grep`/`tr`.

### 4. MetadataLeak — Low (pre-existing, newly visible) — any local process on 127.0.0.1:11434 is trusted as "Ollama"
- **Location**: `query_host_ollama` and the new check both talk to `http://127.0.0.1:11434` without identifying the listener.
- **Verified**: with my own listener on the port, `query_host_ollama` delivered the complete prompt, including the terminal context I put in it (`AWS_SECRET=hunter2`), to the listener; the check reported it as running with the models it advertised. The first process to bind an unprivileged port wins, so on a multi-user host another account can squat it.
- **Same-user malware** can already read the terminal, so the realistic exposure is multi-user hosts. A listener-owner check (`lsof`/`ss`) is unreliable without root, so no code fix was suggested here: document it, and recommend not using the assistant on shared hosts (or pinning `OLLAMA_HOST` to a private port).

### 5. SandboxEscape — Info — a `setsid` child outlives the watchdog (confirmed)
- A fake `ollama` that starts `python3 -c "os.setsid(); exec sleep 120"` in the background left a surviving process after the probe. It runs under the user's own account; process-tree walking (finding 1) kills it only while it is still a descendant. Accepted.

### 6. Verified safe (no finding)
- **Model name → shell**: `query_host_ollama` with `a'; touch X; echo '$(touch X)`touch X``: canary absent, the name arrived as a JSON string in the POST body.
- **Parser**: no panic or hang on any bomb; deep nesting rejected in < 1 ms by serde's recursion limit; at most 100 models; names restricted to `[A-Za-z0-9._:/-]`; `‮`, `$(id)`, `a;b`, `../../etc/passwd` and NUL names are dropped (`hostile models kept: []`).
- **Hostile `$SHELL` path** (spaces, quotes, `;`, `$()`, backticks): both probes ran, no injection (canary absent). Hostile `PATH` entries with quotes do not break the quoting.
- **Version line** with `$(touch …)` and CR: ignored, no execution, `version = None`.
- **SIGTERM-ignoring and flooding `ollama` on bash hosts**: killed at 8.5 s, no survivors, host `sh` RSS 2 MB.
- **UI rendering**: model names, server names and hosts are rendered through `Text(String)`/`Text(verbatim:)`/`Label(String, …)`, i.e. never as markdown or links.
- A watchdog `sleep 8` stays alive for up to 8 s after a quick probe (cosmetic).

## 3. Verdict

Findings, highest severity **Medium** (watchdog ineffective on dash hosts). No injection, code execution or client-side resource exhaustion found.

## 4. Remediation (same increment)

| # | Status | Change |
|---|--------|--------|
| 1 | Fixed | `watchdog_sh` no longer uses job control; it kills the descendant tree via `ps -A -o pid= -o ppid=` (children first). Test `watchdog_kills_the_whole_tree_under_dash` runs the real probe under `/bin/dash` with a sleeping grandchild: returns in ~8 s, grandchild dead. |
| 2 | Fixed | Probe prints with `printf '%s\n'`; resolved paths containing non-printable characters (incl. newline) are discarded in the script. Test `escape_sequences_in_host_output_cannot_forge_markers`. |
| 3 | Fixed | `head -c 4096` before the version `grep`, `head -c 262144` before `tr` on the tags body. Test `oversized_version_output_is_bounded_on_the_host`. |
| 4 | Accepted, documented | Port 11434 is trusted as Ollama; avoid the assistant on shared multi-user hosts. |
| 5 | Accepted | `setsid` escapees are out of reach of any tree walk. |

Residual: `ps`-based kill needs `ps -o pid= -o ppid=` (procps, BSD; not stock busybox) — on such a host only the top process is killed and the client-side 10 s timeout still bounds the UI.

CHAIN_STEP step=hacker result=findings severity=Medium doc="docs/security-findings/2026-10-16-toolbar-host-checks-audit.md" (all Medium/Low items fixed; re-run after fixes: 87 unit + 8 hook + 4 loopback + 6 probe tests pass, clippy -D warnings clean)
