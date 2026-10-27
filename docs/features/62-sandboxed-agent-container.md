# 62. A project and its agent inside a sandbox

Status: draft v2 (2026-10-26, owner's request: "figure out how to do it safely"). Builds on `51-container-runtimes.md` (engines, commands, parsing), `61-project-scoped-hooks.md`, `23-local-models.md`. Research: Docker's own sandbox isolation documentation (docs.docker.com/ai/sandboxes/security/isolation), Pillar Security's cross-tool sandbox-escape write-up (secondary summary only), Apple `container` coverage; the owner's laptop has Docker Desktop (CLI 29.8.2, context `desktop-linux`), no Podman, no Apple `container`, macOS 27.

## 1. Problem and honest goal

An agent in a normal tab runs as the user: `~/.ssh`, `~/.aws`, browsers, other projects, the tmux server, everything. The owner wants to run a project **with its agent in a container** so that the agent, or a prompt injection it reads, cannot steal anything from the main machine.

"Cannot steal anything" has to be stated precisely, because a container is only one of three things that go wrong:

| Threat | Does a plain container stop it? | What this design adds |
|---|---|---|
| A. **Reading host secrets** (`~/.ssh`, `.env`, keys, other projects) | Yes, if nothing is mounted | mount nothing but the project copy; clone only tracked files, so untracked `.env` never enters |
| B. **Sending data out** (exfiltration) | No: open network lets it post whatever it can read | internal network + allow-listing proxy; or no network |
| C. **Write-back poisoning**: the agent plants a file the *host* later executes (`.git/hooks`, `.git/config` fsmonitor/sshCommand/aliases, `Makefile`, `package.json` scripts, CI files, `.vscode/tasks.json`, `.envrc`, `.claude/settings*.json`, `.agents/hooks.json`, `.codex/`, `.grok/hooks`, `.qwen/`, `.opencode/`, and **our own phone-hook files**) | **No, and it is the main real-world escape path** (Docker documents it; Pillar's escapes are all of this kind) | the host project is never mounted read-write: the agent works on a private clone, results come back as commits to be reviewed |
| D. **Escaping the container** (kernel, runtime bugs, `docker.sock`, `--privileged`) | Only partly: containers share a kernel | on macOS Docker Desktop's VM is a second wall; no privileged flags, no socket; Linux hosts get a stated lower assurance |
| F. **The agent approving itself / hiding what it does** | n/a | phone-signed decisions only (public key read-only in the container), a per-sandbox queue, visible mode (§6.2) |
| E. **Stealing the agent's own login** (its API token lives in the sandbox) | No | token is only what the sandbox itself holds; egress allow-list limits where it can go; tokens can be revoked; optional proxy injection later |

What the agent can still take, by design: the project's tracked files and history (it needs them) and the agent token of the sandbox. Everything else on the machine is out of reach if the rules below hold.

## 2. Model

A **sandbox session** is an ordinary tab whose tmux pane process is the container; tabs, feed, usage strip and the terminal work as for any session.

```text
host tmux pane --> docker run -it --rm  <ssh-app image for the agent>
                     |- /work        private CLONE of the project (named volume inside the engine's VM)
                     |- /home/agent  sandbox home: a dedicated host folder ~/.ssh-app/sandboxes/<id>/home (agent logins + transcripts)
                     `- network: internal network -> allow-listing proxy (model API hosts only), or none
```

### 2.1 Workspace modes

1. **Clone (default)**. At start the core makes the clone from **tracked content only** on the host (`git archive HEAD` or `git ls-files -z` + tar, never a copy of the directory) and streams it into the volume; untracked and ignored files (`.env`, local keys) are not copied, `.git/hooks` and `.git/config` of the host are never exposed. Inside, the sandbox has its own `.git` (history imported with `git bundle`). To get work back, the phone's Git screen does `git fetch` from the sandbox (a bundle/`git-upload-pack` stream via `docker exec`, executed on the host as a *fetch only*: fetch does not transfer hooks or config) into a branch `sandbox/<name>`; the user reviews the diff like a pull request from an untrusted contributor, **including the implicit-execution files listed in §1.C**, before merging. A review screen flags changed paths from that list.
2. **Direct mount (explicit, warned)**: the project folder at `/work` read-write, with the dangerous subpaths overmounted read-only (`.git/hooks`, `.git/config`, `.claude`, `.agents`, `.codex`, `.grok`, `.qwen`, `.opencode`, `.vscode`, `.idea`, `.github/workflows`, `.envrc`). Overmounting only covers paths that exist when the container starts, so the mode refuses unless these paths exist or are created empty first, and the UI states plainly that `Makefile`/`package.json`/hooks added later are still writable. Not recommended; exists for non-git folders.
3. **Mountless**: nothing from the host (scratch work, local models).

## 3. Container rules (core validates; none is a user option)

- Engine family only, argv built in the core, every path quoted; one validation function refuses a project that is `$HOME`, `/`, a parent of `$HOME`, contains `.ssh`/`.gnupg`/cloud-credential folders, or whose path passes through a symlink out of itself.
- `--user <non-root uid>`, `--cap-drop=ALL`, `--security-opt=no-new-privileges`, default seccomp (never `unconfined`), `--read-only` root filesystem + `--tmpfs /tmp`, `--pids-limit`, `--memory`, `--cpus`, `--ulimit nofile`. Never: `--privileged`, `--device`, `--pid=host`, `--ipc=host`, `--userns=host`, `--network=host`, `--cap-add`, a bind mount of `/var/run/docker.sock` or of anything under `$HOME` except the sandbox home folder.
- Environment: explicit allow-list from the profile; the host's environment is never inherited.
- Hard links: the clone/volume is created inside the engine, never hard-linked from host files (Docker documents writes through workspace hard links reaching outside).
- Image: built by the app from a generated Dockerfile on a **pinned base digest**, agent CLI installed at a pinned version from its official package, no third-party image pulled by default, nothing secret baked in; tag = agent + digest of the recipe, so a changed recipe rebuilds.

## 4. Network (threat B)

- **Offline**: `--network=none` (local models via a mounted, read-only model socket are out of scope here).
- **Online (default)**: the agent container sits on a Docker `--internal` network with **no route out**; a small proxy container (HTTP CONNECT allow-list: the agent's API hosts, optionally a package registry and `github.com` for clones the user lists) is the only peer with outside access. UDP/ICMP/DNS-to-the-world do not exist for the agent. This is what makes "cannot send data out" true; without it the sandbox only protects reading.
- Allow-lists per agent live in the registry row (`agent_registry.rs`); the user can add hosts per sandbox.
- Out of scope but noted: data can still leave to an allowed host (e.g. through the model API itself); the sandbox limits it to what the sandbox holds (§1).

## 5. Credentials (threat E)

The agent logs in once inside the sandbox (device-code flow over the terminal view); tokens live in the sandbox home folder only. No host `~/.claude`, `~/.codex`, `~/.grok`, `~/.gemini` is ever mounted. "Forget sandbox logins" deletes that folder. Later option: the proxy injects the API key so the sandbox never holds it (as Docker's sandboxes do).

## 6. Feed, usage and control from the phone

The owner wants to keep controlling the agent from the phone (2026-10-26); an earlier draft of this section ("approvals are not needed inside, the agent runs on auto-approve") was wrong for that and is replaced. There are **three layers of control**, from outermost to innermost; the first two are the real security boundary, the third is control and visibility.

### 6.1 Merge review and egress approval (what can leave the sandbox)

- **Merge review**: the only way work reaches the host project is the reviewed branch (§2.1); nothing is merged without the owner.
- **Egress approval**: the proxy (§4) denies any host that is not on the allow-list and raises a phone prompt, "Sandbox <name> wants to connect to example.com: Allow once / Always for this sandbox / Deny" (same signed-decision queue as tool approvals). A denied host stays unreachable. This is the control that actually stops exfiltration.

### 6.2 Tool approvals from inside the sandbox (what the agent does inside)

The agent's hooks run **inside** the container and ask the phone exactly as on the host, through the existing file queue (spec 13) with a **per-sandbox queue folder** instead of `~/.ssh-app/run`:

```text
host:  ~/.ssh-app/sandboxes/<id>/run/pending/    <- mounted read-write at the container's $SSH_APP_HOME/run/pending
       ~/.ssh-app/sandboxes/<id>/run/decisions/  <- mounted read-only; the host helper writes the phone's signed decision here
       allowed_signers (the phone's public key)  <- mounted read-only as a separate file, so the agent cannot swap it
```

- The host helper aggregates the pending folders of all live sandboxes into the same list the phone already shows (each entry labelled with the sandbox name and an "inside sandbox" mark); `ctl decide` for such an entry writes into that sandbox's `decisions/`.
- **The agent cannot approve itself**: a decision counts only with the phone's signature, verified by the hook with `ssh-keygen -Y verify` against `allowed_signers`; the container holds the phone's **public** key only (read-only), never a private key, and the host's real `~/.ssh-app/run`, `bin` and keys are not mounted. A forged or replayed decision file fails verification (nonce and request id are bound, as in spec 13). `decisions/` is read-only for the agent so it cannot even drop files there.
- The hook script and `python3`/`ssh-keygen` are part of the image; the hook files inside the container's home are written by the installer into the sandbox home, never into the host's agent configs (spec 61 is not involved for sandbox sessions).
- Limits on the untrusted side: at most N pending requests per sandbox, request size capped, text shown through `display_safe`, rate limit per minute; a flood becomes one "too many requests" line.
- Stated trust boundary (same as spec 11): a compromised agent process could call the hook with a request text that differs from what it then runs; that can only mislead the owner about an action **inside** the sandbox, whose blast radius is the container and its clone, not the host. It is control and visibility, not the security boundary (the boundary is §3, §4 and §6.1).
- **Mode per sandbox**: "Ask me" (default; every approval-worthy tool call goes to the phone as on the host) or "Auto inside the sandbox" (agent's own auto-approve flag, for long unattended runs; egress approval and merge review still apply). The tab always shows which one is active.

### 6.3 Feed and usage

The sandbox home is a **host folder**, so the existing host readers (transcript/feed, usage probe, quota probes) read the agent's transcripts there by pointing their config dirs at it (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `GROK_HOME` style variables are already supported by the probes). Host code only **parses** these files, never executes them; every parser is fuzzed and bounded. The terminal view, queue and "send now" work unchanged because the pane is an ordinary tmux pane. The Hooks screen (spec 61) lists sandbox sessions as "contained" and offers no hook install for them (their hooks are installed by the sandbox).

## 7. Engines

| Engine | Boundary | Use |
|---|---|---|
| **Docker Desktop (macOS)** | containers share one Linux kernel inside Docker's VM; the VM is the wall to macOS | first target (installed on the owner's laptop) |
| Apple `container` (macOS 26+, Apple silicon) | one lightweight VM per container | stronger; second target, not installed here |
| Docker Sandboxes (`sbx`, microVM) | microVM + host proxy with credential injection | study; may replace §4/§5 later |
| Rootless Podman | user namespaces, no root daemon | preferred on Linux hosts |
| Plain Docker on Linux | shared host kernel, daemon is root, `docker` group = root | allowed with a visible "lower assurance" warning; gVisor/Kata/sysbox runtime recommended |

### 7.1 Which engine runs (owner's rule, 2026-10-26)

The engine is detected on the **server** (spec 51 probe: Docker, Podman, later Apple `container`), never assumed:

| Found on the server | The sheet offers |
|---|---|
| both Docker and Podman | a choice of the engine (the last choice per server is remembered), plus "Run without a sandbox" |
| exactly one | it is used without asking, plus "Run without a sandbox" |
| none | "Install a container engine" (a guide for the server's OS, the install itself is never run silently) or "Run without a sandbox" |

"Run without a sandbox" is **always present**, whatever is installed. Choosing it shows a confirmation that says plainly: the agent will run as the user with access to everything the user can reach (keys, other projects, the whole account) and nothing limits what it can send out; the confirmation names the sandbox that is available and asks for an explicit "Run without protection". It starts the agent exactly like a normal tab; the tab keeps a visible "Unprotected" mark so a sandbox and a plain session are never mistaken for each other. The choice is per start, never a silent default, and is not stored as "always".

## 8. App and core interface

- New session sheet: "Run in a sandbox": project folder, agent, engine per §7.1, mode (Clone / Direct / Mountless), network (Online allow-list / Offline), limits; the unprotected start lives in the same sheet.
- Tab badge "Sandbox"; the Containers screen (spec 51) lists containers labelled `ssh-app.sandbox=<id>` with stop/remove; Settings → Sandbox: defaults, rebuild images, forget logins, show allow-lists.
- Core (UniFFI): `SandboxProfile { engine (Docker | Podman), agent, project_folder, mode, network, allow_hosts, cpus, memory_mb, pids }`, `plan_sandbox(profile) -> SandboxPlan { build_cmds, clone_steps, run_cmd, proxy_cmds }` (pure, golden-tested), `sandbox_image_recipe(agent)`, `review_sandbox_branch(...)` that returns changed paths with the implicit-execution ones flagged.

## 9. Tests (the Docker rule of AGENTS.md applies)

- Approvals: a decision file written by the agent inside the sandbox (unsigned, signed with another key, replayed, for another request id) is refused by the hook; the agent cannot write to `decisions/` or replace `allowed_signers`; a flood of pending requests is capped; the host helper lists only requests from live sandbox folders and never reads outside them.

- Golden snapshots of every generated command line per agent and mode; refusal tests (home, `/`, parent of home, folder with `.ssh`, symlink escape, quotes and `$()` in names, option-looking names).
- Property test: no generated argv ever contains a forbidden flag or mount (§3).
- Live tests only against a **private** engine or, on the owner's Docker Desktop, with his per-task permission, labelled `ssh-app.sandbox-test`, with all containers/volumes/networks removed afterwards and the engine's container list compared before and after. Checks: `~/.ssh` and the host `$HOME` are not visible inside; an untracked `.env` of the host project is absent from the clone; a write to `/work/.git/hooks/pre-commit` in the sandbox does not exist on the host after a fetch; the agent container cannot resolve or reach anything but the proxy; the proxy refuses a non-listed host.
- Fuzz of the review-path classifier and of every parser reading the sandbox home.

## 10. Residual risks (stated, not hidden)

1. A container escape through a kernel/runtime bug: on macOS contained by the Docker Desktop VM (another wall), not on plain Linux Docker.
2. Exfiltration through an allowed API host, and theft of the sandbox's own token (§1.E).
3. A reviewer who merges the sandbox branch without reading the flagged paths: the review screen forces the list on screen but cannot make anyone read it.
4. Docker Desktop itself is a large privileged component with host file sharing; mounts must stay minimal (§3).
5. Prompt injection can still make the agent write misleading code into the clone; that is a code-review problem, not a sandbox one.

## 11. Order of work

`rev(doc)` → core planner + golden tests → proxy/network recipe verified live on Docker Desktop (owner's permission) → clone/fetch flow → bridge → UI (sheet, badge, review screen) → `hacker` (attack the planner's refusals, the fetch, the proxy allow-list, the sandbox-home parsers) → engine rows beyond Docker Desktop.

## 12. Decisions (owner, 2026-10-26)

- Workspace mode: **Clone with review** is the default and the first thing built. Direct mount stays a later, warned option.
- Egress: **allow-listing proxy** (with phone approval for unknown hosts).
- Agents for the first images: **Claude Code, Codex, Grok, Antigravity**.
- Engine choice: §7.1 (Docker Desktop first, then Podman).

## 13. Progress

| Part | State |
|---|---|
| Core planner `crates/core/src/sandbox.rs`: profile validation, image recipes for the four agents, proxy recipe and filter, network/volume setup, clone (git bundle of HEAD), run command (only sandbox mounts, hook queue mounts in "Ask me" mode, agent auto flag in "Auto" mode), fetch (bundle verify, fsck, confined to `refs/heads/sandbox/<id>/*`), cleanup, review path classifier | done; 12 unit tests incl. forbidden-flag and mount-list checks, hostile-name execution with stub binaries, 400-case fuzz; **never run against a real engine** |
| UniFFI export, bridge, UI (sheet, engine choice §7.1, Unprotected mark, review screen) | done 2026-10-26/27; never seen on a device |
| Sandbox-side hook install into the sandbox home, host helper aggregation of per-sandbox pending queues | done; no live push check |
| Egress approvals (proxy log reader, filter reload, Allow / Allow always, push `blocked`, banner with the host) | done; pull-based, never seen on a device |
| Pinned base image digest (the planner refuses an unpinned image; the app needs a real `debian:12-slim@sha256:…` resolved once and recorded) | open |
| Live check on Docker Desktop (image builds, vendor install scripts for Grok and Antigravity inside the image, proxy allow-lists per agent, mounts invisible `~/.ssh`) | open: needs the owner's permission, labelled `ssh-app.sandbox-test` resources, engine state compared before and after |
| `rev` of this doc and `hacker` of the planner | not yet |

Known limits of the first planner (kept honest): the allow-list hosts per agent come from vendor docs and are a starting point extended by approvals; the vendor install scripts (`curl … | bash`) run at image build over TLS and cannot be pinned; the clone starts from `HEAD` (uncommitted work is not copied, by design).

Update 2026-10-26: live check on Docker Desktop passed for Claude Code / Online (see `docs/security-findings/sandbox-and-external-review-2026-10-26.md`); clone now starts on branch `sandbox-work`; the sandbox screen (`SandboxView.swift`) exists and compiles. Update 2026-10-27: review warnings (`review_sandbox_branches`: changed-file count and `implicit_execution_paths` per branch, git runs `merge-base` + `diff --name-only` with no external diff or textconv) and the tab marks (`TabSession.isolation`: green shield for a sandbox, orange for unprotected) are built; both need a look on a device. Open: install offer, other agents and Podman live.

Egress approvals (2026-10-26): pull-based. The proxy (tinyproxy, default deny, logs to the container's output) refuses unknown hosts. While the sandbox screen is open the phone reads `docker logs` of the proxy over its own SSH session (`sandbox_blocked_hosts`), keeps only plain DNS names (no wildcard, no IP literal, at most 20), and shows them with Allow. Allow (`allow_sandbox_host`) appends one exact-host regex to the sandbox's `proxy-filter` and sends `SIGUSR1` so the proxy re-reads it. The agent cannot do this: the command runs only in the phone's SSH session, so the tap is the approval. Checked live on Docker Desktop: refused host appears in the log, is parsed, allowed, and the next request through the proxy succeeds. Fixes found by that check: tinyproxy cannot log to `/dev/stdout` (the default stderr log is used) and the filter file now ends with a newline so an appended line is not glued to the last one. Push for a refused host (2026-10-26): the host daemon (`crates/host/src/egress.rs`) reads `<engine> logs --since 30s ssh-app-sbx-<id>-proxy` every 15 s while a sandbox folder exists, using the engine named in `sandboxes/<id>/engine` (written at setup; only `docker` or `podman` accepted), remembers the refused names per sandbox (max 100), stays silent about what is already in the log at its first look, and gives each new refusal to the push sink as a `waiting` event of the pseudo agent `sandbox`. The push is the constant `waiting` one (no host name: it came from the agent); the sandbox screen shows the name and Allow. The daemon runs only while a phone lease is live, so an app closed longer than the lease gets no push. Covered by unit tests with a canned log; not yet seen on a phone.


Update 2026-10-27: the refused-host push has its own relay category `blocked` (text "A sandbox blocked a host"); the Worker was redeployed (additive: old hosts and old apps keep working, an app that predates the category shows it as a banner while closed). The sandbox screen lists (reachable, blocked, always-allowed hosts, branches) are folded by default; with no container engine on the server the screen offers to copy an install command (`brew install --cask docker` on a Mac, `sudo apt install podman` on Linux) and a "Check again" button; nothing is installed for the owner.

Update 2026-10-27 (several sandboxes): the phone keeps a registry of the sandboxes it started (`SandboxRegistry`: id, agent, folder, engine, network, server). The Sandbox screen shows one folded block per sandbox of the connected server, each with its own allowed and blocked hosts, branches and removal; each agent tab carries its sandbox id (`TabSession.sandboxId`) and the push extension's list has one `sandbox:<id>` entry per sandbox, named after the tab that runs it (else "Sandbox <agent> <last 4>"). The host side was already per sandbox (the egress watcher scans every sandbox folder; the push rate limit is per sandbox).
