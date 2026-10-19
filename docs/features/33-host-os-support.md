# Feature 33: Linux and Windows hosts

Status: spec (2026-10-19). Owner request 2026-10-19: "нужно делать саппорт и для виндовз",
"линукс тоже должен быть в списке". Supersedes the "macOS only" decision of
`16-host-service-analysis.md` §11. Related: `24-host-helper.md` (helper, pin), `26-host-agent.md`,
`32-helper-telemetry-and-files.md` §11 (first notes), `AGENTS.md` (host shell and tmux rules).

## 1. Goal

The app connects to a host over SSH, keeps agents alive in `tmux`, and (with the helper) shows
usage, telemetry, files, approvals. Today everything assumes a macOS arm64 host. Target:

| Host | Support level | How |
|---|---|---|
| macOS arm64 | full (today) | native helper |
| macOS x86_64 | full | second helper build (already on the roadmap) |
| Linux x86_64 / aarch64 | full | static (musl) helper, `/proc` sources, tmux from the distro |
| Windows 10/11 with WSL2 | full inside WSL | sshd + tmux + helper run in the WSL2 distro; it is a Linux host with extras (§6) |
| Windows native (OpenSSH, PowerShell) | **not supported by this feature** | tmux does not exist there (§7) |

## 2. What is macOS-specific today (measured, `grep` of `crates/`)

- **The one embedded helper** is `aarch64-apple-darwin` (`scripts/build-xcframework.sh`,
  `crates/core/build.rs`, 2.8 MB). The pin is the SHA-256 of that one blob
  (`bundled_host_helper_sha256`).
- **Shell snippets**: `shasum -a 256` in 9 places (pin checks, install), `lsof` for the pane
  process's cwd / open brain folder (`host_api.rs` transcript detection), `${SHELL:-/bin/zsh}`
  fallback, `ps` flags (BSD `ps -Ao …` works on macOS; procps on Linux takes the same `-A -o` set,
  busybox does not).
- **Helper sources**: `vm_stat`, `pmset -g batt`, `sysctl hw.ncpu/hw.memsize` in
  `crates/host/src/telemetry.rs`; the old shell telemetry script already has `/proc` fallbacks.
- **Policy lists** name macOS paths (`/library/keychains`, `/etc/master.passwd`) and miss Linux
  ones (`~/.local/share/keyrings`, `/etc/sudoers`, `~/.config/gcloud` …).
- Portable already: SSH, tmux commands, `~/.claude` / `~/.gemini` / Ollama file readers, hook
  installation, JSON protocol, `ls`/`read` (std only), the whole agent / rpc code.

## 3. Design

### 3.1 Host target detection

One probe per connection, cached in the server record: `uname -sm` through the login shell, plus
`/proc/version` (contains `microsoft` ⇒ WSL2). Result `HostTarget`:
`DarwinArm64 | DarwinX86_64 | LinuxX86_64 | LinuxAarch64 | WslX86_64 | WslAarch64 | Unsupported(reason)`.
`uname` missing or an error (cmd.exe / PowerShell greeting, `MSYS`/`CYGWIN`/`MINGW` names)
⇒ `Unsupported("Windows without WSL")` with a message that points to §6. Detection is advice for
the UI and for choosing which blob to install; it is never trusted for security (§5).

### 3.2 Several helper builds, one table in the app

`crates/core/embedded/` holds one blob per Unix target:
`ssh-app-host-aarch64-apple-darwin`, `…-x86_64-apple-darwin`, `…-x86_64-unknown-linux-musl`,
`…-aarch64-unknown-linux-musl` (static, no libc dependency, so any distro and WSL work). A generated
table `{target → (bytes, sha256)}` replaces `BUNDLED_HELPER`. API (UniFFI):

- `detect_host_target(handle) -> HostTarget`,
- `bundled_host_helper_sha256(target: HostTarget) -> Option<String>` (the old no-argument function
  stays and means "this Mac's target" for the existing callers until they are moved),
- `install_bundled_host_helper(target)` picks the blob from the table.

Every pinned call keeps the `helper_sha256: Option<String>` parameter; the app computes the pin
from its own table for the cached target. A wrong or missing target ⇒ `None` ⇒ the existing
fallbacks. App size: four blobs of about 3 MB each is +9 MB; acceptable, compressible with `strip`
and `opt-level="z"` if the owner wants it smaller (open question).

### 3.3 Portable shell fragments

One Rust function (`hostprobe::sha256_of_file_expr`) replaces the nine `shasum` uses:
`sha256sum` if present, else `shasum -a 256`, else `openssl dgst -sha256 -r`; output normalised to the
hex digest. `lsof` uses become `readlink /proc/$pid/cwd` on Linux (selected by `uname`, one script
with a branch) and stay `lsof` on macOS. Everything else in the scripts is POSIX `sh`; scripts are
run with `sh` already (login shell → `exec sh -c`), so fish and zsh differences do not matter.

### 3.4 Helper sources per OS

`telemetry.rs` gets a `Platform` trait with two implementations chosen at compile time
(`#[cfg(target_os)]`): macOS as today; Linux reads `/proc/loadavg`, `/proc/meminfo`
(`MemTotal`, `MemAvailable`), `statvfs("/")`, `/sys/class/power_supply/BAT*/{capacity,status}` and
`AC*/online`, core count from `/proc/cpuinfo` or `sched_getaffinity`, processes from `/proc/<pid>/`
(`stat`, `status` for uid, `cmdline`/`comm`) instead of spawning `ps` (no procps dependency, no
busybox differences, no PATH). The wire format of `telemetry` does not change. `ls`, `read`, `kill`
use only `std` / `libc` and are the same on both.
The agent's pane-cwd discovery (today `lsof` in `usage.rs` / core) uses `/proc/<pid>/cwd`.
The policy tables get a per-OS addition block in the shared `fs_policy_cases.tsv`.

## 4. Non-goals

Native Windows hosts (§7), BSDs, Android hosts, Linux hosts without `tmux` (the app says what is
missing and how to install it, it never installs packages), hosts reachable only over a
non-SSH transport, and any change to the iOS side other than the UI in §8.

## 5. Security

- **The pin is still the app's own digest.** The target chosen by detection only selects which of
  the app's blobs to compare against; a host that lies about its OS gets a blob that fails to run or
  a digest the app does not accept, never a downgrade of the check. The host cannot supply a digest.
- Per-target blobs are built by `scripts/build-xcframework.sh` in one place; the table is generated
  at build time, the digests are not edited by hand. A reproducibility note (`SOURCE_DATE_EPOCH`,
  `--locked`) is part of H1.
- Installing a static binary into `~/.ssh-app/bin` of a Linux account is the same trust step as on
  macOS (user-initiated "Update on this host", mode 700, digest checked on the host after upload).
  No `sudo`, no package manager, no system path is touched.
- `/proc` parsing treats every field as hostile (a process controls its `comm` / `cmdline`):
  length caps, control and bidi characters removed, same rules as `telemetry.rs` T1.
- WSL2 adds one trust boundary the app cannot see: the Windows side can read the WSL file system
  and vice versa. The setup guide (§6) says that the SSH key and the helper live inside the distro.

## 6. Windows through WSL2 (the supported Windows route)

The user sets up, once: WSL2 with a distro, `openssh-server` and `tmux` in it, systemd enabled
(`/etc/wsl.conf` `[boot] systemd=true`) so `sshd` starts, a key in `~/.ssh/authorized_keys` inside
the distro, and network reachability from the phone:

- **Tailscale inside the distro** (recommended: the distro then has its own tailnet address and
  port 22 works as on any Linux host; no port forwarding), or
- Windows 11 22H2+ `networkingMode=mirrored` in `.wslconfig` plus a Windows Firewall rule for the
  sshd port, or
- a `netsh interface portproxy` rule (works, but the WSL2 address changes on restart; documented as
  the fallback with that warning).

The app: `WslX86_64/WslAarch64` is a Linux target with `battery` and `power source` hidden (they
describe a VM), a "Windows (WSL2)" badge, and a short in-app checklist when connecting fails or tmux
is missing. A WSL distro that is stopped (idle WSL2 VMs shut down) answers nothing: the app shows
"the host did not answer; WSL2 may have stopped its VM" (no attempt to start it from the phone).

## 7. Native Windows: why it is a separate decision

tmux has no native Windows build. The app's core promises (a session that survives a dropped TCP
socket, reattach, multi-tab isolation, `window_change` handled by tmux) rest on tmux. Options if the
owner still wants native Windows after §6:

1. a ConPTY-based multiplexer on the host (candidates exist but none is a tmux drop-in; the command
   layer, pane-cwd discovery, `capture-pane`, control mode and `send-keys` would all need a second
   implementation): large, its own spec (proposed 34);
2. no persistence: connect with a plain ConPTY shell and accept that a minimised app loses the
   session: breaks the "zero battery drain, resume instantly" requirement, so not recommended;
3. WSL2 only (this spec).

Default: option 3 until the owner decides otherwise.

## 8. App changes (small)

- Server detail / host service card: the detected OS and arch, the helper blob that will be
  installed, state of `tmux` (`command -v tmux` result).
- `Unsupported` target: one clear message and a link to the WSL2 checklist; no helper install button.
- Nothing in the terminal, feed, git or Files screens changes (they only use SSH + tmux + the
  helper's typed replies, which are the same on every target).

## 9. Tests

- **Unit**: target detection from captured `uname -sm` / `/proc/version` strings (macOS, Linux
  glibc/musl, WSL2, Git-Bash/MSYS, PowerShell error text, empty); table lookup; the generated
  digest table against the files on disk; `sha256_of_file_expr` shell text.
- **Fixtures**: captured `/proc/meminfo`, `/proc/loadavg`, `/proc/<pid>/stat` with spaces and
  parentheses in `comm`, `power_supply` trees, in `crates/host/testdata/linux/`; parsers are
  compiled and tested on every OS (not only under `cfg(target_os = "linux")`).
- **Linux integration**: a container is available on the dev machine (`docker`); the loopback
  harness (`crates/core/tests/loopback_sshd.rs`) gets a Linux variant that builds the musl helper,
  starts `sshd` + `tmux` in a container on an ephemeral loopback port and runs the same assertions
  (usage, telemetry, `ls`, `models`, install + pin, fallbacks). It uses its own container and
  tmux socket; it never touches the developer's tmux (AGENTS.md "Test isolation").
- **Parity**: the same JSON shape test for `telemetry` on both platforms (field presence, ranges).
- WSL2 itself cannot be tested here; it is covered by the Linux container run plus unit tests for
  detection, and verified by the owner on a Windows machine.
- Cross-compiling needs `x86_64-apple-darwin`, `aarch64-unknown-linux-musl` and
  `x86_64-unknown-linux-musl` targets and a linker (`cargo-zigbuild` or musl cross toolchain):
  H1 checks what is installed (only `x86_64-unknown-linux-gnu` is today) and documents the setup.

## 10. Phases

| Phase | Content |
|---|---|
| H1 | Target detection, table of blobs and digests, `sha256` fragment, build script for the extra targets (macOS x86_64 first); no behaviour change on the Mac |
| H2 | Linux helper: `/proc` telemetry, pane cwd, Linux policy rows, musl builds, Linux container integration tests, hacker pass |
| H3 | WSL2: detection, UI badge and checklist, docs for the three networking routes |
| H4 | App UI of §8 and the host service card changes |
| H5 | Owner decision on native Windows (§7); only then a spec 34 |

Each phase follows the dev-chain (doc, rev, rust, rev, bridge, rev, ios, rev, hacker). Order
relative to feature 32: T3 (`read`/`kill`) first, because H2 needs a stable helper surface.

## 11. Open questions for the owner

1. App size: four blobs (about +9 MB) acceptable, or size-optimise first? **Answered 2026-10-19: acceptable** (no size work before H1).
2. Linux distros to name as supported (proposal: anything with glibc or musl, OpenSSH, tmux ≥ 3.0). **Answered 2026-10-19: any distro** (static musl helper; the requirements stay OpenSSH and tmux ≥ 3.0).
3. WSL2: is the checklist-in-app enough, or should the app also generate the `.wslconfig` /
   `wsl.conf` snippets for the user to paste? **Answered 2026-10-19: generate them** (H3/H4: the
   app shows ready-to-copy `/etc/wsl.conf` (`[boot] systemd=true`), `.wslconfig` (mirrored
   networking) and the `authorized_keys` / `sshd` / `tmux` commands for the chosen networking route).
4. Native Windows later (§7, option 1) or never? **Answered 2026-10-19: later** (H5 stays open; no work now, spec 34 only when the owner raises it).

## 12. Review notes (rev, 2026-10-19)

- Controversial: shipping four binaries inside an iOS app. The alternative (download the helper from
  a release URL) adds a network trust path and an update channel to secure; the embedded table keeps
  the single "app is the root of trust" property of feature 24. Kept, with the size question open.
- Controversial: `/proc` instead of `ps` on Linux costs more code but removes the procps/busybox
  variance and a PATH dependency (cf. the T1 hacker finding on `PATH`-resolved tools).
- No contradiction found with 24, 26, 28 or 32; `AGENTS.md`'s "dynamic `$SHELL`, login shell"
  rule is kept (scripts still run through the login shell and `sh`).

## 13. Implementation notes

### H1 (2026-10-19): target detection, helper table, portable digest — done; not run on a device

- `crates/core/src/hosttarget.rs`: `HostTarget` (UniFFI enum, `Unsupported { reason }`),
  `parse_host_target(probe_output)`, the probe `uname -sm` + `/proc/version`; Windows shells
  (cmd / PowerShell errors, MSYS / MinGW / Cygwin) give "Windows without WSL2…"; messages go through
  `display_safe`. `SshSessionHandle::detect_host_target()`.
- Helper table: `crates/core/build.rs` embeds `embedded/ssh-app-host-<triple>` for the four triples
  (the legacy `ssh-app-host` is the `aarch64-apple-darwin` entry); new exports
  `bundled_host_helper_sha256_for(target)`, `bundled_host_helper_size_for(target)`;
  `install_bundled_host_helper_for(target)` refuses unsupported targets and builds that are not
  embedded. The old no-argument API is unchanged (macOS arm64).
- `scripts/build-xcframework.sh` builds the other targets with `cargo zigbuild` (installed here:
  `cargo-zigbuild`, `zig 0.17`; `rustup target add` done) and skips what it cannot build.
  Result today: `x86_64-apple-darwin` builds (2.9 MB, embedded); both Linux musl targets fail to
  compile because the helper uses `libc::getpeereid` (agent socket peer check), which Linux does not
  have: that is H2 (`SO_PEERCRED`) together with the `/proc` telemetry.
- `SHA256_OF_H` / `SHA256_OF_T` in `hostprobe.rs` replace the ten `shasum -a 256` uses: `shasum`,
  else `sha256sum`, else `openssl dgst -sha256 -r`; test runs the expression with `sh` against Rust's
  digest, also with only `sha256sum`/`openssl` on `PATH`.
- Tests: detection table (macOS, Linux, WSL2 on both CPUs, MSYS/MinGW/Cygwin, cmd, PowerShell,
  FreeBSD, riscv, empty, hostile text), table lookups, loopback (`detect_host_target` on this Mac,
  unsupported and not-embedded installs refused). fmt, clippy and the workspace pass; the
  developer's tmux (pid 35350) was untouched.
- The pin stays the app's own digest of its own blob (§5); `detect_host_target` only chooses which
  blob. Swift does not use the new API yet (H4).

### H2 implementation notes (2026-10-19)

- `crates/host/src/linux.rs`: parsers for `/proc/meminfo`, `loadavg`, `cpuinfo`, `<pid>/stat` (name between
  the first `(` and the last `)`), `status` (uid), `uptime`, `/etc/passwd`, `cmdline` and `power_supply`,
  tested on every OS; the readers are Linux-only. The `telemetry` JSON is identical to macOS (no `ps`,
  `vm_stat`, `pmset`, no `PATH` dependency). `proc_info`/`has_children` use `/proc` too.
- Both musl builds (aarch64 2.5 MB, x86_64 2.7 MB) are embedded by `make xcframework`
  (`peer_is_us` uses `SO_PEERCRED` on Linux). Checked in a `debian:stable-slim` container: `hello`,
  `telemetry`, `ls --pane`, `read`, `kill` (refusals, stale pid, tmux server under a supervisor).
- Policy: added `~/.config/gcloud`, `~/.config/gh`, `~/.docker`, `~/.password-store`,
  `~/.local/share/keyrings`, `/etc/sudoers(.d)`, `/etc/gshadow` and the file name `gshadow`.
- Core transcript detection reads `/proc/<pid>/cwd` and `/proc/<pid>/fd` first, `lsof` as fallback.
- Not tested: a full app-to-Linux-sshd session (the loopback suite runs against the Mac), x86_64 at run time.
- Hacker pass: `docs/security-findings/2026-10-19-linux-helper-h2.md`.
