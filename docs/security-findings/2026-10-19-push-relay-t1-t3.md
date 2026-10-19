# Push relay (feature 35, T1-T3), hacker pass

Scope: `scripts/apns-worker` (deployed), `crates/host/src/push.rs` and the agent sink, `crates/core/src/push.rs` and the four `host_push_*` calls, the app's `PushManager` and push card. Live tests only against our own deployed Worker with fake tokens, and against a temporary `SSH_APP_HOME`/loopback sshd.

Live attacks run
- Worker: no secret, wrong secret, a prefix of the secret, lower-case `bearer`, GET, other path, empty body, 700-byte padding, bad token/category/env, an extra `command` field: 401/401/401/401/405/404/400/400/400 and the extra field is dropped (the forwarded body is built from validated fields only). 22 requests for one token in a row: the first 20 reach APNs, the 21st and 22nd get 429.
- Host: registration with `http://`, a quote-injection URL, user-info URL, bad token/env/short secret, a 3 KB body: all refused, no file written. A 0644 `push.json` and a symlinked `push.json` are ignored. The status output never contains the secret or the token (only the last 6 characters). Real `curl` send to the Worker: fake token gives 410 and the registration is removed; a wrong secret gives "relay failed" and keeps it.
- Core: URL/secret/token validators reject quote and `;` injections before a command string is built (unit tests); the loopback test registers, reads back, refuses a different pin on the host, unregisters.

Findings
1. Metadata, Low (fixed): the registration (with the relay secret) was an argument of the `sh -c` command and visible in `ps` on the host for a few milliseconds. Now the exec channel sends it on stdin (`exec_raw_with_stdin`), a small wrapper (`stdin_to_env`) reads up to 4 KiB into the environment variable `SSH_APP_IN` and the command pipes that to `push-register`; the command line contains nothing sensitive (unit test) and the loopback test registers and reads back through this path.
2. DoS, Low (accepted): the hourly counter is in KV (eventually consistent), so a burst across edge locations can pass slightly more than 20. Only secret holders reach it.
3. Replay, none: a captured request just repeats a constant notification, limited by the counter and by `apns-collapse-id`.
4. Not applicable: key confusion, downgrade (one fixed call), MITM beyond TLS to Cloudflare/curl with `--proto =https` and no redirect following.
5. `.p8` handling: only as a Worker secret; local copy has mode 0600 outside the repository. Sandbox needs a key created for "Sandbox & Production" (done, `R2LYYFG7JF`).

Verdict: clean (one Low accepted, one fixed).
