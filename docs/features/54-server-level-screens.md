# Feature 54: Server-level screens (sessions grouped by server, container and process monitors)

- **Status**: UI, probes and Ports implemented 2026-10-23 (findings: `docs/security-findings/2026-10-23-ports-and-probes.md`). The server's own connection implemented 2026-10-24 (exec-only handle, pinned host key required, closed 15 s after the last screen and in the background; findings `docs/security-findings/2026-10-24-server-connection.md`), device run pending. Open: Kubernetes actions beyond read.
- **Design**: Liquid Glass throughout; system `List`, `Section`, `Menu`, `glassEffect` buttons.

## 1. Purpose

Docker, Kubernetes and host processes belong to a **server**, not to a session and not to the app as a whole. Today the Containers screen and the host monitor are reached from the session's quick menu, which suggests they belong to that session. The owner wants the Sessions screen split by server, and each server section to carry its own buttons for full-screen monitors.

## 2. Behaviour

- **Sessions screen**: one section per server (header: name, connection state, host key status); the sessions of that server are the cards below. A server without sessions still shows its section (so its monitors stay reachable). Sessions without a server (local shell, if any) go to the last section "This device".
- **Server header actions** (glass buttons, compact row): *Containers* (Docker/Podman/nerdctl/Incus/LXD runtime picker, feature 51), *Kubernetes* (pods, nodes, namespaces, logs; read first, actions behind confirmation), *Processes* (host monitor: CPU, memory, top processes, kill with confirmation; replaces the session-bound host telemetry sheet), *Ports* (listening ports and forwards, if available).
- The screens open as full navigation destinations bound to the server profile; they use their own SSH connection (the server's existing control channel), not the active session.
- A button appears only when it can work: Containers/Kubernetes are hidden until a probe on that server found a runtime or `kubectl` context (cached, re-probed on pull to refresh); unavailable reasons stay as in feature 51.
- The quick menu keeps shortcuts to the same screens, opening them for the **active session's server**.
- Kubernetes: first release is read-only (list, describe, logs, events); delete/restart/scale behind a confirmation sheet and only when the context is the one the owner picked. Tests never touch a real cluster (fake `kubectl` script).

## 3. Constraints

- No new background timers: monitors poll only while their screen is open.
- All process/container/pod names are untrusted display text.
- Docker/kube commands are built from fixed argument lists; names are passed as arguments, never through a shell string.

## 4. Plan (dev-chain)

`doc-dev` → `rev` → `rust-core-dev` (kubectl layer, host process snapshot/kill, probes) → `rev` → `swift-bridge-dev` → `rev` → `ios-ui-dev` (grouped Sessions screen, server header, Kubernetes and Processes screens) → `rev` → `hacker` → merge. The grouped screen and server-bound entry points (UI only, existing Containers and host monitor) can ship first.
