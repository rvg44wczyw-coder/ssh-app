# Feature 25: `ssh-app-host hook` / `ctl` — the approval hook in Rust (phase P1 of the host service)

Status: spec (2026-10-18). Part of `docs/features/16-host-service-analysis.md` (owner decisions in
its section 11); follows `24-host-helper.md` (P0: helper binary, pinned-digest install, fallback).

## 1. Goal

`scripts/agent-hooks/ssh-app-hook.py` (≈950 lines) is the security-critical host side of approvals:
queue files, SSHSIG verification, host guard (deny/ask/allow rules with shell peeling), session
registry, status-line wrapper, `ctl` for the app. P1 moves that logic into the typed helper:

- `ssh-app-host hook <agent> <event>` (agent hook on stdin),
- `ssh-app-host ctl <list|wait|decide|ack|status|sessions>` (used by the app over SSH),
- `ssh-app-host statusline [PREV_HEX]` (Claude status-line wrapper).

Behaviour, file formats and the queue protocol are **unchanged**: the Python script is the spec and
the 32 `hook_script` tests are the parity suite (section 5).

Not in P1: stdio rpc, a resident process, event-driven wake-ups (all P2, where the launchd agent
holds the state; the 0.4 s poll inside `ctl wait` stays), push (P3), telemetry port.

## 2. Design

### 2.1 No config migration: the installed file stays the entry point

Agents' settings already contain `python3 -I "<dir>/ssh-app-hook" <agent> <event>`, and the app
recognises its own entries by that shape (`HOOK_MARKER`). Changing the command would force a
rewrite of every agent config on every host. Instead the installed file `~/.ssh-app/bin/ssh-app-hook`
remains a Python file and becomes a **shim in front of the existing logic**:

```python
HELPER_SHA256 = "<digest>"   # substituted by the core at install time; "" in the repo copy
# if HELPER_SHA256 and the helper file ~/.ssh-app/bin/ssh-app-host is a regular, non-link file owned
# by us, not group/other writable, and sha256(contents) == HELPER_SHA256:  os.execv(helper, [helper, "hook"|"ctl"|"statusline", ...])
# otherwise: the unchanged Python implementation runs.
```

- The pin is the digest of the helper embedded in the core (same pin as P0), so a replaced, old or
  tampered helper is never executed; the Python path runs instead.
- The digest is computed in-process (`hashlib`, ~2 MB) and the file is opened once with
  `O_NOFOLLOW`; the bytes hashed are the bytes executed on macOS only if the path is unchanged in
  between, same-user caveat as in 24 section 7 (the same-user attacker already owns the account).
- `agent_hook_script()` returns the script with the pin substituted (`bundled_host_helper_sha256()`;
  empty when no helper is embedded, which disables the shim).
- Dropping `python3` from the command line altogether is a later step (new command shape + migration
  of installed entries + removal of the Python code) and waits for a device run.

### 2.2 Rust modules (`crates/host`)

| Module | Content (ported 1:1 from the script) |
|---|---|
| `queue` | dirs/`private_dir`, `read_small` (no symlink, no FIFO hang, uid check, size cap), `write_new` (0600, O_EXCL), request construction (`clean`, `summarize`, length-prefixed `request_digest`), `submit` with the verified wait, `sweep_stale` |
| `verify` | decision parsing/validation, canonical message `SSH_APP_APPROVAL_V1:...`, `ssh-keygen -Y verify -f allowed_signers -I ssh-app -n ssh-app-approval -s <sigfile>` (fixed argv, 5 s timeout), allowed-signers trust check |
| `guard` | `guard.json` loading (owner + dir checks), rule parsing, tool aliases, `glob_regex`, `shell_segments`, `peel` (wrappers, `sh -c`, `$(...)`/backticks, depth 4), `bash_spec_matches`, `path_spec_matches`, size cap fail-closed |
| `hook` | per-agent payload parsing and reply shapes (claude, codex, qwen, antigravity, crush, opencode, grok), session registry, context/stop messages |
| `ctl` | `status`, `list`, `wait`, `decide`, `ack`, `sessions` |
| `statusline` | limits recording (`run/limits/<sha256(realpath dir)[:16]>.json`) and the previous-command passthrough |

Dependencies added: `sha2`, `hex` or std-only hashing (decided in implementation; a vetted crate is
preferred to home-grown SHA), `shlex` for POSIX word splitting (the script falls back to
`split()` when `shlex` raises; the Rust port does the same when it returns `None`), `libc` for
`geteuid`/`O_NOFOLLOW`/`O_NONBLOCK`. No async runtime, no network.

### 2.3 Failure policy (unchanged, restated)

Any error, absent/untrusted queue, missing `ssh-keygen`, nothing valid in time: the hook has **no
opinion** (exit 0, no decision) and the agent applies its native policy; OpenCode is the documented
exception (blocked when nobody answers). `allow` is printed only after a verified phone approval or
an explicit host-guard allow rule; only a verified deny blocks. A panic must not become a decision:
`panic = "abort"` in the release profile means an aborted helper prints nothing and exits non-zero —
the shim therefore treats a non-zero exit of a **hook** call as "no opinion" only if nothing was
printed; since `os.execv` replaces the process there is no wrapper to do so, so the helper installs
a panic hook that exits 0 silently for `hook` and prints `{"ok":false,...}` for `ctl`.

## 3. Security

- Same trust model and checks as the script (queue dirs private, regular files only, size caps,
  decision age window, per-id decision cap, signature over the request's own copy).
- No new capability: the helper reads/writes only `~/.ssh-app/{run,guard.json,allowed_signers}`,
  the agents' config dirs it already read in P0, and starts `ssh-keygen` and `tmux` (fixed argv,
  no shell) — **except** the status-line passthrough, which runs the user's previous status-line
  command through `sh -c` exactly as the script did (that command is the user's own, hex-encoded
  by the app into the settings line; nothing from the agent payload is interpolated).
- Guard rules can only deny/ask/allow-by-explicit-rule; unparsable rules are skipped; inputs over
  32 KiB are never fed to the splitter and fail closed when rules exist.
- Shim: a tampered helper is not executed (digest pin); a missing helper falls back to Python.

## 4. Failure behaviour

Helper absent/mismatch/crashing before it prints: Python path (absent/mismatch) or no opinion
(crash). The usage strip and approvals therefore never depend on the helper being installed.

## 5. Tests

- Parity: the existing `crates/core/tests/hook_script.rs` suite runs against both implementations.
  `Host::cmd` renders the script into the temp home; with `SSH_APP_TEST_IMPL=rust` the render has the
  pin of a helper built by cargo and the helper installed next to it, otherwise the pin is empty.
  The suite must pass in both modes (run twice in verification).
- A test proves the shim really runs the helper (marker in `status`: helper reports
  `"impl":"rust"`, Python reports `"impl":"python"`).
- Shim safety: wrong pin, replaced helper, symlinked helper, group-writable helper → Python runs.
- Unit tests per module in the host crate for the guard (peel/glob/aliases/oversize) with the cases
  of `host_guard_sees_through_the_cheap_ways_around_a_prefix_rule`.
- Test isolation: tests set `SSH_APP_HOME` to a temp dir, remove `TMUX`/`TMUX_PANE`, never call tmux.

## 6. Exit criteria

Both modes of the suite green; clippy/fmt clean; `ctl status` reports `impl`; app unchanged
(wire format identical); hacker pass on the Rust port (guard bypass fuzz against the Python as
oracle, FIFO/symlink/oversize attacks on the queue).

## 7. Review notes (rev)

- Risk: a second, large implementation of a security-critical component. Accepted because the
  parity suite runs on both and the Python stays as the fallback and the oracle; a differential fuzz
  for the guard (same random commands through both, results compared) is part of the hacker pass.
- The shim costs one Python start (~20 ms) per hook call; the gain in P1 is a tested, typed
  implementation and the base for P2, not speed. Latency drops when the command line stops
  starting Python (later step).
- `ctl wait` keeps polling at 0.4 s inside the helper; event-driven wake-up belongs to the P2 agent.

## 8. Implementation notes (2026-10-18)

Done: `crates/host` modules `queue`, `guard`, `hook`, `ctl`, `sessions`, `statusline`, `pyjson`
(Python-compatible `str()`/`json.dumps` text so the request digest matches), `proc`; subcommands
`hook`, `ctl`, `statusline`; `hello` reports cap `hook`. The script got `HELPER_SHA256` and
`exec_helper`; `agent_hook_script()` substitutes the pin; `ctl status` reports `impl`
(`rust`/`python`). Tests: the hook suite runs on both (`SSH_APP_TEST_IMPL=rust cargo test -p ssh-core
--test hook_script`), `the_shim_runs_only_the_exact_pinned_helper`, `crates/host/tests/guard_parity.rs`
(4000 generated calls, same rule picked by both), `hook_cli.rs`. Security pass:
`docs/security-findings/2026-10-18-host-hook-p1.md`.

Deviations: the spec's separate panic hook became `catch_unwind` plus a silent panic hook (no
`panic = abort`); a hook payload over 32 MiB is refused (deny) instead of being read without a
limit; the helper size cap is 16 MiB (the debug build used by tests is 8.6 MB).
