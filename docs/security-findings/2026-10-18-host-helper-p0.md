# Review: `ssh-app-host` (feature 24, P0)

Date: 2026-10-18. Live attacks only against the helper binary on a scratch tree under
`/tmp/claude-501` and a throwaway loopback sshd with a private tmux socket.

## 1. Scope
`crates/host` (usage port), `crates/core` (`usage.rs` command builders, `host_api.rs` install and
status, `build.rs` embedding), the app's Host service card. Applied: subprocess/argv construction,
input validation, file-system access driven by untrusted data (transcript folders), DoS, tampering
with the installed helper. Ruled out: MITM/replay/downgrade (no protocol; transport unchanged),
timing, randomness.

## 2. Findings
### 1. [DoS] First port read whole lines and followed folders without a bound. Medium, fixed before release
A transcript line without a newline (300 MB) was loaded into memory (measured 1.27 GB peak) and a
huge directory tree was walked before the time budget started.
Fix: lines over 8 MiB are skipped without being kept (`read_line_capped`), the folder walk honours
the 15 s deadline, child-process output is capped at 16 MiB. Re-run with a FIFO, a 300 MB line and
a symlink loop: 0.77 s, 41 MB peak, valid JSON. Regression: `long_lines_are_skipped_not_loaded`.

## 3. Checked and held
- No shell anywhere in the helper (`ps` and `tmux` with fixed argv); arguments are only config
  folders (count, length and control characters validated, again by the core).
- Files are opened only when `metadata()` says regular (a FIFO cannot block it), never written.
- Install: the digest is checked in Rust, then on the host before `mv`; the file is a mode-700 file in
  a mode-700 folder; a mismatching or replaced helper is reported `Mismatch` and never run; the
  usage call re-checks the digest and falls back to Python (tested: a replaced helper that prints
  `{"evil":1}` is not executed).
- The pin is the embedded build's own digest, not read from the host.
- Chunked upload: base64 text only, 48 KiB per command; no host-supplied value is interpolated.

## 4. Info
- Same-user attacker: can swap the file between the digest check and the `exec`; they can already
  run anything as the user (stated in the spec).
- macOS arm64 only; a different platform gets `Unusable` and the Python probe.

## 5. Verdict
Findings, highest severity **Medium** (item 1, fixed). No open Critical/High.
