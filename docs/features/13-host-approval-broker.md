# Feature 13: Host Approval Queue & Phone Approvals (replaces the `control.sock` broker of Feature 03)

> **Note (2026-10-26):** parts of this document that describe the Unix-socket approval scheme are superseded by the file queue (`~/.ssh-app/run/{pending,decisions}`, signed by the phone key). See specs 24, 57 and 62.

- **Slug**: `13-host-approval-broker`
- **Milestone**: M17
- **Status**: In implementation
- **Supersedes**: the "local IPC socket / `ssh-app-ctl` daemon" parts of `03-agent-control-and-secure-approvals.md` §3 and the socket protocol of `11-agent-hooks-and-launch-profiles.md` §4. APNs wake-up (Feature 03 §4–5) stays out of scope (see §9).
- **Participating roles**: `doc-dev`, `rev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Hacker gate**: yes (signature verification on the host, file-based IPC, untrusted text shown to the user)

## 1. Purpose

Make the M15 hooks actually ask the phone. A Claude Code `PermissionRequest` / Antigravity `PreToolUse` hook must block until the user taps **Approve** or **Deny** in the iOS app (or a timeout passes), and the host must only honour a decision that was **signed by the phone's Ed25519 key for exactly that request**.

## 2. Design decision: a queue directory, not a daemon

Feature 03 specified a persistent broker owning `control.sock`. That needs a long-running process on the host (start/stop, crash, one more thing to install and audit). A hook process is already alive for exactly as long as a request is pending, so **the hook is the broker for its own request**:

```
 agent ──spawn──> ssh-app-hook ── writes ──> ~/.ssh-app/run/pending/<id>.json   (0600, dir 0700)
                       │                                  ▲
                       │ polls                            │ ssh exec: `ssh-app-hook ctl list`
                       ▼                                  │
   ~/.ssh-app/run/decisions/<id>.json  <── `ctl decide` ── iPhone app (signs with its key)
                       │
        verifies the signature with `ssh-keygen -Y verify` against ~/.ssh-app/allowed_signers
                       ▼
           prints the agent-specific decision JSON
```

Properties: no listening socket of any kind, nothing runs when no approval is pending, the phone speaks only through its normal authenticated SSH session, and the verification lives in the process that releases the agent.

Tradeoffs (stated): polling every 0.5 s while a request is pending (negligible, bounded to 120 s); the pending queue is plain files in the user's account (same trust boundary as the hooks themselves, Feature 11 §4.4); `python3` and OpenSSH `ssh-keygen` (≥ 8.0, for `-Y verify`) are required on the host (both are present wherever sshd + the existing hook already run). If `ssh-keygen -Y verify` is unsupported the hook never accepts a decision (fail to native prompt).

Named tradeoffs: (1) **Foreground-only delivery** is the explicit M17 acceptance boundary: with the app closed nothing reaches the phone and the agent falls back to its native prompt after 120 s; push wake-up is backlog (§9). (2) **`ssh-keygen` instead of an in-script Ed25519 verifier**: a vendored pure-Python verifier (~80 lines) would remove the OpenSSH ≥ 8 dependency but is hand-rolled crypto in the security-critical path; the audited OpenSSH implementation was chosen.

## 3. Host file layout (all under `~/.ssh-app/run`, created `0700`)

| Path | Mode | Written by | Content |
|---|---|---|---|
| `run/` | 0700 | installer / hook | root of the queue |
| `run/pending/<id>.json` | 0600 | hook | one request (§4.1) |
| `run/decisions/<id>.<rand16hex>.json` | 0600 | `ctl decide` | a signed decision (§4.2); several files per id may exist, the hook takes the first valid one |
| `~/.ssh-app/allowed_signers` | 0600 | installer (`install_approval_key`) | `ssh-app <ed25519 public key>` – the only key whose signature releases an agent |

`<id>` is 32 lowercase hex characters (128-bit CSPRNG, `os.urandom`). Every path built from an id is validated against `^[0-9a-f]{32}$` first (no traversal).

## 4. Protocol

### 4.1 Pending request (hook → file)

```json
{"v":1,"id":"<32 hex>","kind":"approval|notify","agent":"claude|antigravity","event":"PermissionRequest",
 "tool":"Bash","command":"git push origin main","cwd":"/Users/me/proj","session":"<agent session id>",
 "ts":1760640000,"nonce":"<32 hex>","command_sha256":"<64 hex>","expires":1760640120}
```

`command_sha256` binds **everything the user is shown**: SHA-256 over `agent`, `tool`, `cwd`, `command` (the stored, already truncated strings), each encoded as `<UTF-8 byte length>:<bytes>\n` so that moving a newline from one field to the next cannot produce the same hash. The field is still called `command_sha256` in the wire format and the canonical payload (Feature 03) for compatibility.

- `command` is truncated to 2000 characters, `tool`/`cwd`/`session` to 512, by the hook; the hash is computed over the strings **exactly as stored**.
- `kind:"notify"` entries are informational (Stop / Notification): the hook writes them and returns immediately; the phone shows them once and acknowledges them (`ctl ack`). Unacknowledged notifications older than 10 minutes are deleted by the next hook run.
- The hook deletes its own `pending/<id>.json` and `decisions/<id>.json` when it exits (also on timeout/signal via `finally`).

### 4.2 Decision (phone → file)

```json
{"v":1,"id":"<32 hex>","approved":true,"ts":1760640007,"signature":"-----BEGIN SSH SIGNATURE-----\n…\n-----END SSH SIGNATURE-----\n"}
```

The signed message is the existing canonical payload (Feature 03 §2.1, unchanged):

`SSH_APP_APPROVAL_V1:{id}:{command_sha256}:{nonce}:{ts}:{approved}` (`approved` is `true`/`false`)

`ts` is **host time** (taken from the `host_time` field of `ctl list`, §4.4), so phone clock skew is irrelevant.

The signature is an OpenSSH **SSHSIG** (namespace `ssh-app-approval`, hash `sha512`), so the host verifies it with stock OpenSSH.

### 4.3 What the hook verifies before honouring a decision

All of these, using **its own** copy of the request (never the values in the decision file):

1. The file name `<id>.<rand>.json` carries the pending id and `decision.id` equals it; `approved` is a JSON boolean.
2. `request.ts <= decision.ts <= hook_now` and `decision.ts - request.ts <= 120` (host clock only; freshness, and a signature is useless for any other request because id/nonce/hash differ).
3. `ssh-keygen -Y verify -f ~/.ssh-app/allowed_signers -I ssh-app -n ssh-app-approval -s <sigfile>` over the canonical message built from the hook's own `id`, `command_sha256`, `nonce`, and the decision's `ts` and `approved`, exits 0. `<sigfile>` is a `0600` file inside `run/`; the message goes on stdin; the call has a 5 s timeout. Argument values are fixed strings or the validated id – no user text in argv. Before verifying, the hook checks that `allowed_signers` is a regular file owned by the euid with no group/other write (mode ≤ 0644); otherwise no opinion.
4. An invalid decision is **ignored**: that file is deleted and the hook keeps waiting (checking every file for its id) until its timeout. The first valid decision wins. A file-writer without the phone key can therefore neither release nor deny a request; the hook just keeps waiting (§7).

Result mapping (unchanged from Feature 11): valid + approved → allow; valid + denied → deny; no valid decision before the timeout, or any error → **no opinion** (Claude: no output; Antigravity: `ask`, which prompts in the terminal). `allow` is only emitted after a verified approval (agy documents `allow` as auto-approve).

### 4.4 `ctl` subcommands (used by the app over an SSH exec channel)

`python3 -I "$HOME/.ssh-app/bin/ssh-app-hook" ctl <list|decide|ack|status>`; the argument vector is constant, **all data goes through stdin** as one JSON line (never interpolated into a shell string).

| Subcommand | stdin | stdout |
|---|---|---|
| `ctl list` | – | `{"host_time":1760640003,"items":[…]}`: pending/notify requests sorted by `ts`, already filtered (valid id, parseable, not expired), at most 20 |
| `ctl decide` | the decision JSON (§4.2), ≤ 16 KB | `{"ok":true}` or `{"ok":false,"error":"…"}`; writes `decisions/<id>.<rand16hex>.json` (`O_EXCL`, `0600`) only if a `pending/<id>.json` of kind `approval` exists and fewer than 5 decision files exist for that id |
| `ctl ack` | `{"id":"<32 hex>"}` | `{"ok":true}`; removes a `notify` entry only |
| `ctl status` | – | `{"ok":true,"ssh_keygen":true|false,"allowed_signers":true|false,"pending":N}` |

`ctl decide` does **not** verify the signature (the hook does); it only refuses malformed input and unknown ids.

Examples (stdout of `ctl list` and `ctl decide`):

```json
{"host_time":1760640003,"items":[{"v":1,"id":"3f…a9","kind":"approval","agent":"claude","tool":"Bash","command":"git push origin main","cwd":"/Users/me/proj","ts":1760640000,"expires":1760640120,"nonce":"…","command_sha256":"…"}]}
{"ok":true}
```

## 5. Rust core (`crates/core`)

New / changed items (UniFFI-exported):

```rust
#[derive(uniffi::Record)] pub struct PendingApproval {
    pub id: String, pub kind: String /* "approval" | "notify" */, pub agent: String, pub event: String,
    pub tool: String, pub command: String, pub cwd: String, pub session: String,
    pub timestamp_sec: u64, pub expires_sec: u64, pub nonce: String, pub command_sha256: String,
}
pub fn sign_approval_sshsig(private_key_openssh: String, message: Vec<u8>) -> Result<String, SshCoreError>;   // SSHSIG PEM
pub fn approval_request_digest(agent: &str, tool: &str, cwd: &str, command: &str) -> String; // SHA-256 over agent\ntool\ncwd\ncommand
// crate-internal (not exported): parse_approval_list (strict, drops malformed or tampered entries),
// build_ctl_command(List|Status) with constant text, build_decide_command / build_ack_command
// (validated id, JSON piped to stdin of `ctl`), allowed_signers_line, build_install_key_command.
#[derive(uniffi::Record)] pub struct ApprovalList { pub host_time_sec: u64, pub items: Vec<PendingApproval> }
#[derive(uniffi::Record)] pub struct ApprovalStatus { pub ssh_keygen: bool, pub allowed_signers: bool, pub pending: u32 }
// on SshSessionHandle:
async fn list_pending_approvals(&self) -> Result<ApprovalList, SshCoreError>;
async fn submit_approval_decision(&self, id: String, approved: bool) -> Result<(), SshCoreError>;
async fn acknowledge_notification(&self, id: String) -> Result<(), SshCoreError>;
async fn install_approval_key(&self) -> Result<(), SshCoreError>;                // writes allowed_signers + creates run dirs
async fn approval_status(&self) -> Result<ApprovalStatus, SshCoreError>;
```

- `submit_approval_decision` first re-lists the pending requests, finds the id, **recomputes the hash of the displayed fields and compares it with `command_sha256`** (guards against a tampered list), builds the canonical message with `ts = host_time` from that same listing, signs with the session's own private key and pipes the decision into `ctl decide`. A request that is gone or expired returns a typed error.
- `install_approval_key` derives the public key from the session key and writes `~/.ssh-app/allowed_signers` (`ssh-app <pubkey>`, atomic write, mode 0600, dirs 0700). It is called by `install_agent_hooks_with_preset` when the preset needs the broker (PhoneApproval), and the script that is written is the new `ssh-app-hook` (§6).
- Parsing is defensive: ids must match `^[0-9a-f]{32}$`; every string is length-capped and stripped of control and bidi-override characters (the same predicate as the Ollama probe); at most 20 entries; malformed entries are dropped, never panic.

## 6. Hook script (`scripts/agent-hooks/ssh-app-hook.py`)

Replaces the Unix-socket client with the queue protocol of §4; keeps the failure policy of Feature 11. Environment overrides: `SSH_APP_HOME` (default `~/.ssh-app`, used by tests) and `SSH_APP_HOOK_TIMEOUT` (default 120). The run directory and its parent must be owned by the euid with no group/other bits, otherwise the hook has no opinion (same trust check as before, now applied to the directories).

Approvals require the default hook directory `~/.ssh-app/bin`: the phone runs `ctl` from that path, so a profile with a custom hook directory installs its hooks but cannot be reached from the phone.

## 7. Security analysis

| Threat | Mitigation |
|---|---|
| Another host user / process writes a fake decision | Dirs are `0700`/files `0600`; and the hook verifies an Ed25519 SSHSIG against `allowed_signers`, so a writer without the phone key cannot release or deny a request. |
| Replay of an old decision to a new request | Signed message contains the unique `id`, `nonce`, and request hash; the hook uses its own values; a decision for another request fails verification. |
| Phone shown a different command than the one signed | The signed hash is the hook's `command_sha256`; the core recomputes the hash of the displayed text before signing. |
| Malicious command text (control / bidi characters, huge) | Capped and sanitised in the core; rendered with `Text(verbatim:)` in a scrolling monospaced box; never interpreted. |
| Argument/shell injection through `ctl` | Fixed argv; data via stdin; ids validated; no shell interpolation of any field. |
| Race / junk decision written first | Several decision files per id are allowed (≤ 5); the hook verifies each, deletes invalid ones and accepts the first **valid** one, so junk cannot block the real decision. |
| Phone shown tool/cwd different from what the hook saw | `agent`, `tool`, `cwd` and `command` are all inside `command_sha256`; the core recomputes it before signing. |
| Flood of pending files (DoS) | `ctl list` returns ≤ 20; the hook refuses to create a request when ≥ 100 files exist (no opinion → native prompt); stale (> 10 min) entries are removed on each run; the decision body is ≤ 16 KB. |
| Phone key compromise | Out of scope: the key already grants SSH access. |
| Compromised agent / same-user code | Out of scope by design (Feature 11 §4.4): the hooks are a safety net, not a sandbox. Named tradeoff: a same-account process can replace `allowed_signers` with its own key and sign its own approvals, so the signature protects against forged files from processes that do not control that file, not against a fully compromised account (a root-owned trust store would be needed). |
| Key reuse | The phone's SSH login key also signs approvals. SSHSIG namespace `ssh-app-approval` separates the domains (an approval signature is never a valid login and vice versa); a dedicated approval key was rejected as a second secret to provision and back up. |

Residual (documented): a same-account process can delete `allowed_signers` or a pending file; effect is "no opinion", never an approval.

## 8. iOS app

- Polling is per **distinct connected server** (several tabs on the same host share one poll; tabs on different hosts each poll their own); approvals are keyed by `(server id, request id)` and shown with the server name.
- `TerminalSessionViewModel` (foreground only): while the scene is active and the session is connected, polls `listPendingApprovals` every 2 s (an exec on the existing SSH connection; **stops in background** to keep the zero-battery rule; a poll also runs immediately on `didBecomeActive`).
- `ApprovalsViewModel` publishes the pending list; a modal card (`ApprovalRequestSheetView`) shows for the first approval: agent, tool, working directory (last 2 path components shown, full on tap), the remaining time from `expires - host_time`, the **full command** in a scrollable monospaced box, remaining seconds, buttons **Approve** (prominent) and **Deny**; a haptic fires when a new request arrives. Notify entries show as a dismissible banner and are acknowledged.
- Approve/Deny call `submit_approval_decision`; errors (expired, gone, host without OpenSSH 8) are shown inline. Result of a signed decision is not "confirmed" until the entry disappears from the next poll.
- Installing/refreshing hooks with the PhoneApproval preset also calls `install_approval_key`; the profile screen shows `approval_status` (ssh-keygen present, key installed).
- Backgrounded app: nothing is delivered (documented; APNs is §9). The agent keeps waiting up to 120 s and then falls back to its native prompt.

## 9. Out of scope / backlog

- APNs wake-up through the zero-knowledge Cloudflare Worker, Notification Service Extension and Live Activities (Feature 03 §4–5).
- Pending-list long poll (SSH exec streaming) instead of 2 s polling.
- Codex / Grok hooks.

## 10. Tests

- **Rust unit**: canonical bytes; `sign_approval_sshsig` round trip verified with a real `ssh-keygen -Y verify` (skipped if absent); `parse_pending_approvals` (valid, malformed id, control/bidi chars, > 20 entries, oversized fields, duplicate ids); `build_ctl_command` is constant.
- **Hook script (Python via `tests/hook_script.rs`)**: with a temp `SSH_APP_HOME`: valid signed approve → allow; valid signed deny → deny; wrong key, tampered `approved`, wrong id, stale ts, future ts → ignored and the hook times out with no opinion; first-valid-wins; permission check (group-readable dir → no opinion); flood guard; cleanup of files on exit.
- **Loopback sshd integration**: install hooks with PhoneApproval → run the hook in the background → `list_pending_approvals` sees it → `submit_approval_decision` → the hook prints the allow JSON; a deny; an expired request gives a typed error.
- **Coverage** ≥ 85 % on the new Rust code.
- **Manual on the iPhone** (the user's live test): launch Claude Code in a tab with the PhoneApproval profile, trigger a Bash command, approve/deny from the app.

## Answered requests do not flash back (2026-10-20)

A poll that was already in flight when the answer was sent still lists the request, so the card
used to disappear and reappear for a moment. The app now remembers the requests it answered for
30 seconds, hides them from the polls and counts them as known in the long poll. After 30 seconds a
request the host still lists is shown again (the answer was not taken). Device check pending.
