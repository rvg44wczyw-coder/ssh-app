# Security findings: feature 32 T3, helper `read` and `kill` (2026-10-19)

## 1. Scope

Commit `02b8173` on `main` (clean tree). Reviewed: `crates/host/src/files.rs` (`read`, `fd_path`),
`crates/host/src/procs.rs` (`kill`, `refusal`, `ancestors`, `is_tmux_server`),
`crates/host/src/telemetry.rs` (`proc_info`, `etime_secs`, `started`), `crates/host/src/main.rs`
(`read_reply`, `kill_reply`), `crates/core/src/file_browser.rs` (`build_helper_read_command`,
`parse_helper_read`), `crates/core/src/telemetry.rs` (`build_helper_kill_command`,
`parse_helper_kill`), `crates/core/src/host_api.rs` (`read_remote_file_preview`,
`kill_remote_process`, helper path and fallback).

Applies: file-system access driven by untrusted input (path, links, races), process signalling
(authorisation of the target), subprocess (`ps`), parsing of a host reply, resource exhaustion. Ruled
out: crypto, replay, downgrade, timing (nothing secret is compared), argument injection in the
script (only `u32` / `u64` numbers and the shell-quoted path in `$1` reach it).

Live tests (own local subprocesses, helper binary from the tree, temp trees in the scratchpad;
tmux only on a private socket with `env -u TMUX -u TMUX_PANE`; before and after, the developer's
tmux was pid 35350, `HOME=/Users/ivs`):

- `read`: hard link to a key file; link chain to a key; link to `/dev/zero` and `/dev/null`;
  FIFO; 10 GB sparse file; NUL at byte 9000; ESC / OSC sequences and invalid UTF-8 in the text;
  case variants `.ENV`, `ID_RSA`; `credentials ` with a trailing space; huge, junk and missing
  `--max` / `--path`; a **race** of 12 s: one thread atomically swapped a real directory and a link to
  `.ssh` under the same name (45 666 swaps) while 4 223 `read` calls ran against `<dir>/k`.
- `kill`: private daemonised tmux server (refused); pids 0, 1, `4294967295`, `2147483648`, `-5`, `abc`;
  the helper's parent and grandparent (refused); stale and correct `--started` on a spawned `sleep`;
  `ps -p` behaviour for out-of-range pids.
- Code analysis only: the tmux server running under a parent other than pid 1, a failing `ps` during
  the ancestor walk, sibling SSH channels, pid reuse inside the tolerance, hostile replies in the
  core (covered by its unit tests).

## 2. Findings

### 1. [SandboxEscape] Low: tmux-server detection is only "comm is tmux and parent is pid 1"

Where: `crates/host/src/procs.rs` `is_tmux_server`.

Scenario: a tmux server whose parent is not pid 1 is not recognised and can be stopped from the
process list, ending every session. On macOS an orphaned daemon is re-parented to launchd (pid 1),
and the live test confirmed the refusal for a daemonised private server (`ppid 1`, message
"That is the tmux server"). On Linux with a systemd user manager (a child subreaper) orphans are
re-parented to it, not to pid 1, and a server started under a supervisor (`tmux -D`) also has a
real parent: both slip through. I could not build a foreground server here to demonstrate it, so
this is reasoning, relevant for feature 33 (Linux and WSL2 hosts).

Fix direction: recognise the server by what it is: a `tmux` process that has child processes (the
panes' shells; a client has none), or that listens on a socket named in `$TMUX_TMPDIR`/`-S`; keep
the ppid rule as an extra.

### 2. [SandboxEscape] Low: a failing `ps` during the ancestor walk turns the protection off

Where: `procs.rs` `ancestors` (stops at the first hop where `proc_info` returns `None`).

Scenario: `proc_info` fails when `ps` is slow (4 s limit per hop), killed, or the host is under
process-table pressure. The chain is then `[helper]` or shorter, the `sshd` / login shell that
carries the exec channel is not in it, and a `kill` of that pid is allowed (the app's own list
shows them). Effect: the phone's connection for that call is dropped; reconnect works. I verified
the normal case live (parent and grandparent refused) but did not force a `ps` failure (the tool is
called by absolute path now).

Fix direction: fail closed. If the walk cannot reach pid 1 or a process whose parent is 1 (or hits
the hop limit), refuse with "cannot verify ancestors"; also use `getppid()` for the first hop
instead of `ps`.

### 3. [InputValidation] Low: `pid as i32` relies on `ps` rejecting huge numbers

Where: `procs.rs` `kill` (`libc::kill(pid as i32, sig)`), `main.rs` `kill_reply` (`parse::<u32>`).

Scenario: `4294967295` as `i32` is `-1`, and `kill(-1, SIGTERM)` signals every process the account may
signal (the phone session, tmux, everything). Today the target never gets that far: `proc_info`
returns `None` for it because macOS `ps` answers "process id too large" for all of
`4294967295`, `4294967294`, `2147483648`, `2147483647` (live), and the helper answered "No such
process" for `4294967295` and `2147483648` (live). The protection is a side effect of another
tool's argument parsing (procps and busybox behave differently), not an explicit check.

Fix direction: refuse `pid > i32::MAX as u32` (or any pid above the kernel's `PID_MAX`) before any
lookup, and use `i32::try_from(pid)` instead of the cast; the core's `validate_kill_pid` can do the
same.

### 4. [PathTraversal] Low (same-account only): a hard link to a blocked file reads fine

Where: `files.rs` `read` (the policy is textual, on the canonical path).

Scenario: `ln ~/.ssh/id_ed25519 ~/work/notes.txt`, then `read ~/work/notes.txt` returns the key
(verified live). The account owner can create it (and can read the key anyway); a repository
checkout or another process running as the same account could plant such a link so that the phone
user, browsing the folder, sees and opens it as `notes.txt`. Hard links cannot be told from files by
name.

Fix direction: refuse regular files with `st_nlink > 1` whose other links cannot be inspected, or
compare `(st_dev, st_ino)` against the blocked files that exist (`~/.ssh/*`, `.aws/credentials`,
...) and refuse matches; document that the policy is a safety net for the user's own phone, not a
security boundary against same-account code.

### 5. [InputValidation] Low: binary detection looks only at the first 8 KiB

Where: `files.rs` `read` (`buf[..8192]` for NUL).

Scenario: a file with a NUL at byte 9 000 is reported `binary: false` (verified live), so the
helper sends 9 005 characters including NULs as `text`; the UI shows control characters. No code
runs (the preview is plain text, ESC and OSC bytes are only displayed, verified in the reply), it is a
display problem only.

Fix direction: detect over the whole returned buffer (it is at most 128 KB), or count control
characters.

### 6. [InputValidation] Low: bidi characters stay in the preview's `path` and file name

Where: `crates/core/src/file_browser.rs` `parse_helper_read` (`path` rejects control characters
only; `filename` is cut from it).

Same class as T2 finding 1: a file named `f<U+202E>gnp.txt` is shown reordered in the preview title
and "Insert Path" inserts the raw name. Fix direction: pass `filename` through `display_safe`.

### 7. [Informational] Kill scope and other notes

- The terminal tab's own `sshd` child (a different SSH channel than the exec call) is not an ancestor
  of the helper, so the list can still stop it: the tab drops and reconnects on the next foreground.
  The confirmation dialog is the only guard; consider marking `sshd` processes of the user.
- `--started` is optional in the helper. The app always sends it from the helper's list; if the list
  came from the shell fallback (`started_epoch` empty) the kill goes through the helper without the
  stale-pid check. The reuse window is also bounded by `ps` elapsed-time granularity: the tolerance
  is 3 s.
- The pid can be reused between `proc_info` and `kill(2)` (milliseconds); same-account only.
- Same-account critical processes (`loginwindow`, `Finder`, user launch agents) can be stopped by
  design, after the app's confirmation.
- `still_running` is true for a zombie; cosmetic.

## 3. Checked and held

- Race: 4 223 `read` calls against 45 666 directory/link swaps under the same path: 0 leaks of the
  secret, 1 070 answered `restricted` (the descriptor's real path, from `F_GETPATH`, was checked and
  refused), 540 read the harmless file. The re-check after `open` is what closes the
  canonicalize-then-open gap.
- Non-regular files: link to `/dev/zero`, `/dev/null` and a FIFO are refused in 0 s with "Not a regular
  file" (the open uses `O_NONBLOCK`, nothing blocks).
- Size and memory: a 10 GB sparse file with `--max 131072` returned in 0.00 s with 4 MB RSS (size
  reported correctly, `truncated: true`); `--max` huge or non-numeric falls back to the default or
  the 128 KB cap.
- Link chains and names: link-to-link to a key refused; `.ENV`, `ID_RSA` refused (case-insensitive);
  `credentials ` (trailing space) is refused as well, since the policy trims the path.
- `kill` refusals (live): pid 0 and 1, the helper's parent and grandparent, a stale `--started` (the
  victim stayed alive), the daemonised tmux server; the correct start time killed a spawned `sleep`
  and the reply said `still_running: false`. Out-of-range, negative and non-numeric pids are refused
  or rejected as arguments.
- Core: only numbers and the quoted path reach the scripts; the reply parsers return `None` for
  absent / older / garbage replies (fallback), show a stated refusal as the error, cap the preview
  text at 140 000 characters, reject paths with control characters and re-validate the reported
  path with the app's own policy.

## 4. Verdict

Findings (highest severity: Low): no Critical/High; six Low items and one informational list.
None blocks T3; findings 2 and 3 are cheap fail-closed fixes and should be done now, finding 1
before the Linux helper (feature 33 H2).

## 5. Resolution (2026-10-19, same day)

- 1: `is_tmux_server` also treats a `tmux` process with child processes (or whose children could
  not be listed) as the server; tests `refusals` (supervised server, unknown children).
- 2: `ancestors` starts from `getppid()` and returns `None` unless the walk reaches the top; `kill`
  then refuses with "Cannot verify which processes carry this SSH connection". Test:
  `kills_a_spawned_sleep_and_refuses_ancestors_and_stale_pids` checks the chain contains the helper.
- 3: pids above `i32::MAX` are refused in the helper (argument parsing and `refusal`) and by the
  core's `validate_kill_pid`; live: `kill --pid 4294967295` / `2147483648` now fail at the argument.
- 4: a file with `st_nlink > 1` is compared (device, inode) against the credential files of the
  home folder (`.ssh/*`, `.aws/*`, `.gnupg/*`, `.kube/*`, the blocked names); live with a fake `HOME`:
  the hard link is refused, an ordinary file next to it reads. Same-account code can still hard-link
  a credential file that is outside those folders; the policy stays a safety net.
- 5: binary detection looks at the whole returned buffer (live: NUL at byte 9 000 → binary).
- 6: the preview's file name goes through `display_safe`.
- 7 stays as documented.

Verdict after fixes: clean for T3.
