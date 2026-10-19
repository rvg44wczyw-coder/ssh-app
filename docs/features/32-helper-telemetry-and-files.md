# Feature 32: telemetry and the file browser in `ssh-app-host`

Status: spec (2026-10-19). Follows `24-host-helper.md` (P0), `25-host-hook.md` (P1) and
`26-host-agent.md` (P2); owner decision 16 §11: macOS helper only, pinned by digest, fallback
always kept. Roadmap item "port telemetry and the file browser to the helper (own spec)".

## 1. Why

Both features run shell text today and parse its text back:

| Part | How it works now | Problem |
|---|---|---|
| Host monitor (`telemetry.rs`) | One `sh` script prints `===SECTION===` blocks (`sysctl`, `vm_stat`, `df`, `pmset`, `ps | sort | awk`); `parse_host_telemetry` splits them | Fragile text parsing (`vm_stat` page counts, `pmset` wording), the 150-process cut-off is an `awk` filter, locale-dependent output |
| File list (`file_browser.rs`) | `cd`, `pwd -P`, `LC_ALL=C ls -la`, parsed by columns | Names with newlines, leading spaces or ` -> ` are ambiguous, symlink targets are guessed, a 100k-entry folder is sent whole |
| File preview | `realpath`, then a second exec `head -c` | Two round trips and a window between the checks (a link can be swapped after the sensitive-path check); the deny list exists only in the client |
| Kill | `kill` / `kill -9` of a pid from the list | The pid is checked for `<=1` only |

The helper already gives typed JSON, unit tests and a pinned digest. Moving these two features
there removes the parsing, makes the checks host-side and atomic, and gives the tests real fixtures.

## 2. Scope and non-goals

In: helper subcommands `telemetry`, `ls`, `read`, `kill`; core functions that call them when the
pinned helper is present; app changes limited to feeding the same models.

Out: SFTP / `remote_fs` (Files extension, uploads and downloads) stays as it is; git operations
(`gitops` family, feature 28) stay shell scripts (they need `git`'s own behaviour); Linux hosts.
**The shell paths are not temporary:** the helper is macOS-only, and the telemetry script still has
`/proc` fallbacks for Linux hosts, so those paths stay as the permanent fallback (absent helper,
older helper, digest mismatch, non-macOS host). Nothing is deleted by this feature.

## 3. Helper interface

`hello` reports caps `"telemetry"` and `"fs"` (new helper version; the app embeds it, its SHA-256
becomes the new pin, the user presses "Update on this host" once, as for P1). Every call is the
usual pinned exec (`build_*_command(sha256, ...)` with the digest check and `HELPER_ABSENT`),
arguments are separate `argv` entries, never spliced into a script. Replies are one JSON object on
stdout, `{"ok":true,...}` or `{"ok":false,"error":"..."}`; exit 0 on a handled result.

### 3.1 `ssh-app-host telemetry`

```
{"ok":true,"host":"mac.local","os":"Darwin 25.0.0 arm64","cores":12,
 "load":[1.2,1.5,1.7],
 "mem":{"total":38654705664,"used":24100000000},
 "disk":{"total":994662584320,"used":410000000000},
 "battery":{"percent":87,"charging":true,"source":"AC Power"},   // absent on a desktop
 "me":{"user":"ivs","uid":501},
 "procs":[{"pid":1234,"ppid":1,"uid":501,"user":"ivs","cpu":3.2,"mem":1.1,
           "etime":"1-02:03:04","comm":"claude","args":"claude --resume"}]}
```

- Sources: `getloadavg` and `statvfs` / `sysctlbyname` through `libc` (no `unsafe` beyond the FFI
  call wrappers already used for `getpeereid`); `vm_stat` and `pmset -g batt` and one
  `ps -Aww -o pid=,ppid=,uid=,user=,pcpu=,pmem=,etime=,comm=,args=` run with fixed argv and
  parsed in Rust (the text formats are tested with captured fixtures; spawning keeps the helper
  free of the Mach API).
- Used memory is computed as before (active + wired + compressed pages, 16 KiB page size from
  `vm_stat`'s own header); the unit test pins the formula against a captured `vm_stat`.
- The process list: top 150 by CPU **plus** every process whose command matches the agent / local
  LLM pattern (same list as `proc_filter!`), at most 400 entries, `args` cut at 300 characters and
  made display-safe (control characters removed).
- Classification (`ProcessCategory`, `ProcessGroup`) stays in the core (`classify_*`, already
  tested) and is applied to the helper's raw fields; the helper does not classify.
- Time budget: 5 s; a source that fails leaves its field out and the reply still succeeds.

### 3.2 `ssh-app-host ls (--path ABS | --home | --pane SESSION) [--hidden] [--max N]`

```
{"ok":true,"path":"/Users/ivs/work","parent":"/Users/ivs","truncated":false,
 "entries":[{"name":"src","kind":"dir","size":0,"mtime":1760000000,"mode":493,"hidden":false,
             "link":null,"restricted":false},
            {"name":"notes -> x","kind":"file","size":120,"mtime":..,"mode":420,"hidden":false}]}
```

- Directories first, then names case-insensitively; at most 2000 entries (`truncated` says so).
- `kind`: `dir`, `file`, `symlink`, `other` (fifo, socket, device). For a symlink: `link` is the
  raw target (lossy UTF-8, display-safe) and `target_kind` is what `stat` says, `broken` when it
  does not resolve. The names are real bytes of the file system: no parsing, so newlines and the
  text ` -> ` in a name are unambiguous (the app shows control characters escaped).
- `--pane SESSION` asks tmux for `#{pane_current_path}` (same private-`TMUX_TMPDIR` rules as the
  other tmux calls) and falls back to `$HOME`; `SESSION` is validated like everywhere else.
- `path` is the canonical (`realpath`) directory; the policy of §3.5 is applied to it, and entries
  that the policy would refuse to open are returned with `restricted: true` (listed, greyed, not
  enterable) instead of being hidden, matching today's behaviour of refusing on tap.

### 3.3 `ssh-app-host read --path ABS [--max N]`

One call, atomic: canonicalise, apply §3.5 to the canonical path, `open(O_RDONLY|O_NOFOLLOW|
O_NONBLOCK)` on that canonical path, `fstat` the descriptor (must be a regular file), read at most
`max` bytes (default 65536, cap 131072). Reply: `{"ok":true,"path":..,"size":..,"truncated":..,
"binary":..,"text":".."}`; binary detection (NUL in the first 8 KiB) and lossy UTF-8 are done
host-side so a binary file never crosses the wire as text. A path that is a FIFO or device is
refused without being opened for blocking read.

### 3.4 `ssh-app-host kill --pid N [--force]`

Refuses: `pid <= 1`; the helper's own pid and all its ancestors (the SSH session's `sshd` and
shell: killing them would drop the phone mid-call); a process owned by another uid; the **tmux
server** (`comm == tmux` with parent 1: it would end every session; the reply says so and the app
shows it, the terminal remains the way to do it deliberately). Otherwise `SIGTERM`, or `SIGKILL`
with `--force`; the reply says whether the process was signalled and whether it still exists after
200 ms. Matching is by pid **and** the start time passed back from the list (`etime` is not
enough, so the list also carries `started` epoch seconds): a pid recycled since the list was
fetched is refused as `stale`.

### 3.5 Path policy (moved host-side, same rules)

The deny list of `validate_remote_dir_path` / `validate_remote_file_read_path` (credential
directories, key and token files, shell history, `.env*`, `*.pem`, `*.key`) is implemented once in
the helper as `fs_policy` and applied to the **canonical** path and, for symlinks, to the target
chain (every hop), so a harmless-looking link cannot lead into a blocked place. The core keeps its
validators (they still guard the shell fallback and fail fast); a unit test feeds both
implementations the same list of names and requires identical verdicts (differential, like the
guard parity test of P1).

## 4. Core and app

- `crates/core`: `fetch_host_telemetry` and `list_remote_directory` / `read_remote_file_preview` /
  `kill_remote_process` get a helper path next to the existing one: if the app passes the pin
  (`expected_sha256`; new optional parameter on the exported functions, `None` keeps today's
  behaviour) and the helper's `hello` caps contain the capability, the helper is used; any error
  of `HELPER_ABSENT`, an old cap list or a malformed reply falls back to the shell path for that
  call. The records exposed to Swift are unchanged except `RemoteProcessInfo.started` (optional)
  and `RemoteDirectoryEntry.restricted` (default false).
- The capability check is cached per connection (`hello` once), so a normal call is one exec.
- App: pass `bundledHostHelperSha256()` at the call sites that already do so for usage; the host
  service card (feature 24) lists the two new caps. Nothing else changes visibly except that odd
  file names now display correctly and a refused kill says why.

## 5. Security

- Same trust as P0/P1: the helper runs only when its digest equals the app's pin; no new
  capability to execute free-form commands, no write operation except signalling a process.
- Paths come from the user's own taps but are treated as hostile: absolute, no NUL, canonicalised
  host-side, policy on the canonical path, `O_NOFOLLOW` open of that path, regular-file check on
  the descriptor. The old two-exec window (`realpath` then `head`) disappears; a swap of the file
  for another regular file between canonicalisation and open stays possible for the same-uid
  attacker only, who owns the account anyway (same caveat as 24 §7).
- Output is bounded everywhere (2000 entries, 400 processes, 128 KiB preview, 300-character
  fields) and display-safe (control and bidi characters removed) before it reaches the UI.
- Kill safety rules in §3.4 cannot be bypassed from the app: they are checked in the helper.
- Directory listing exposes names the user could already see with `ls`; the policy only adds
  protection (restricted entries are not openable).

## 6. Failure behaviour

Helper absent, old, replaced or crashing: the existing shell implementation runs and the screens
behave as today. A helper that answers `{"ok":false}` for one call (permission, vanished folder)
is shown as that call's error, not retried through the shell (the shell would fail the same way).
Non-macOS hosts never see the helper path.

## 7. Tests

- Unit (host crate): `vm_stat`, `pmset`, `ps` fixtures including odd command lines; memory
  formula; `ps` columns with spaces in `args`; policy table; kill refusals (ancestor chain, pid
  1, other uid, tmux server, stale `started`) against injected process tables.
- Differential: policy verdicts and the sensitive-name list equal to the core validators.
- Loopback (the sshd harness, private HOME, temp trees): `ls` of a folder with a newline name,
  a leading-space name, ` -> ` in a name, unicode, a symlink to a blocked directory, a broken
  link, a FIFO, a folder of 5000 files (truncation); `read` of text, binary, FIFO (must return
  at once), a link to `~/.ssh/id_x` (refused), a file that becomes a symlink between two calls;
  `telemetry` shape on the dev machine (fields present, process cap, agent processes kept);
  `kill` of a spawned `sleep` (works), of the test's own sshd ancestors (refused), stale pid
  (refused). Fallback test: same calls with a mismatched pin return the shell path's answers.
- Test isolation: nothing starts or signals the real tmux or any process that is not spawned by
  the test (AGENTS.md); before/after check of `tmux display -p '#{pid}'` and `show-environment`.
- Swift: compile-time only, as everywhere.

## 8. Phases (each: core + tests + rev + hacker, helper rebuilt, digest re-pinned)

| Phase | Content |
|---|---|
| T1 | `telemetry` (helper + core path + fixtures), process `started` |
| T2 | `ls` and `fs_policy` (+ differential test), core path for the listing |
| T3 | `read` and `kill` (+ race and refusal tests), core paths |
| T4 | App: pin passed, host service card shows caps, device run with the owner; only then (and only for macOS) consider removing nothing — the shell paths stay for other hosts |

Exit criteria per phase: fmt, clippy `-D warnings`, all tests twice (helper present and
mismatched pin), a hacker pass for T2 and T3 (policy bypass by link chains, race, FIFO, oversize),
a loopback run that compares helper and shell answers on a folder with ordinary names.

## 9. Review notes (rev, 2026-10-19)

- Controversial: this adds attack surface to a security-critical binary for features that work.
  Accepted because the new surface is read-only plus one signal call, every input is validated
  host-side, and the current text parsing and two-exec read are the weaker design. The decision
  to *not* delete the shell paths limits the blast radius of a helper bug.
- Controversial: spawning `vm_stat` / `pmset` / `ps` instead of calling Mach APIs keeps `unsafe`
  out but still depends on those tools' text. Their formats are stable and now covered by fixtures
  in one place; revisit only if a macOS release changes them.
- Open question for the owner: the tmux server refusal in §3.4 (default: refuse). If a "kill
  anyway" path is wanted it should be a separate confirmation, not a flag.
- No contradictions found with 24, 25, 26 or 28; the capability list and the pin mechanism are the
  ones already in use.

## 10. Implementation notes

### T1 (2026-10-19): `telemetry` — done in core and helper; not run on a device

- Helper: `crates/host/src/telemetry.rs`, subcommand `telemetry`, cap `"telemetry"` in `hello`.
  `sysctl`, `vm_stat`, `df -k /`, `pmset -g batt`, `ps -Ao …`, `uname`, `hostname` run with a fixed
  argv (no shell, `LC_ALL=C`, 4 s each, 1 MiB output cap); load average via `getloadavg`. The reply
  carries raw values (`mem.total/used` bytes, `disk.total_kb/avail_kb`, `load[3]`, `me_uid`,
  `procs[]`); percentages, GB/MB, process group and category are derived in the core, so the
  classification code stays single-sourced. A source that fails is left out.
- Deviation from §3.1: `args` and `started` are not in the reply yet. The monitor shows the `comm`
  path only (as before); `started` arrives with `kill` in T3.
- Core: `fetch_host_telemetry(helper_sha256: Option<String>)` and
  `fetch_host_telemetry_once(config, helper_sha256)` (UniFFI signature change; bindings regenerated,
  Swift callers pass `bundledHostHelperSha256()`). Any non-usable reply (absent, old, replaced
  helper; `{"ok":false}`; garbage) falls back to the shell script. A malformed pin is an error.
- Tests: 5 helper unit tests (vm_stat formula and page size, df, pmset, ps cut/keep/controls),
  3 core tests (reply → snapshot, fallback replies, pinned command), loopback test compares
  helper and shell answers (hostname, OS, cores, memory total, disk total) and checks that a wrong
  pin still answers via the shell. fmt, clippy `-D warnings` and the whole workspace pass; the
  real tmux server (pid 35350, `HOME=/Users/ivs`) was unchanged by the run.
- Security view: read-only, no user text reaches the helper or any command; output is bounded
  (400 processes, 240-character names, control characters removed). No hacker pass needed for T1
  beyond this; T2 and T3 get one.
- The user must press "Update on this host" once for the new helper to be picked up (digest pin).

### T2 (2026-10-19): `ls` and the path policy — done in core and helper; not run on a device

- Helper: `crates/host/src/files.rs`, subcommand `ls (--path P | --home | --pane S) [--hidden]
  [--max N]`, cap `"fs"`. The folder is canonicalised, the policy runs on the real path, entries
  come from `read_dir` + `symlink_metadata` (names are the file system's own, so `a -> b` and
  leading blanks are exact), links are resolved and the policy is applied to the target;
  `restricted` marks what cannot be opened. Names with control characters or non-UTF-8 are never
  listed. 2000 entries at most, 20000 read at most. A missing folder falls back to `$HOME`
  like the shell version.
- Policy parity: `crates/host/testdata/fs_policy_cases.tsv` is one table read by both the helper
  (`files::dir_policy` / `file_policy`) and the core validators; both must give the table's verdict.
- Core: `list_remote_directory(path, include_hidden, helper_sha256)`; the path is the only user
  text and travels as `$1`; a refusal stated by the helper is returned as an error (not retried
  through the shell); anything else falls back to `ls -la`. New record field
  `RemoteFileEntry.is_restricted`; the browser dims such rows, shows a lock and does not open them.
  Names are shown through `display_safe`, the `path` stays exact.
- Found by the loopback comparison: the shell listing turns `"  lead"` into `"lead"`; the helper
  keeps it.
- Tests: 4 helper unit tests (table, modes, odd names / links / restricted / hidden / refusal,
  truncation and `$HOME` fallback), 4 core tests, loopback comparison with the shell listing and a
  wrong pin. fmt, clippy and the whole workspace pass; the real tmux (pid 35350) was unchanged.
- Hacker pass done: `docs/security-findings/2026-10-19-helper-ls-t2.md` (four Low findings, all
  fixed except the informational ones; 8 s deadline, exact tmux target, hidden entries not counted,
  display-safe paths, truncated notice).

## 11. Other host systems (owner request 2026-10-19)

The owner now wants Windows and Linux hosts as well (decision 16 §11 said macOS only; superseded).
Facts that shape the work:

- **Linux**: tmux is native. The helper is POSIX Rust, so a `*-unknown-linux-musl` build is
  straightforward; the places that assume macOS are `vm_stat` / `pmset` / `sysctl hw.*` in
  `telemetry.rs`, `lsof` in the transcript detection script, `shasum` in the pin check, the
  `~/Library` paths. `/proc` gives load, memory, battery (`/sys/class/power_supply`) and cwd.
- **Windows**: tmux does not run natively on Windows. Realistic routes: (a) WSL2 with OpenSSH
  inside it (everything works as on Linux, including tmux and the helper; the app connects to the
  WSL sshd), (b) Windows' own OpenSSH server with PowerShell/cmd: no tmux, so no persistent
  sessions unless a replacement is used (ConPTY-based multiplexers exist but are not tmux and
  would need their own command layer). Route (a) is the proposal; (b) is a separate, larger
  feature.
- The helper embedded in the app is one arch today; it needs a per-OS/arch table (digest pin per
  target) and `uname`-based selection at install time.

This needs its own spec (proposed number 33) before any code: host OS detection, per-OS helper
build and pin, the Linux telemetry/FS sources, the WSL2 route and what is explicitly unsupported.
