# 21. Files app integration (File Provider)

- **Slug**: `files-provider`
- **Milestone**: M22 (stage 1 read-only, stage 2 write, stage 3 favourites)
- **Roles**: `doc-dev` → `rev` → `rust-core-dev` → `rev` → `swift-bridge-dev` → `rev` → `ios-ui-dev` → `rev` → `hacker`
- **Hacker gate**: **Yes** (network, SSH key in the Keychain, remote commands, files from the host)

## 1. Problem and scope
The user must always be able to pull a file from a host to the phone. Today the file browser only lists
directories and previews up to 128 KB of text; nothing leaves the app. Goal: every saved server shows up
in the system **Files** app as a location; folders open, files download on demand and can be opened,
saved to the phone or shared to any app.

In scope (stage 1): read-only browsing and download of regular files through an `NSFileProviderReplicatedExtension`
(iOS 17 is already the deployment target). Stage 2: upload, create folder, rename, delete.
Stage 3: favourite folders per server (e.g. `~/Terminals`), a "Save to Files" action in the in-app browser.
Out of scope: background sync, offline pinning of whole trees, push-driven change notifications (polling on open only),
folder archives (`tar`) - later.

## 2. Architecture
```
Files app ──► File Provider extension (SSHAppFiles, own process, ~50 MB limit)
                 │  links SshCoreBridge (same Rust core)
                 ├─ reads servers + pinned host keys from the App Group container
                 ├─ reads the SSH key from the shared Keychain access group
                 └─ per request: connect → one exec channel → close
```
- No long-lived socket or process: the system starts the extension on demand, so Zero Battery Drain holds.
- One server = one **domain** (`NSFileProviderDomain`, identifier = server id). The app adds/removes domains when
  servers are added/removed (`NSFileProviderManager.add/remove`) and re-signals on rename.
- Item identifiers are `server-id:` + base64url(absolute remote path); the root is `.rootContainer` = the server's
  start folder (`$HOME` unless a favourite is set). Path is re-validated on every call, never trusted from the system.

## 2a. Review decisions (rev, 2026-10-17)
1. **Not `SshSessionHandle`.** It starts a tmux PTY session on `connect` and its exec path (`exec_raw` → `ExecOutput.text()`) is
   text-only and capped at `MAX_EXEC_OUTPUT`. Files need a new object `RemoteFsClient` (UniFFI) built on
   `transport::connect_authenticated` + a new binary-safe streaming primitive (`exec_stream(command, sink)` /
   `exec_with_stdin` for upload) with no tmux and no PTY. `SessionConfig` is not reused: `RemoteFsConfig {host, port, username,
   private_key_pem, pinned_host_key: String (required)}`.
2. **Replicated extension needs a working set.** Even read-only, `NSFileProviderReplicatedExtension` requires
   `itemVersion`, a working-set enumerator and sync anchors. Design: `itemVersion = mtime_ns:size`; anchor = counter persisted
   in the App Group per domain; `enumerateChanges` re-lists the folders the system has materialized/visited, diffs against a
   small per-domain SQLite/JSON snapshot (path → version) and reports updates/deletes. "Anchor-less full set" is not allowed.
3. **Item identifiers must survive rename.** Path-based ids break the contract on rename/move. Stage 1 (read-only, no local
   rename) keeps `server:base64url(path)`; stage 2 must replace it by stable ids (`dev:inode` from `stat`) with a path table in the
   snapshot, or stay with delete+create semantics - decision recorded before stage 2 starts.
4. **Portable listing.** The host may be macOS (BSD `stat -f`) or Linux (GNU `stat -c`). The listing script detects `stat --version`
   and uses the matching format with `find <dir> -mindepth 1 -maxdepth 1 -print0`-style NUL separation; tests feed both dialects.
5. **Symlink race.** `realpath` then read is not atomic; the read command re-checks `[ -f ]` on the resolved path and the cap
   is enforced by the receiver, not by trust. Residual TOCTOU is accepted (same host user could read the file anyway) and documented.
6. **Blocklist is a safety net, not a boundary.** Credential stores outside the list (`.docker/config.json`, `.config/gh`,
   shell history, browser profiles) stay reachable; the user owns the host, so this is a guard against accidental sharing from
   the Files UI. Extended list: `.docker/config.json`, `.config/gh/hosts.yml`, `.zsh_history`, `.bash_history`, `.gnupg`, `Library/Keychains`.
7. **Connection reuse.** Per-request connect over Tailscale is slow for folders with many requests. The extension keeps one
   connection per domain for an idle window of 15 s, then closes it (still no persistent socket).
8. **Keychain migration safety.** Order: add item to the shared group → read it back and compare → only then delete the
   old item; if anything fails keep the old one and retry next launch. Never delete first.
9. **Locked device.** Items are served only after first unlock (Keychain class). Optional per-server "Require Face ID/passcode
   to open in Files" (LocalAuthentication in the extension) is a stage 3 item.
10. **Errors.** `SshCoreError` gets `FileNotFound`, `NotARegularFile`, `TooLarge`, `Cancelled` variants (typed, mapped in §5).

## 2b. Examples
- Listing: `list_remote_entries("/Users/ivs/Terminals", false)` → `[RemoteEntry{name:"notes.md", kind:File, size:812, modified_unix:1760650000, ...}]`.
- Download: `download_remote_file("/Users/ivs/Terminals/notes.md", "<extension tmp>/notes.md", 2 GiB, progress)` → `TransferResult{bytes:812}`;
  a path under `/.ssh/` → `InvalidInput`; a 3 GiB file → `TooLarge` before any byte is written when `stat` size is known.
- Item id: `server-7:L1VzZXJzL2l2cy9UZXJtaW5hbHM` ↔ `/Users/ivs/Terminals`.

## 3. Rust core contracts (new module `crates/core/src/remote_fs.rs`)
```rust
pub struct RemoteEntry { pub path: String, pub name: String, pub kind: RemoteFileKind,
                         pub size: u64, pub modified_unix: i64, pub mode: u32, pub link_target: Option<String> }
pub struct RemoteStat   { pub entry: RemoteEntry }
// on the new RemoteFsClient (async, typed errors, no unwrap; see 2a.1)
fn stat_remote(path) -> Result<RemoteEntry>;
fn list_remote_entries(path, include_hidden) -> Result<Vec<RemoteEntry>>;           // stat-format, not `ls -la` parsing
fn download_remote_file(path, dest_local_path, max_bytes: u64, progress: Arc<dyn TransferProgress>) -> Result<TransferResult>;
// stage 2
fn upload_remote_file(src_local_path, dest_path, overwrite: bool, progress) -> Result<()>;
fn make_remote_dir(path) / rename_remote(from, to) / delete_remote(path, recursive: false)
```
- Transfer = exec channel streaming (`cat -- PATH` / `dd`-free; upload to a temp name `.<name>.part.<rand>` then `mv`),
  chunked 64 KB, written straight to the local file, never fully in memory. `TransferProgress` is a UniFFI callback trait
  with cancellation (`is_cancelled()`).
- Listing uses a fixed script with `find`-less `stat` format parsed with markers, names sanitised through
  `approval_queue::display_safe` for display but the **raw bytes of a name are kept for the path** only if they contain no
  NUL/newline (such files are hidden in stage 1 and reported as "unsupported name").
- All paths are passed as quoted parameters via `login_sh_script` (existing quoting helpers), never concatenated.
- Same blocklist as the browser (`validate_remote_dir_path` / `validate_remote_file_read_path`: `.ssh`, `.aws`, `.gnupg`,
  `.kube`, private keys, `.env*`, `*.pem`, `*.key`, ...) applied to the **resolved** path (`pwd -P` / `realpath`) so a
  symlink cannot expose a blocked file. Blocked items are not listed; a direct request returns `NSFileProviderError.noSuchItem`
  (`.serverUnreachable` is never used for policy errors).
- Size cap per download (default 2 GiB, setting), disk-space check before start, per-request timeout.

## 4. Shared data (app ↔ extension)
- **App Group** `group.com.semenov.sshapp`: `servers.json` (id, name, host, port, user, key id, pinned host key, start folder).
  The app migrates from `UserDefaults.standard` and keeps writing the shared copy; the extension only reads.
- **Keychain access group** `$(TeamIdentifierPrefix)com.semenov.sshapp.shared` for the key items; accessibility stays
  `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`, non-synchronizable. Existing items are migrated to the group
  (read with the old attributes, write with the group, delete old) on first launch of the new build.
- Entitlements for app and extension: App Group, Keychain sharing. Bundle id of the extension
  `com.semenov.sshapp.files`; both capabilities need to be registered in the developer account (open item, see §9).

## 5. State and lifecycle
- Connection per request: `Connecting → Connected → (op) → Closed`; failures map to `NSFileProviderError`:
  host key mismatch → `.serverUnreachable` plus a notification-less error text "Host key changed - open the app";
  auth failure → `.notAuthenticated`; timeout/offline → `.serverUnreachable`; policy → `.noSuchItem`.
- Host key is **pinned only**: the extension never accepts a new/unknown host key (first-use trust is done in the app);
  an unpinned server is not exposed as a domain.
- Enumeration is on demand; `enumerateChanges` returns a fresh anchor-less full set (stage 1). Materialized files are
  system-managed; the extension keeps no cache beyond the request and removes partial downloads on error/cancel.
- App background/foreground does not affect the extension.

## 6. Security considerations (for `hacker`)
- Private key never leaves the Keychain except into the Rust core memory (zeroized on drop, as now); the extension does
  not log paths, names or errors with host data beyond error kind.
- Remote file names and sizes are untrusted: control/bidi characters stripped for display, length caps, entry count cap
  (e.g. 20 000 per folder, otherwise error "folder too large").
- Downloads are inert: no auto-open, no quarantine bypass; file protection class
  `NSFileProtectionCompleteUntilFirstUserAuthentication` for the provider's storage.
- Stage 2: upload never overwrites silently (`overwrite` must be explicit, system conflict handling used); delete is
  non-recursive in v1 and refuses outside the server's home unless a favourite root is set; rename/move validates both ends.
- Host side is touched only by the logged-in SSH user; no new service or port. Threat: a malicious host returning huge
  listings / endless streams → caps and timeouts above.
- Extension memory limit: streaming only; chunk buffer 64 KB.

## 7. UI (SwiftUI app)
- Settings → Servers: per server toggle "Show in Files" (default on after the host key is pinned), start folder field.
- In-app browser: "Save to Files…" / "Share" on a file (uses the same download into a temp folder then `fileExporter`), progress and cancel.
- Copy explaining that the host must be reachable (Tailscale on) for the Files location to open.

## 8. Verification
Rust unit tests: entry parser (hostile names: newline, NUL, `..`, huge sizes, bidi), identifier encode/decode round trip and
rejection of tampered ids, blocklist on resolved paths, path quoting (`'`, spaces, `$()`, leading `-`), size cap, cancel.
Loopback tests (stage 1) only through `Sshd::tmux()`-style private setup; **no real tmux/HOME**; no `kill-server`.
Swift tests: item id round trip, domain registry diff (add/remove), error mapping to `NSFileProviderError`.
Manual on device: browse, open, save to phone, share, offline error, changed host key, large file cancel, memory under limit
(Instruments). The simulator is not available on the dev machine, so the extension is checked on the iPhone.
Rust coverage target >= 80 % for `remote_fs`.

## 9. Open items
- Register App Group and Keychain sharing for both bundle ids (developer account) and create the extension target
  (hand-edited `project.pbxproj`; ids `AA0000NN`/`BB0000NN` from 55).
- Decide the default per-download cap and whether to show system-wide notifications for failed transfers.
- Verify the Rust XCFramework size/linking works inside an app extension (same binary linked twice, +size).
- Payments (off): file transfer stays free; keep it in mind if limits are enabled later.
