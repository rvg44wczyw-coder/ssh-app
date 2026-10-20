# Audit: zero-touch onboarding, `ssh-app-host pair` and `doctor` (feature 44)

Date: 2026-10-20. Scope: `crates/host/src/{pair,doctor}.rs`, `crates/core/src/onboarding.rs`,
`transport.rs` (password login, host key probe), the pairing sheet and the host check in the app.
Method: reading the code, the unit tests, the loopback sshd tests (private HOME, private tmux socket;
the real tmux server pid was 35350 before and after every run).

## Verdict

No open High or Medium finding. Six findings were fixed during the audit; three Low points remain
as documented limits.

## Findings and what was done

| # | Level | Finding | Result |
| :--- | :--- | :--- | :--- |
| 1 | Medium | A key that was commented out in `authorized_keys` (`# ssh-ed25519 AAAA...`) counted as "already present", so a phone whose key had been disabled was reported as paired and could not log in. Both the helper and the pasted command matched the blob anywhere in a line. | Fixed: lines whose first non-blank character is `#` are ignored in `has_blob` and in the shell command. Tests: `blob_search_matches_whole_fields_only`, `a_commented_out_key_is_not_taken_for_an_authorized_one`. |
| 2 | Low | The password field had `textContentType(.password)`, which can make iOS offer to save the host password to iCloud Keychain, against "never stored". | Fixed: the hint is removed. |
| 3 | Low | The first design had the helper print the path of `authorized_keys`; a path with the user name is not needed by the app. | Fixed: not returned. |
| 4 | Low | `doctor` returned absolute config paths in the first design (reveals the user name). | Fixed: only `~`-relative names are returned, and a test checks the report holds no part of the home path. |
| 5 | Low | `has_blob` and the shell `grep -F` also matched part of a longer field. | Fixed in the helper (whole fields); the shell command uses the 68-character blob, which cannot be a substring of another key type's blob. |
| 6 | Low | Pairing through the helper needs the helper installed first; with a wrong pin the helper is not run. | Checked: `pair_this_phone` with another pin answers `HELPER_ABSENT`, nothing is written (loopback test). |

## Checked and found sound

- **Injection.** The key is one of two types with a fixed blob shape and alphabet, the device name
  holds letters, digits, space and `._-` only. Both are validated in the core and again in the
  helper; the paste command contains only that alphabet (tests with quotes, `$()`, backticks,
  newlines, options, NUL, non-ASCII). The pairing request reaches the helper on stdin, never in a
  command line.
- **Files.** `~/.ssh` and `authorized_keys` are opened with `O_NOFOLLOW`; a symlink at either is
  refused and not followed (tests); a directory writable by others is refused; a file owned by
  someone else is refused; a loose file mode is set to `0600` and reported; files over 1 MiB are
  refused; the file is never rewritten, only appended (a missing final newline is completed first);
  one backup of the first state is kept; CRLF and non-UTF-8 content stay byte for byte.
- **Concurrency.** An exclusive `flock` on a lock file opened with `O_NOFOLLOW`; eight parallel
  pairings of one key give one line.
- **Password.** Used once, only after the user has seen and confirmed the host key; the connection
  is pinned to that fingerprint, so another key aborts before the password is sent (loopback test
  with a wrong pin). Held in a zeroizing buffer in the core and dropped right after the login.
- **Doctor.** Read-only (a test checks nothing is created), no shell, fixed argv, 2 s per tool,
  `TMUX` removed from the environment, no file contents returned (a test plants a secret in
  `settings.json`), versions and the Tailscale address are re-validated in the core.
- **No hosted script.** Nothing is downloaded and run anywhere in the feature.

## Remaining limits (accepted, documented)

1. The password is a Swift `String` in the sheet and is copied inside `russh` while the login runs;
   neither can be wiped. It is removed from the view state as soon as the call starts.
2. `doctor` runs the tools it finds on the user's own `PATH` (`tmux -V`, `tailscale ip -4`); a
   hostile `PATH` is already the user's own compromise.
3. The pinning step proves only that both connections reached the same key; the user still has to
   compare the shown fingerprint with the host (the sheet says how).
