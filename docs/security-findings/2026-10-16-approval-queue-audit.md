# Security audit: Feature 13 — host approval queue, phone signatures, hook `ctl`

- **Date**: 2026-10-16
- **Target**: `main` working tree (Feature 13): `scripts/agent-hooks/ssh-app-hook.py`, `crates/core/src/approval_queue.rs`, `session.rs` approval methods, iOS `ApprovalsCenter` / `ApprovalBannerView`
- **Auditor**: `hacker` (own temp `SSH_APP_HOME` directories, own `ssh-keygen` keys, local subprocesses; nothing outside the sandbox)

## 1. Scope

Attack classes applied: InputValidation, SandboxEscape (file/IPC trust), DoS, Replay, MetadataLeak, KeyConfusion (namespaces). Ruled out: MITM/Downgrade (transport is the existing authenticated SSH session, no negotiation in scope), Timing (verification is done by OpenSSH; no secret comparisons in our code), WeakRandomness (ids/nonces come from `os.urandom`).

Live tests run:
1. `ctl list` against a queue containing: a 500k-deep JSON file, invalid UTF-8, a **FIFO**, and a directory named like an id.
2. The hook against a decisions directory containing a FIFO, a symlink to `/etc/passwd`, and a 10 MB file.
3. Path traversal and newline injection in the `id` of `ctl decide`.
4. Untrusted queue: world-writable `pending/`, and `pending` replaced by a symlink to another directory.
5. Hostile agent fields: NUL, newline, bidi override, lone surrogate, 200 KB `cwd`.
6. Flood: 150 hook runs.
7. **Signature fuzz** against real `ssh-keygen -Y verify`: 120 bit-flipped/truncated signatures, message variants (`true`→`false`, trailing newline/NUL, empty), and a signature made under another namespace.
8. Digest collision search across field boundaries.
9. Rust parser bombs: 200k-deep arrays, 100k-deep objects, 200k garbage entries.

## 2. Findings

### 1. DoS — Low — a FIFO in the queue blocks `ctl list` and the hook (confirmed, fixed)
- **Location**: `read_small` opened files with `O_RDONLY|O_NOFOLLOW` before `fstat`.
- **Attack**: a same-account process creates a FIFO named `<32 hex>.json` in `pending/` (or `<id>.<16 hex>.json` in `decisions/`). `open()` on a FIFO with no writer blocks forever, before the regular-file check. Measured: `ctl list` hung for > 20 s (killed by the harness); a hook would hang past its own deadline until the agent's 130 s timeout, so phone approvals stop working.
- **Fix**: open with `O_NONBLOCK`, then reject non-regular files. Regression test `fifos_in_the_queue_cannot_hang_the_hook_or_ctl`; re-run: `ctl list` returns in 0.07 s and the hook ends at its own deadline.

### 2. InputValidation — Low — ambiguous request digest (confirmed, fixed)
- **Location**: `command_sha256 = SHA-256(agent "\n" tool "\n" cwd "\n" command)`.
- **Attack**: with newlines inside fields, `tool="a\nb", cwd="c"` and `tool="a", cwd="b\nc"` hash identically (confirmed), so a signature could not tell the two displays apart.
- **Fix**: each field is encoded as `<UTF-8 byte length>:<bytes>\n` in the hook and in the core. Tests: Rust unit test for non-collision and byte-length prefixing; the integration tests compare the hook's digest with the core's for a field containing a 3-byte bidi character.

### 3. DoS — Info — stale `.sig-*` temp files after `kill -9`
- A hook killed with SIGKILL during verification leaves a small `0600` file in `run/`. The stale sweep now includes `run/` (regular files only).

### 4. SandboxEscape — Info (documented trade-off) — same-account trust
- A process running as the same user can replace `~/.ssh-app/allowed_signers` with its own key and sign its own approvals. The signature stops forgery by anything that does not control that file (including a hostile file written into the queue), not a compromised account. Recorded in Feature 13 §7 and Feature 11 §4.4.

## 3. Verified secure (no finding)

- **Signatures**: 0 of 120 mutated signatures accepted; the original verifies; a signature made under namespace `file` is rejected both by `-n` and by the `namespaces=` restriction in `allowed_signers`; message variants (`false`, trailing newline, NUL, empty) are rejected. (One run initially reported "accepted" because `ssh-keygen -Y sign` refused to overwrite an existing `.sig`: a harness artefact, confirmed by a clean re-run.)
- **Replay / redirection**: id, nonce, request hash and host timestamp are all signed and the hook uses its own copies; covered by the integration tests (wrong key, flipped `approved`, other request hash, wrong nonce, early/future timestamp, signed-vs-sent timestamp mismatch).
- **Path traversal**: `../../../tmp/x` and an id with a trailing newline are rejected before any path is built; no files created.
- **Untrusted queue**: world-writable `pending/` and a symlinked `pending/` both make the hook give no opinion and create nothing.
- **Hostile files**: deep JSON, invalid UTF-8, directories, symlinks, oversized files are skipped; no crash, 0.07 s.
- **Flood**: the pending directory stops at 100 files.
- **Display spoofing**: bidi/control characters survive in the host files but the core renders them as `⟦U+XXXX⟧`; `cwd` is truncated to 512 characters.
- **Parser bombs** (Rust): empty result, no panic, bounded time.
- **Injection**: no host-supplied string reaches a shell string; the only dynamic command text is the validated 32-hex id and our own JSON/PEM passed as a quoted `printf` argument.

## 4. Verdict

Findings, highest severity **Low**; all fixed in the same increment (re-run: 100 unit, 15 hook-script, 5 loopback sshd, 6 probe tests pass; `clippy -D warnings` clean).

CHAIN_STEP step=hacker result=findings severity=Low doc="docs/security-findings/2026-10-16-approval-queue-audit.md"
