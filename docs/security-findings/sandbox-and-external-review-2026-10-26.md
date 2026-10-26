# Sandbox planner, sandbox queue aggregation, TOFU prompt, log redaction (2026-10-26)

## Scope

Reviewed: `crates/core/src/sandbox.rs` (+tests), `crates/core/src/host_api/sandbox.rs`, `crates/host/src/ctl.rs` (per-sandbox queues), `agyquota::folder_runs_project_code`, the Swift host-key confirmation (`MainTerminalView.swift`) and the removal of host/user/server names from `print`. Also the external review received on 2026-10-26.

Live tests: unit and loopback tests only (hostile names with `$()`/backticks, 400-case planner fuzz, symlink / FIFO / flood / loose-mode entries in a sandbox queue, all against temp folders). **Nothing was run against a real Docker or Podman engine**: the image builds, the proxy allow-list and the mounts are code-analysis only until the owner permits a live check.

Ruled out: crypto (no new primitives), downgrade (no negotiation).

## External review, point by point

1. High, agents run with the SSH user's rights: the answer is spec 62 (sandbox). Planner built and tested; UI and egress approvals open. Until then the host guard stays heuristic. Status: **open, in progress**.
2. Medium, first host key trusted automatically: **fixed in the app**. A server with no pinned fingerprint now probes the key (`probeServerHostKey`, nothing sent), shows the fingerprint in "Trust this server?" and pins it only on confirmation. Needs a device check.
3. Low–Medium, private key crosses UniFFI as `String`: **accepted residual risk**. A Swift `String` cannot be zeroized; the key is created once, goes straight to the Keychain and is not kept. A fix needs a `Data`/bytes record on the Rust side; listed in `docs/todo.md`.
4. Low, quota probe may run project hooks/MCP: **mitigated**: probes are skipped in folders with MCP configs or foreign hooks (`folder_runs_project_code`).
5. Logs reveal host/user/server names: **fixed** (names removed from `print`).
Accepted limits (Git hooks, App Lock is not encryption, iCloud list tampering without vault) stay documented as such.

## Findings

1. **SandboxEscape / Low**: the agent can write any file into its own `run/pending` folder and so put forged text in front of the phone. The phone shows the sandbox tag, the request is unsigned until the owner signs a decision, and the agent cannot sign. The reader (`ctl.rs`) opens entries with `O_NOFOLLOW|O_NONBLOCK`, regular files only, owner uid, size cap, at most 200 entries, and ignores sandbox folders that are not 0700 or have a bad name (tests `symlinks_fifos_loose_folders_and_floods…`, `a_sandbox_folder_that_is_not_private…`). Residual: a request can lie about its command text; the user is shown it as the agent's claim. Fix direction: show the "from sandbox X" mark prominently in the approval card.
2. **SandboxEscape / Medium (unverified)**: image builds run vendor install scripts (Grok, agy) from the network at build time, outside the proxy. Pin versions/hashes where the vendor publishes them; build on the host side with no project mounted (already so).
3. **MetadataLeak / Low**: tinyproxy logs contain requested host names; kept in the sandbox state folder (0700).
4. **Open**: egress approvals are not implemented, so an allow-list change needs a manual edit/re-plan; the sandbox UI and engine-choice sheet are not built, so the feature is not reachable from the app.

## Verdict

Findings (highest: Medium, unverified until a live engine check). The sandbox must not be offered in the UI before the live check and findings 2 and 4 are closed.

## Live check on Docker Desktop (2026-10-26, owner's permission)

Test: `crates/core/tests/sandbox_live.rs` (ignored, `SSH_APP_LIVE_DOCKER=1`). Throw-away `$HOME` and git repo; only `ssh-app-sbx-*` resources; containers, images, networks and volumes of the owner compared before and after: identical. Real tmux pid unchanged (35350).

Held (Claude Code profile, Online, approvals to the phone): image and proxy builds; clone of HEAD only; uid = host uid, `CapEff` 0; no `~/.ssh`, `/Users` or `docker.sock` inside; root filesystem read-only; the untracked `.env` and the host's `.git/hooks` did not cross; `decisions` and `allowed_signers` read-only, `pending` writable; allow-listed host answered through the proxy (404 from the API host), another host refused (000), direct access refused (internal network); host working tree and HEAD unchanged after the review fetch; hostile hook did not run.

Bug found and fixed: the clone of a HEAD-only bundle was a detached HEAD, so the review fetch of `--branches` had nothing ("Refusing to create empty bundle"). The clone now starts on branch `sandbox-work`; fetched as `sandbox/<id>/sandbox-work`.

Not covered: Grok / agy / Codex image installs, Podman, the agent itself running (no login inside), offline profile, auto-approve profile.

### Egress approvals (live, 2026-10-26)

A host name in the proxy log comes from the agent, so it is filtered to plain DNS names before it is shown; allowing runs only from the phone's SSH session, accepts one exact host (wildcards refused), caps the filter at 128 lines and refuses a symlinked filter. Residual: a name the agent picks is shown to the owner as is (for example a look-alike domain); the owner decides. Finding 4 of the list above (egress approvals) is closed, except for push notifications.

### All four agents, live (2026-10-26)

Claude Code, Codex, Grok and Antigravity: image builds, binary starts (no login), uid not root, no capabilities, host paths invisible, read-only root, `decisions`/`allowed_signers` read-only, direct egress refused, each agent's listed hosts answer through the proxy, an unlisted host is refused, review fetch brings `sandbox/<id>/sandbox-work`. Bug found and fixed: the agents were installed under `/home/agent`, which the run mounts over with the sandbox home, so none of them was on the PATH at run time; they now install under `/opt/agent` (outside the mount). The list management (remove, add, wildcard typed by the owner) rewrites the filter in place because the proxy holds a bind mount of that file.

### Login copy (live for Claude Code, 2026-10-26)

By the owner's decision the agent's login file is copied into the sandbox home by default. Only that file crosses (not the agent's config, whose hooks/MCP/permissions would defeat the sandbox); `cp` runs on the host so the content never reaches the phone or the logs; a symlink source is refused; an existing sandbox copy is not overwritten; mode 0600. Residual: code inside the sandbox can read the token (it can leave only through the allow-listed hosts), and a copy and the original may invalidate each other when one refreshes (unverified). The first Claude run also showed that `platform.claude.com` was missing from its default hosts (added).
