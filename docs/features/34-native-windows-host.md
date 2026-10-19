# Feature 34: Native Windows host (no WSL2)

Status: spec, written 2026-10-19 after the owner's "стартуй с виндой". Follows `33-host-os-support.md` §7
(option 1). Android stays excluded. Payments stay off.

## 1. Goal

A Windows 10/11 PC reachable over Tailscale/OpenSSH works as a host without WSL2: persistent tabs
that survive a dropped socket, reattach on foreground, resize, and the agent features (Claude Code,
Codex, ... running in PowerShell or cmd) behave as on a Mac. WSL2 (spec 33) stays supported and is
the better route for agents that only exist for Linux.

## 2. Why it is a spec of its own

tmux is the one thing the whole app rests on: session persistence, `window_change`, `capture-pane`,
`send-keys`, control mode, pane cwd, scrollback, copy mode. There is no native Windows tmux. So the
host needs its own multiplexer, and the core needs a second backend behind the existing tmux layer.

## 3. Design

### 3.1 `ssh-app-mux`: a small ConPTY multiplexer (part of the helper crate)

- Windows build of the helper (`x86_64-pc-windows-msvc`, later `aarch64`), subcommand `mux`.
- A per-user **server** process (started on first use by `mux ensure`, detached, no service) owns the
  sessions. Each session = one ConPTY (`CreatePseudoConsole`) with the user's shell (default:
  `pwsh.exe` if present, else `powershell.exe`, else `cmd.exe`; the choice is queried, never assumed),
  a scrollback ring (default 10 000 lines, a VT-parsed screen model so `capture` returns the visible
  grid and the scrollback as text), size, title, cwd.
- Clients talk to the server over a **named pipe** `\\.\pipe\ssh-app-mux-<SID>` with an ACL that
  allows only the current user's SID (and SYSTEM). The server checks the client token on connect
  (`GetNamedPipeClientProcessId` + token SID compare); anything else is dropped.
- The server exits when it has no session for a grace period. No background work otherwise; sessions
  end when their shell exits or on `kill`.

### 3.2 The command surface mirrors what the core already asks tmux

`mux new/attach/list/kill/resize/send/capture/pane-cwd/rename/has`. `attach` streams raw VT bytes
both ways over the SSH exec channel (stdin/stdout of `ssh-app-host mux attach -t NAME`), like
`tmux attach` does today, so the terminal view (SwiftTerm) is unchanged. `window_change` becomes
`mux resize` through a control byte sequence on the attach stream (a documented escape that
cannot appear in user input: length-prefixed frames, not in-band escapes).

### 3.3 Core: a backend trait over tmux

The tmux functions in `crates/core` (`tmux.rs`, session lifecycle, pane context, capture, send,
transcript detection, file browser pane lookup) go behind a `Multiplexer` trait with two
implementations, `Tmux` (today's code, unchanged) and `Mux`. Selected by the detected host target
(`HostTarget::WindowsX86_64 / WindowsAarch64`). The macOS/Linux/WSL paths must stay byte-for-byte
the same commands (guard with the existing tests).

### 3.4 Host target detection and the shell

`HOST_TARGET_SCRIPT` runs through the login shell today (POSIX). On Windows the default OpenSSH shell
is cmd.exe or PowerShell: detection gets a second probe (`cmd /c ver` / `$PSVersionTable`) whose output
recognises native Windows (today it recognises the failure and shows the WSL2 hint). All Windows
commands are issued as one `ssh-app-host.exe <sub> ...` invocation so no cmd/PowerShell quoting
reaches user data: arguments are passed through a single base64/JSON blob on stdin or as
`CreateProcess` argv built by the sshd, never concatenated into a script. A quoting layer for cmd.exe
is **not** written; if an argument needs it, it goes via stdin.

### 3.5 Helper subcommands on Windows

`telemetry` (GetSystemTimes/GlobalMemoryStatusEx/GetDiskFreeSpaceEx/process snapshot through the
Windows API, battery via `GetSystemPowerStatus`), `ls`/`read` (same policy table plus Windows rows:
`%APPDATA%`, `.ssh`, `NTUSER.DAT`, `SAM`, credential stores; case-insensitive, `\` and `/`, drive
letters, UNC, reparse points, 8.3 names, alternate data streams `:stream`), `kill` (TerminateProcess /
Ctrl-Break; same refusals: own ancestors, other user, the mux server), `hook`/`agent`/`models`/
`usage` (agents' hooks write under `%USERPROFILE%`; the unix socket agent becomes the same named pipe).

### 3.6 Install and pin

The helper is uploaded over SFTP/exec to `%USERPROFILE%\.ssh-app\bin\ssh-app-host.exe` (ACL: user only),
pinned by SHA-256 exactly as now (`Get-FileHash` or `certutil -hashfile` fragment, built-in tools by
absolute path `%SystemRoot%\System32\...`). The embedded Windows blob is Authenticode-unsigned: Windows
Defender SmartScreen does not apply to files written by sshd, but Defender may flag a fresh unsigned
exe. Mitigation and its limits are open question Q3.

## 4. Requirements on the PC

Windows 10 1809+ (ConPTY), OpenSSH Server enabled (`Add-WindowsCapability`), key in
`%ProgramData%\ssh\administrators_authorized_keys` for admins or `%USERPROFILE%\.ssh\authorized_keys`,
Tailscale. The app generates these steps (like WSL2 snippets) for PowerShell.

## 5. Security

- Named pipe ACL and token check (3.1); no TCP listener, no service, no elevated process.
- Same-user model as the Mac helper: protection against version drift and accidents, not against
  code already running as the user.
- Path policy: Windows normalisation is the main risk (§3.5); the shared table gets Windows rows and a
  canonicalisation step (`GetFinalPathNameByHandle` on the opened handle, re-checked after open, as
  `fd_path` does on Unix).
- Escape sequences from the shell reach the phone's terminal as today (SwiftTerm); the mux screen
  model must not execute anything (no OSC 52 paste, title length-capped).
- Test isolation (AGENTS.md): Windows tests never touch the developer's tmux/sshd; the mux server in
  tests uses a unique pipe name.

## 6. Non-goals

Windows Server Core without ConPTY, running the app's sessions as SYSTEM, a GUI on the PC, Windows ARM
until x86_64 works, mouse support beyond what ConPTY forwards.

## 7. Tests and what cannot be verified here

The dev machine is a Mac: no Windows, no wine-grade ConPTY. Verifiable here: all parsers and the VT
screen model as pure Rust with fixtures; the framing protocol; the policy table; cross-compiling the
helper (`cargo xwin` or `zig` with `x86_64-pc-windows-gnu`/`msvc` target) to check it builds. **Not
verifiable here:** ConPTY behaviour, the named pipe ACL, OpenSSH quoting on a real PC, Defender. Each
phase therefore ends with a checklist for the owner to run on a Windows PC (and the work is not
called done before that).

## 8. Phases

| Phase | Content |
|---|---|
| W1 | Cross-compile the helper for Windows; target detection of native Windows; `Multiplexer` trait refactor with the Tmux implementation unchanged (all existing tests green) |
| W2 | `mux` server: VT screen model + scrollback (pure, tested), framing protocol, session registry, command surface; named pipe + ACL (Windows-only code behind `cfg(windows)`) |
| W3 | Core `Mux` backend: new/attach/resize/capture/send/pane-cwd/kill, reattach on foreground |
| W4 | Helper on Windows: telemetry, ls/read policy rows, kill, install and pin |
| W5 | Agent features on Windows: hooks, agent socket as pipe, transcript detection |
| W6 | App UI: Windows setup snippets (OpenSSH, key file, firewall), shell choice, limits notice |
| W7 | Owner device checklist on a real PC; fixes |

Each phase follows the dev-chain with a hacker pass.

## 9. Open questions (defaults in brackets, work proceeds on the defaults)

1. Default shell: PowerShell 7 if installed, else Windows PowerShell [yes; per-tab override later].
2. Scrollback depth and memory per session [10 000 lines, 20 MB cap].
3. Unsigned helper exe vs Defender: ship unsigned and document an exclusion for
   `%USERPROFILE%\.ssh-app\bin`, or the owner provides a code-signing certificate [unsigned, documented].
4. Must `aarch64` Windows (Copilot+ PCs) be included in the first release [no, W1-W7 on x86_64 only].
5. Is a Windows PC available to the owner for W7 [needed; without it the feature stays "implemented,
   unverified"].

## 10. Implementation notes

### W1 (2026-10-19): `Multiplexer` kind, pure mux parts — done, not run on Windows

- `crates/core/src/multiplexer.rs`: `MultiplexerKind { Tmux, WindowsMux }` with `attach_or_create`,
  `detach`, `kill`, `list_detailed`. Tmux delegates to the existing builders (a test compares the
  strings); `WindowsMux` builds `.\.ssh-app\bin\ssh-app-host.exe mux ...`, a relative path from the
  login directory, so cmd.exe and PowerShell need no expansion. Session name goes through
  `validate_session_name`; command and start folder travel as hex of a JSON object (test with
  quote, `&`, `|`, `%`, `^`, `$()` and backticks). `SessionConfig.multiplexer` (default Tmux)
  selects it for the attach command; list/kill still use tmux until W3.
- `crates/mux`: VT screen model (`screen.rs`) and frame codec (`frame.rs`), tested on macOS.
- Changed after review of the design: the attach client runs inside the console that sshd's own
  ConPTY gives the SSH channel, so resize arrives as a console-size change; the `Resize` frame
  stays for the client-to-server hop only.
- Test fix: the process list is chosen by name (`comm`), so the kill test now uses a symlink named
  `claude-victim`; a copied system binary is killed by macOS (code signature).

### W2 (2026-10-19): server logic and Windows glue — logic tested, glue compiled only

- `crates/mux/src/server.rs`: session registry over a `PtyBackend`, one client per session (a new
  attach replaces the old), repaint on attach, resize, `capture`, `Control` lines (`list`, `kill`,
  `detach`, `has`, `capture`), 32-session limit, per-client bounded queue. Tested with a Unix pty
  (19 tests including a fuzzed client, a non-reading client and the limits).
- `Hello` now carries size, command and start folder; `winquote::quote_arg` quotes a command line.
- `crates/winhost` (`ssh-app-host-win.exe`, to be installed as `ssh-app-host.exe`): ConPTY backend
  (PowerShell 7, then Windows PowerShell, then `%ComSpec%`, found on `PATH`), per-user named pipe
  with SID DACL, console client for `attach`, `mux ensure|serve|list|kill|detach|capture`.
  Cross-compiled with `cargo zigbuild --target x86_64-pc-windows-gnu` (1.2 MB) and linted;
  **never run**. The embedded blob is not wired into `make xcframework` yet (W4).
- Hacker pass: `docs/security-findings/2026-10-19-windows-mux-w1-w2.md`.
