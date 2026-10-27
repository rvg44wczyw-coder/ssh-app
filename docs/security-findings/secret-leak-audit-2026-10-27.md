# Secret and personal-data leak audit of the public repository (2026-10-27)

## Scope

All 449 commits on every ref (`git log --all`, pushed head `origin/main` = `d3c87d1` plus the local commits), the
current tree and the list of ever-tracked file names. Read-only; no value was printed, only counts and file names.
Method: `git log --all -G<pattern>` / `-S<value>` and `git grep` over `HEAD`.

## Results

| Check | Result |
|---|---|
| Private key blocks (`BEGIN … PRIVATE KEY`) | 7 commits, all in code or docs as string literals (format checks, `.replace(/-----BEGIN PRIVATE KEY-----/…)` in the Worker, a doc line); no key body anywhere |
| Ever-tracked `*.p8`, `*.pem`, `*.p12`, `wrangler.toml`, `.env*`, `auth.json`, `push.json`, provisioning profiles, `id_*` | none (only `wrangler.toml.example`) |
| Apple key files in `~/.private_keys` (the two `AuthKey_*` files, the relay secret file): their contents searched for in history with `-S` | 0 occurrences each |
| E-mail addresses (the owner's relay address, `@gmail.com`, `@icloud.com`), API-token shapes (Google, OpenAI-style, GitHub, Slack, AWS, JWT, `Bearer …`) | 0 commits |
| Commit authors and committers | all 449 are the GitHub no-reply identity; no work account |
| Tracked agent settings (`.claude/`, `CLAUDE.md`, `settings.local.json`) | none |
| Ignored local config (`scripts/apns-worker/wrangler.toml`) | not tracked, not in history |

## Personal, not secret (left as is, listed so it is a conscious choice)

1. The local account name `ivs` and paths such as `/Users/ivs/rust/ssh-app` in tests and docs (file browser and usage-slug tests,
   spec 21/32 examples, two agy conversation ids with project names in `docs/architecture/agy-usage-and-limits.md`) and the real tmux pid 35350 in findings.
   Fixing the tree is cheap; rewriting history is not worth it. Proposal: replace in the tree only, on the owner's word.
2. The bundle id `com.semenov.sshapp…`, the Apple team id `JJ5UK9N5JM` and the APNs key id `R2LYYFG7JF` (an id, not the key): all
   are visible in any shipped app and in Apple's portals; not secret.
3. The Worker address `ssh-app-apns-bridge.dgtny5wyr2.workers.dev` (default in `SSHApp.swift`, tests, docs): public by nature. The Worker
   refuses calls without the relay secret and limits 20 pushes an hour per token, so knowing the address gives nothing but a 401.
   It does tell anyone which Cloudflare account subdomain is the owner's.
4. Example tailnet addresses (`100.92.14.88`, `100.64.0.x`) are not the owner's (`tailscale ip` starts `100.110.`).
5. The sample prompt `ivs@mac ~/rust/ssh-app % claude` in the terminal colour-scheme preview.

## Verdict

Clean: no secret, key, token or private address has leaked into the repository history. Five personal-data items listed above are by choice.
