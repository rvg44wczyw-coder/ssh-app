# Feature 51: Container Runtimes Beyond Docker (Podman, nerdctl, Compose, Kubernetes, Incus/LXD)

- **Slug**: `container-runtimes`
- **Target Release**: extends Milestone 34 (feature 45)
- **Roles**: `doc-dev`, `rev`, `rust-core-dev`, `rev`, `swift-bridge-dev`, `rev`, `ios-ui-dev`, `rev`, `hacker`
- **Hacker Gate**: Yes (remote commands built from names, container shells, a cluster's pods and namespaces, strings that come from the host)
- **Status**: Draft (2026-10-22).

---

## 1. Problem and scope

The Docker screen (feature 45) works only where the `docker` command exists. Many hosts run something else: Podman (default on Fedora/RHEL), nerdctl over containerd, system containers under Incus/LXD, and clusters under Kubernetes. Docker Compose stacks are the usual way containers are grouped, and a phone user wants "restart the stack", not nine restarts.

### 1.1 In scope

| Runtime | Kind | What the phone gets |
| :--- | :--- | :--- |
| Docker | engine (exists) | as in feature 45 |
| Podman | docker-compatible CLI | list, stats, start/stop/restart, logs, shell |
| nerdctl | docker-compatible CLI | the same, **unverified on a real host** (see 1.3) |
| Compose stacks | grouping of the above | containers grouped by project; start/stop/restart of a project |
| Kubernetes | `kubectl` | pods of the current context: list, logs, shell, "restart" by deleting a pod that has a controller |
| Incus / LXD | `incus` / `lxc` | instances: list, start/stop/restart, boot log, shell |

### 1.2 Out of scope

- **Apple `container`** (macOS): deferred. Its documentation has no `restart` command and does not list the JSON fields of `list`/`stats`; code written from a guess would be wrong. It comes back when someone with the tool installed provides a recorded `container list --all --format json` and `stats` sample (fixtures, section 7).
- Building images, editing compose files, creating containers, `kubectl apply`, scaling, namespaces/contexts management (the context is whatever the host's `kubectl` already uses), Swarm, Nomad.
- Virtual machines (Multipass, Lima, UTM).

### 1.3 What is and is not proven

The development machine has Docker and `kubectl` only (no cluster context), no Podman, nerdctl, Incus or LXD. So: Docker is tested live (read-only, as in feature 45). Everything else is built on **documented output formats** (Go templates, `--format csv`, `kubectl -o json`) and tested with **fixtures taken from the tools' documentation**, not recorded from a host. Every runtime other than Docker is labelled "not yet seen on a real host" in the app until the owner confirms it once (a setting records the confirmation; section 4.4). Where a field is not documented the parser takes the safe reading and shows less rather than guessing.

---

## 2. Model

```rust
/// What runs the containers of a host. Detected on the host, never assumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ContainerRuntime { Docker, Podman, Nerdctl, Kubernetes, Incus, Lxd }

/// One thing the phone can show, list, act on and open a shell in.
/// `Container` already exists (feature 45); the fields below are added, none removed.
pub struct DockerContainerInfo {
    // existing: id, name, image, state, status_text, created_at, ports, unhealthy
    /// Compose project this container belongs to, `None` if it is not part of one.
    pub project: Option<String>,
    /// Compose service name, for the card's subtitle.
    pub service: Option<String>,
    /// Kubernetes namespace; empty for engines and Incus.
    pub scope: String,
    /// A restart is possible and safe: always true for engines and Incus, for a pod only with a
    /// recreating owner (3.4). The UI reads this; the core re-checks it before acting.
    pub restartable: bool,
}

pub struct RuntimeProbe {
    pub runtimes: Vec<ContainerRuntime>,   // those found on the host, in the order below
}

pub enum DockerUnavailable { NotInstalled, NoPermission, DaemonDown, Unexpected, NoCluster }  // NoCluster is new
```

Order of detection: `docker`, `podman`, `nerdctl`, `kubectl`, `incus`, `lxc`. More than one may be present (a Mac with Docker and kubectl): the screen shows a segmented control with the runtimes found; the last choice is remembered per server. `lxc` is offered only when `incus` is absent (the same commands, the older name).

### 2.1 Capabilities

Not every runtime can do everything; the UI reads these instead of guessing:

```rust
pub struct RuntimeCaps {
    pub stats: bool,         // memory/CPU numbers
    pub restart: bool,       // a real restart (Kubernetes: delete-with-controller, see 3.4)
    pub start_stop: bool,
    pub logs: bool,
    pub shell: bool,
    pub groups: bool,        // Compose projects / Kubernetes namespaces
}
```

| | stats | restart | start/stop | logs | shell | groups |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| Docker/Podman/nerdctl | yes | yes | yes | yes | yes | Compose projects |
| Kubernetes | no (needs metrics-server; out) | pod with a recreating owner only | no (pods are not stopped) | yes | yes | namespaces |
| Incus/LXD | no | yes | yes | boot log only | yes | projects (hidden if one) |

---

## 3. Commands and parsing (all in `crates/core/src/container_runtime.rs`, which replaces the Docker-only builders; `docker.rs` keeps its public names as thin wrappers so feature 45 callers and tests stay valid)

All commands run through the existing `execute_command_on_active_session` (login shell, `2>&1`) with the same discipline as feature 45: every dynamic value passes `validate_container_identifier` (or the namespace/pod variants below) **and** `shell_quote`, never concatenation.

### 3.1 Docker, Podman, nerdctl (the "engine family")

The binary is a closed set (`docker`, `podman`, `nerdctl`), chosen by the enum, never taken from input.

- **Probe** (one call, also yields the runtimes list): `for c in docker podman nerdctl kubectl incus lxc; do command -v $c >/dev/null 2>&1 && echo "RUNTIME:$c"; done`.
- **List**: Docker keeps its tested `ps -a --format '{{json .}}'`. Podman prints a JSON *array* for `--format json` and its documented Go placeholders are `.ID .Names .Image .State .Status .Ports .CreatedAt .Labels`; because the JSON shape is not documented, the family list uses a **template, not JSON**, for Podman and nerdctl:
  `ps -a --format 'ID={{.ID}}{{"\x1f"}}NAME={{.Names}}{{"\x1f"}}IMAGE={{.Image}}{{"\x1f"}}STATE={{.State}}{{"\x1f"}}STATUS={{.Status}}{{"\x1f"}}PORTS={{.Ports}}{{"\x1f"}}CREATED={{.CreatedAt}}{{"\x1f"}}PROJECT={{.Label "com.docker.compose.project"}}{{.Label "io.podman.compose.project"}}{{"\x1f"}}SERVICE={{.Label "com.docker.compose.service"}}{{.Label "io.podman.compose.service"}}'`
  (the separator is written as the template string `{{"\x1f"}}`: a bare `\x1f` outside an action is **not** interpreted by Docker's formatter and would come out as the four characters `\x1f`; checked live against Docker, which also confirmed `.Label "key"`.)   one container per line, fields split on the unit separator `\x1f` (U+001F), which cannot occur in any of these values, so a comma or pipe in a name or port list cannot shift fields (the whole `Labels` string is **not** parsed: its values contain commas and paths). `{{.Label "k"}}` is Docker's documented accessor; Podman documents `.Label`; if a runtime rejects the template the call fails as `Unexpected` and the message shows the first line it printed.
  If `.State` is empty (nerdctl may omit it) the state is derived from the `Status` text: starts with `Up` → running; `Exited`/`Created` → exited; `Paused`; `Restarting`; anything else unknown.
- **Docker** list also gets the project/service fields: the same two `Label` placeholders are added to its call as a second `docker ps -a --format '<template above>'` after the JSON, separated by a marker line `---PROJECTS---`, so the tested JSON path is unchanged.
- **Stats**: `stats --no-stream --format '{{.MemUsage}}{{"\x1f"}}{{.CPUPerc}}' <id>` for Podman/nerdctl (documented placeholders); Docker keeps JSON. Parsed by the existing `parse_size` and percent parser.
- **Actions**: `start|stop|restart <id>`, logs `logs --tail N --timestamps <id>`, shell `exec -it <id> sh -c '…bash || sh'`; identical to Docker with the binary swapped.
- **Unavailable reasons** are read per engine from the same phrases plus Podman's: "Cannot connect to Podman" / "podman.sock" → `DaemonDown`; "permission denied" → `NoPermission`; "command not found"/the probe says none → `NotInstalled`.

### 3.2 Compose stacks

A stack is a group of containers with the same `PROJECT`. The app groups them; **there is no compose file access**.

- Project actions (`start`, `stop`, `restart`) run **on the host as one script**, not as a series of calls from the phone: the phone's socket closes when the app is backgrounded (zero battery drain), which would leave a stack half-applied. The script is built by the core from validated, `shell_quote`d ids (at most 20; more is refused with a message), runs `<engine> <verb> <id>` for each in turn, prints `DONE:<id>` after each success and `FAILED:<id>` plus the engine's first error line and stops at the first failure. It does not use `compose -p`: `compose` needs the plugin and sometimes the original files, the per-container verbs work wherever the engine does. The host call cap (check the real value in `execute_command_on_active_session`; stack calls get 120 s where the cap is configurable) cutting the call leaves the outcome unknown: the screen then says "may be partly done" and reloads the list, which is the truth.
- Order: `restart` and `stop` in reverse of the list order is not knowable without the compose file (`depends_on` is in labels but its parsing is out of scope); the order is the list order and the card says "order is not dependency-aware".
- A stack action asks first, naming the project and the count ("Restart 9 containers of forte-dev-toolbox?").

### 3.3 Incus / LXD

The binary is `incus`, or `lxc` when `incus` is absent.

- **List**: `incus list --format csv -c nst` (columns name, state, type; `csv` and the column letters `n s t` are documented). Fields split on `,`; names cannot contain a comma (instance names are hostnames). `state` is mapped: `RUNNING`→running, `STOPPED`→exited, `FROZEN`→paused, others unknown. Virtual machines (`VIRTUAL-MACHINE`) are listed with a "VM" badge; shell and actions are the same commands.
- **Actions**: `incus start|stop|restart <name>` (`stop` may take time; the host call cap of 40 s applies, as for Docker).
- **Logs**: `incus console <name> --show-log` (the boot log; the spec says so on the screen: "boot log, not live output").
- **Shell**: `incus exec <name> -- sh -c 'command -v bash >/dev/null 2>&1 && exec bash || exec sh'`.
- **Unavailable**: not found → `NotInstalled`; "permission denied" or "you don't have the needed permissions" → `NoPermission` (the user must be in `incus-admin`); "connection refused"/"socket" → `DaemonDown`.
- No stats, no per-instance resource numbers.

### 3.4 Kubernetes (`kubectl`)

The context is the host's current one. The app never switches contexts and never passes a kubeconfig path.

- **List**: `kubectl get pods --all-namespaces -o json 2>&1 | head -c 3145728` (stable, documented; the output is cut at 3 MB (the exec channel keeps 4 MB) before parsing, a cut document is reported as "too many pods to list" and not guessed from). If the answer is `Forbidden` for all namespaces, the call is retried once for the current namespace (`kubectl get pods -o json`) and the screen says it shows that namespace only. Parsed with `serde_json` into one entry per pod: id = `namespace/name`, name = `name`, scope = namespace, image = first container's image (+ "+N" for more), state from `status.phase` (`Running`→running, `Pending`→restarting-ish "pending" shown amber, `Succeeded`→exited, `Failed`→dead, `Unknown`→unknown), status text = phase plus the restart count of the busiest container and a waiting reason (`CrashLoopBackOff`, `ImagePullBackOff`) when `containerStatuses[].state.waiting.reason` is set; `unhealthy` = any container not ready while running. The pod count is capped at `MAX_CONTAINERS` (the cap of feature 45) after sorting problems first; a note says "showing N of M".
- **Namespace and pod names** have their own validator: DNS-1123 (`[a-z0-9]([-a-z0-9.]*[a-z0-9])?`, ≤ 253), rechecked on every command; the id is split on the first `/` and both halves validated.
- **Logs**: `kubectl logs --tail=N --timestamps -n <ns> <pod> --all-containers=true`, cut at 1 MB like Docker logs.
- **Shell**: `kubectl exec -it -n <ns> <pod> -- sh -c 'command -v bash >/dev/null 2>&1 && exec bash || exec sh'`. For a pod with several containers the default container is used (kubectl picks it and prints which).
- **"Restart"**: only if the pod has an owner of kind `ReplicaSet`, `StatefulSet`, `DaemonSet` or `ReplicationController` (they recreate it; a `Job` pod is not recreated once it ends, and a `Node`-owned mirror pod is managed by the kubelet, so neither gets the button): `kubectl delete pod -n <ns> <pod> --wait=false`. A pod with no such owner is **not** restartable: the button is absent and the card says "no controller - deleting it would lose it". The confirmation names the pod and says it is deleted and recreated.
- **Start/stop, stats**: not offered (see the capability table).
- **Unavailable**: not found → `NotInstalled`; "The connection to the server … was refused", "Unable to connect", "no route" → `DaemonDown` (text says "cluster unreachable"); "current-context is not set" / "no configuration has been provided" → `NoCluster`; "Forbidden" → `NoPermission`.

---

### 3.5 Examples

```text
probe     -> "RUNTIME:docker\nRUNTIME:kubectl\n"              => [Docker, Kubernetes]
podman    -> "ID=3f2a\x1fNAME=web\x1fIMAGE=nginx\x1fSTATE=running\x1fSTATUS=Up 2 hours\x1fPORTS=0.0.0.0:80->80/tcp,:::80->80/tcp\x1fCREATED=...\x1fPROJECT=shop\x1fSERVICE=web"
          => one running container "web", project "shop", service "web"; the commas in PORTS stay in PORTS
incus     -> "web,RUNNING,CONTAINER\ndb,STOPPED,VIRTUAL-MACHINE" => web running, db exited with a VM badge
kubectl   -> {"items":[{"metadata":{"namespace":"app","name":"api-7d9","ownerReferences":[{"kind":"ReplicaSet"}]},"status":{"phase":"Running","containerStatuses":[{"ready":false,"restartCount":5,"state":{"waiting":{"reason":"CrashLoopBackOff"}}}]}}]}
          => id "app/api-7d9", amber, "Running, 5 restarts, CrashLoopBackOff", unhealthy, restartable
stack     -> restart "shop" (3 ids) prints "DONE:3f2a\nFAILED:9c1b no such container" => 1 done, 1 failed, 1 not started
```

## 4. App

### 4.1 Screen

The Docker screen of feature 45 becomes the **Containers** screen. A segmented control at the top lists the runtimes the probe found (hidden when there is one). The card layout is shared; buttons come from `RuntimeCaps`. Compose projects and Kubernetes namespaces are collapsible sections (Compose: section header with Restart/Stop/Start for the whole project; namespaces: no group action). Containers without a project sit under "Not in a stack".

### 4.2 Quick menu, Siri, Shortcuts

The quick-menu tile keeps the id `docker` and is titled "Containers". The Siri "Restart a Docker container" intent keeps its phrase and matches by exact single name across the **engine family only** (not pods/instances: a Shortcut must never delete a pod by a spoken name).

### 4.3 Unavailable messages

One message per reason and runtime, same shape as feature 45 (what is wrong, the way out, Try again). New: `NoCluster` ("kubectl has no cluster selected on this host. Run `kubectl config use-context …` there"). A pod's "cluster unreachable" says the host's `kubectl` could not reach the API server, not that the phone has a problem.

### 4.4 "Not yet seen on a real host"

Docker is verified. For the other runtimes the header shows a small "unverified" tag until the user taps "It works" once on that server; the choice is stored per server and runtime and affects only the tag. This is honest status, not a gate.

### 4.5 Tabs and tmux

A container/pod/instance shell opens a tab exactly like Docker's: command from the core builder, session name `docker-<slug>` becomes `<runtime>-<slug>` (validated by the same slug rule), icon `shippingbox.fill`. Zellij tabs use the same names.

---

## 5. Security

| Threat | Mitigation |
| :--- | :--- |
| Command injection from container, image, pod, namespace or project names (all come from the host and may be hostile on a shared box) | Names reach a command only after `validate_*` (charset, length) **and** `shell_quote`; the binary is chosen by enum; no value from the host is ever interpolated into a template string. Names are displayed through the existing `clean()` (control characters removed, length capped). |
| Output forged to look like another entry (a container named to contain `\x1f` or newlines) | Unit separator and newline are removed by `clean()` before parsing is trusted; a line that does not have exactly the expected field count is dropped, not guessed. |
| Deleting the wrong or an irreplaceable pod | Restart only for pods with an owner; confirmation names pod and namespace; Siri/Shortcuts never touch pods or instances; bare pods have no button. |
| Stack action half-applied | Sequential, stops at the first failure, reports done / failed / not started. |
| Docker group = root, `incus-admin` = root, `kubectl` admin context = cluster admin | The messages say so; nothing adds the user to a group. A shell inside a container/pod is not covered by the hook guard (stated on the screen). |
| Probe output used as command text | The probe prints fixed tokens (`RUNTIME:docker` …); the app maps them through the closed enum and ignores any other line. |
| Large output (thousands of pods) | Capped at 200 entries with a "showing N of M" note; logs cut at 1 MB; calls cut at 40 s by the host layer. |
| Zero battery drain | Nothing runs until the screen opens; auto-refresh runs only while the screen is open and the app is active (as in feature 45). |

---

## 6. Interface changes (UniFFI)

Added: `enum ContainerRuntime`, `struct RuntimeCaps`, `fn runtime_caps(runtime) -> RuntimeCaps`, `fn runtime_probe_command() -> String`, `fn parse_runtime_probe(output) -> Vec<ContainerRuntime>`, and on `SshSessionHandle` the runtime-aware calls `probe_container_runtimes()`, `fetch_runtime_containers`, `fetch_runtime_stats`, `runtime_container_action`, `runtime_stack_action`, `fetch_runtime_logs`; plus `build_runtime_exec_command(runtime, id)` and `runtime_session_name(runtime, name, id)`. The feature-45 Docker functions stay as they are (the Siri intent and the tests keep using them); the new calls are separate, not changed signatures. `DockerContainerInfo` gains `project`, `service`, `scope`; `DockerUnavailable` gains `NoCluster`. Swift callers pass `.docker` where they do not care (the Siri intent). `DockerAction::Restart` on Kubernetes maps to the delete-with-controller rule; the core refuses with `InvalidInput` for a pod without an owner (the owner rule travels as `restartable` on the info record, section 2).

---

## 7. Tests

Rust (all fixtures are text in the test module; none starts a runtime):
1. Probe: tokens → runtimes in order; junk lines ignored; empty → none.
2. Family list: a Podman-shaped output with `\x1f` separators parses; an empty `State` falls back to `Status`; a name with a comma and a ports list with commas do not shift fields; a line with the wrong field count is dropped; `PROJECT`/`SERVICE` from either label.
3. Docker list unchanged (existing tests) plus the `---PROJECTS---` section joins project to id.
4. Incus csv: states map, VM badge, stray comma line dropped.
5. Kubernetes: a pod list (Running, Pending, CrashLoopBackOff with restarts, Succeeded, bare pod without owner, multi-container) parses; pod with owner is restartable, bare one is not; namespace/pod validators refuse `..`, `-n x`, uppercase, spaces, `;`, a leading `-`.
6. Every command builder: the binary is from the enum; a hostile id (`$(x)`, `; rm`, newline, leading `-`) is refused before any command text exists; quoted output golden strings.
7. Unavailable mapping per runtime phrase, including `NoCluster`.
8. Restart refusal for a bare pod; stack sequence stops at the first failure (state machine test with a fake executor).
9. **Live, read-only**: Docker (as in feature 45: runs only if the daemon answers), `kubectl get pods -o json` only if a context is set (on this machine it is not, so the test asserts the `NoCluster` message, which is a real run of the real binary). No test starts, stops or deletes anything.

Swift: runtime picker hidden for one runtime, shown for several; capability-driven buttons (no Stop for pods, no Restart for a bare pod); stack header actions ask with a count; Siri intent still matches only engine containers; per-reason messages.

Coverage target ≥ 80 % per touched Rust file.

---

## 8. Open items / risks (stated, not hidden)

- Podman, nerdctl, Incus and LXD parsing is built on documentation, not on captured output. First real use on such a host may show a field to adjust; the "unverified" tag and the `Unexpected` message (which quotes the first line) are the safety net.
- `nerdctl`'s support for `.Label` and `.State` placeholders is unconfirmed; the fallback to `Status` text covers `State`, a rejected `.Label` makes the call fail visibly (no stacks), and the base list is retried without the label placeholders once before giving up.
- Apple `container` is deferred (1.2).
- Kubernetes metrics (`kubectl top`) need metrics-server and are not included.
- Compose dependency order is not respected (3.2).

---

## Revision notes (`rev` of the doc, 2026-10-22)

1. **The record separator was wrong.** A bare `\x1f` in the `--format` string is not interpreted by Docker's formatter (checked live: the output carried the literal characters `\x1f`); only a template action `{{"\x1f"}}` yields U+001F (checked live, together with `.Label "key"`). Section 3.1 now uses the action form.
2. Stack actions moved from "one call per container from the phone" to **one host-side script**: the socket closes in the background, which left a stack half-applied.
3. Kubernetes: output size cap before parsing, retry for the current namespace when listing all namespaces is forbidden, restart limited to owners that recreate the pod (not `Job`, not `Node`-owned mirror pods).
4. `restartable` added to the model (it was used in section 6 but not defined); examples added (3.5).
5. Implementation review (2026-10-22): the `---PROJECTS---` marker counts only as a line of its own (a label containing the text split the output and hid containers); the Kubernetes "Forbidden" retry applies only to non-JSON answers (a pod named or annotated "forbidden" narrowed the list silently); the stack script's watchdog is 110 s (the client gives up at 120 s); the pod list is capped at 3 MB because the exec channel keeps 4 MB.
