# Security findings: feature 34 W1/W2, native Windows multiplexer (2026-10-19)

## 1. Scope

`crates/mux` (`frame.rs`, `screen.rs`, `server.rs`, `winquote.rs`), `crates/core/src/multiplexer.rs`,
`crates/winhost` (`conpty.rs`, `pipe.rs`, `cli.rs`).

Applies: parsing of a byte stream from a peer (frames, escape sequences), command construction for
cmd.exe/PowerShell, resource exhaustion, access control of a local IPC endpoint. Ruled out: crypto,
replay, timing.

Live tests (macOS, own processes, a Unix pty in place of ConPTY, tests in the repository):
200 random byte streams as a client; 200 000 random escape-sequence bytes with resizes into the
screen model; frames fragmented 1 byte at a time, oversize (4 GiB) and malformed lengths; a client
that never reads while the shell floods output; session limit (32); bad session names in `Hello` and
`Control`; hostile command text (`" & | % ^ $() `` '`) through the hex spec. **Not tested (no Windows
here):** everything in `crates/winhost` (compiled and linted for `x86_64-pc-windows-gnu` only): the
pipe DACL and `FIRST_PIPE_INSTANCE`, ConPTY, console modes, OpenSSH quoting, Defender.

## 2. Findings

### 1. [DoS] Medium, fixed: a client that stops reading stalled the whole session
`server.rs`: the output pump wrote to the client while holding the session lock, so a client with a
full pipe blocked the pump and with it `list`, `capture`, `kill` and every other client of that
session. Verified by reading; fixed by a bounded per-client queue with a writer thread: a full queue
drops the client (test `a_client_that_stops_reading_cannot_stall_the_session`: floods the screen,
client is dropped, `list`/`capture`/`kill` keep answering).

### 2. [InputValidation] Low, fixed: attach/kill accepted names the app would refuse
`Control` and `Hello` use the same alphabet as the app (`[A-Za-z0-9_.-]`, 1-64, no leading `-`);
verified with `../x`, `a b`, `a&b`, quotes and `%`.

### 3. [SandboxEscape] Informational (not verified on Windows): pipe access control
Design: name contains the user's SID, DACL `D:P(A;;GA;;;<SID>)`, `PIPE_REJECT_REMOTE_CLIENTS`, first
instance created with `FILE_FLAG_FIRST_PIPE_INSTANCE`, no TCP listener. A process of the same user can
of course use the pipe (same-user model). SYSTEM/Administrators are not in the DACL (they can take
ownership anyway). To verify on a PC: `accesschk`/`icacls` of the pipe from another account.

### 4. [Informational] A session runs the user's own shell with the user's rights
`command` and `start_dir` from the phone run as the logged-in user, like any SSH command. They are
never interpolated into a script: hex of a JSON object in the SSH command, `CreateProcessW` with a
quoted argument (`quote_arg`, unit-tested against the `CommandLineToArgvW` rules) for the shell's
`-Command`/`/k`.

### 5. [Informational] Escape sequences
The screen model never acts on what it parses: OSC 52 and DCS/APC are swallowed, titles are
control-stripped and capped at 128 characters, sequences longer than 256 bytes are dropped, sizes are
bounded (500x300), scrollback is capped (10 000 lines). Output bytes are forwarded to the phone's
terminal unchanged, as with tmux.

## 3. Verdict

Findings (highest severity: Medium, fixed). The logic is clean; the Windows glue is **unverified**
until the owner's device checklist (W7) is done.
