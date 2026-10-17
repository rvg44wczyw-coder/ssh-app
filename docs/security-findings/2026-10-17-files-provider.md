# Security review: Files app integration (File Provider), 2026-10-17

Scope: `crates/core/src/remote_fs.rs`, transport streaming, `ios/SSHAppFiles`, shared Keychain data. Method: code review plus live
probes against a throwaway sshd on loopback (private HOME, no tmux touched).

| # | Severity | Finding | Status |
|---|---|---|---|
| 1 | Medium | `mkdir`, `rename`, `delete` and `upload` checked only the lexical path. A symlink in an allowed folder pointing into a blocked one (`work/keys -> ~/.ssh`) let a write land in `~/.ssh` (e.g. delete `authorized_keys`, plant a key). | Fixed: parent folder resolved with `realpath` and the result re-checked (`resolved_child`); live test covers mkdir/delete/rename (both ends)/upload through a link |
| 2 | Low | Upload temp file `<dest>.part` was predictable and written through an existing link | Fixed: `<dest>.part.$$` and `set -C` (no clobber) |
| 3 | Low | An upload to an existing file kept streaming into a command that had exited (client stalled until the idle limit) | Fixed: existence check up front, write stall ends with the command's exit status |
| 4 | Info | Race between `realpath` and the read/write (symlink swapped in between) | Accepted: needs a process of the same host user, who can read the file anyway |
| 5 | Info | Blocklist is name-based; other credential stores stay reachable | Accepted and documented; list extended |
| 6 | Info | A file name containing a newline can produce a forged entry line pointing to a non-existent path | Accepted: headers are only trusted before the first entry; any use of such a path is re-validated and fails |
| 7 | Info | GNU `stat` branch not run on Linux | Open: parser covered, script to be run on a Linux host |
| 8 | Info | Files served to any app with Files access while the phone is unlocked | Inherent to the feature; per-server Face ID gate is a stage 3 item |

Verified: no host key is ever trusted on first use (client refuses to build without a pin; wrong pin refused), key never leaves the
Keychain except into the core (zeroized there), downloads are streamed to a `.part` file renamed on success and removed on error or
cancel, size cap enforced both from `stat` and on the stream, folder listing capped at 20 000 entries, no recursive delete.
