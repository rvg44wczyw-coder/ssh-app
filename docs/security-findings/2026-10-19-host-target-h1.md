# Security findings: feature 33 H1, host target detection and the portable digest (2026-10-19)

## 1. Scope

Commit `9779277` on `main` (clean tree). Reviewed: `crates/core/src/hosttarget.rs`
(`parse_host_target`, `HOST_TARGET_SCRIPT`, `require_supported`), `crates/core/src/hostprobe.rs`
(`SHA256_OF_H`, `SHA256_OF_T`) and its ten uses in `usage.rs`, `telemetry.rs`, `file_browser.rs`,
`crates/core/build.rs` (helper table), `usage.rs` (`bundled_helper_for`,
`bundled_host_helper_sha256_for`), `host_api.rs` (`detect_host_target`,
`install_bundled_host_helper_for`), `scripts/build-xcframework.sh`.

Applies: a host-supplied string that steers which build the app installs; a shell expression that
decides whether the pinned helper may run; build and embedding of the blobs. Ruled out: replay, key
confusion, timing (no secret is compared; the digest is public), path traversal (no path input).

Live tests (local only): the digest expression run with `sh` against a file, with a planted `shasum`,
a `shasum` that consumes stdin and fails, no tools at all, `openssl` only (macOS LibreSSL), a path
with a space, a directory and an unreadable file; and `parse_host_target` called through a small
scratch crate (outside the repository, path dependency on the core) with clean, noisy, spoofed,
bidi and 5 MB / 20 MB inputs. Code analysis only: the exit-status behaviour of the `||` chain under
`set -e`, the stale-blob reasoning for `build.rs` and the build script.

## 2. Findings

### 1. [InputValidation] Low (availability): shell-profile output before `uname` makes a supported host look unsupported

Where: `crates/core/src/hosttarget.rs` `parse_host_target` (takes the first two tokens of everything
before `===PROC===`); the probe runs through the login shell (`login_sh_script`), so anything the
user's profile prints (a greeting, `nvm`/`conda` messages, a motd) comes first.

Scenario: a Mac or Linux account whose `~/.zprofile` prints one line. Verified live:
`"Welcome back!\nDarwin arm64\n===PROC===\n"` → `Unsupported("Unsupported system: Welcome")`;
`"Last login: Mon\nnvm 0.39 loaded\nLinux x86_64\n…"` → `Unsupported("… Last")`. When H4 uses the
result to refuse installing the helper, such a host would be locked out of the helper. The reverse
also holds: a banner line that starts with `Darwin arm64 …` wins over the real `uname` line.

Fix direction: take the last non-empty line before the marker (the probe prints `uname` last, like the
other probes that read `out.lines().rev()`), or have the probe print a unique prefix
(`TARGET uname -sm`) and parse only that line.

### 2. [DoS] Low: the "unsupported" message is unbounded

Where: `hosttarget.rs` (`format!("Unsupported system: {}", display_safe(other))`, same for the CPU).

A host answering with one 5 MB token yields a 5 MB `reason` (verified live: the message starts with the
token and is not cut); 20 MB of probe input parsed in 0.47 s, so only the UI string size matters.
Fix direction: cut `os`/`arch` to about 40 characters before formatting.

### 3. [Downgrade] Informational: the host cannot weaken the pin by lying about its system

What a lying host can do: claim `Darwin x86_64` / `Linux aarch64` / WSL (verified: a `microsoft`
marker only changes the label, both variants share one triple). The consequence is that the app
uploads a different one of its own blobs; the digest checked on the host and later compared on every
call is the digest of that blob, taken from the app's table, never from the host. A blob for the wrong
CPU simply does not run (`hello` fails, status is "unusable", the existing fallbacks run). The host
already executes whatever the user asks of it over SSH, so steering the choice of blob gives it
nothing.

### 4. [Informational] The digest expression: what holds and what it is worth

Live results:

- Normal run: correct digest. Path with a space: correct (the digest of that file). Directory and
  unreadable file: empty or the empty-input digest (`e3b0c442…`), which differs from any pin, so
  every pin check (`!= pin` → `HELPER_ABSENT`, `= pin` → run, install `= sha256 || mismatch`) fails
  closed.
- No digest tool at all: the expression prints nothing; same fail-closed outcome.
- `openssl dgst -sha256 -r` (LibreSSL on macOS) is parsed correctly by `cut -d' ' -f1`.
- A first tool that reads stdin and then fails (`cat >/dev/null; exit 1`) did not leave the second
  tool with an empty stream here (the correct digest came back); had it, the result would have been
  the empty-input digest, a mismatch (fail closed), not a false match.
- A missing target file makes the group's redirection fail, so even a planted tool prints nothing.
- **A planted `shasum` earlier in `PATH` that prints the pinned digest satisfies the check** (any
  existing file). It needs the same account; the old single `shasum` had exactly the same property, so
  H1 does not add exposure, but it also does not close it: the pin protects against version drift and
  accidental replacement, not against code already running as the user (feature 24 §7). Fix direction
  if this ever matters: call the tools by absolute path (`/usr/bin/shasum`, `/usr/bin/sha256sum`,
  `/usr/bin/openssl`) instead of resolving them through `PATH`.
- Under `set -e` the `$(...)` inside `[ … ]` cannot abort the script; exit status of the group is
  irrelevant because only the output is compared.

### 5. [Informational] Embedding and build

- `build.rs` skips empty blobs and prefers `ssh-app-host-<triple>`; the legacy `ssh-app-host` is used
  for the `aarch64-apple-darwin` entry only when the triple file is missing. The legacy API
  (`bundled_host_helper_sha256`, still used by the Swift UI) reads the legacy blob directly, so if the
  two files ever differ (a manual build) the legacy digest and
  `bundled_host_helper_sha256_for(DarwinArm64)` disagree; the unit test only asserts equality when both
  exist. Fix direction: derive the legacy functions from the table.
- `scripts/build-xcframework.sh` removes a target's blob before building it, so a failed build cannot
  leave a stale one; `--locked` keeps dependencies as in `Cargo.lock`. Building with `cargo zigbuild`
  pulls a C cross toolchain (zig); the produced blobs are still pinned by digest in the app, but their
  bit-for-bit reproducibility was not checked.

## 3. Verdict

Findings (highest severity: Low): no Critical/High; two Low items (banner-fragile detection,
unbounded message) and three informational notes. Neither Low blocks H1, but finding 1 must be fixed
before H4 relies on the detection to refuse anything.

## 4. Resolution (2026-10-19, same day)

- 1: the probe prints `TARGET <uname -sm>` and the parser takes the last marked line before
  `===PROC===`; a profile greeting or a banner line imitating the answer no longer matters, and
  output without the marker is "did not answer" (or the Windows hint for cmd / PowerShell error
  text). Test `profile_noise_and_long_text_do_not_matter`; the loopback test runs the real probe.
- 2: host text echoed in messages is cut to 40 characters (`echo`), test above (5 MB token).
- 5 (first point): `bundled_host_helper_sha256`, `_size` and the install of the legacy API are now
  derived from the table (`DarwinArm64`), so the two can no longer disagree.
- 3, 4 and the rest of 5 stay as documented (the absolute-path hardening of the digest tools is the
  same decision as the `PATH` note of the T1 audit and is left for when the same-account threat is
  taken on).

Verdict after fixes: clean for H1.
