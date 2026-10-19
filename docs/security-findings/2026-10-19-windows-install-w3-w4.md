# Security findings: feature 34 W3/W4, Windows detection and helper install (2026-10-19)

## 1. Scope

`hosttarget.rs` (`WINDOWS_TARGET_SCRIPT`, `parse_windows_target`), `windows_install.rs`,
`session.rs` (`detect_host_target_once`, `ensure_windows_helper_once`), `multiplexer.rs` (helper
path), `wsl.rs` (`windows_setup_snippets`), the Swift flow in `TerminalSessionViewModel`.

Applies: a command line that travels through cmd.exe, a file written on a host from embedded bytes,
a host answer that steers behaviour, text pasted as administrator. Ruled out: crypto, replay, timing.

Live tests (unit tests on macOS; nothing ran on Windows): install commands are fixed text plus hex
(reassembled and compared with the input, every line under 7 000 characters), digests and paths with
shell metacharacters are refused, `certutil` output parsing in both historical formats, probe answers
of a POSIX host, an unexpanded `%PROCESSOR_ARCHITECTURE%`, odd CPUs, hostile keys in the snippets.
**Not verified:** how cmd.exe treats the commands, `certutil -decodehex` behaviour, OpenSSH quoting.

## 2. Findings

### 1. [Downgrade] Informational: a lying host can only pick a build or a UI path
A host that claims Windows gets the Windows build uploaded and the `mux` command lines; one that hides
it gets tmux commands. The digest that is checked comes from the app's own table; a wrong build fails
to run. Nothing the host says reaches a command except the validated session name.

### 2. [InputValidation] Low: install depends on cmd.exe being the SSH shell
With PowerShell as the default shell `if not exist ... mkdir` is a syntax error and `echo ... >` writes
UTF-16. The result is a failed install, never a wrong file: the last step compares the digest and
deletes the file on mismatch. The error text now names the cause and the setup snippets set
`DefaultShell`.

### 3. [Informational] Name derived from 48 bits of the digest
`ssh-app-host-<12 hex>.exe`: the app verifies the full digest with `certutil` before each first use per
app run, not at every start (the Unix pin checks every call). A same-account process could swap the
file between; this is the same-account limit documented for the Mac helper. Hardening idea: have
`attach` re-check its own image hash (the exe can hash itself on start and compare with its name).

### 4. [Informational] Setup text runs as administrator
The snippets contain fixed text and the validated key (alphabet without quote, `$`, backtick, space or
newline), so a key line cannot leave its single quotes.

## 3. Verdict

Findings (highest severity: Low). Clean for W3/W4 as far as macOS can show; the Windows behaviour
is part of the W7 device checklist.
