# Feature 37: Multi-Agent Mesh & Cross-Server Room Federation (N-Session & Cross-Host Collaboration)

- **Slug**: `multi-agent-mesh-federation`
- **Target Release**: Milestone 26 (Collaborative Multi-Agent Intelligence)
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Hacker Gate**: Yes (multi-session execution dispatch, cross-host message routing, distributed workspace locks)
- **Status**: Not ready: sections 3.2-3.3 and the lock need a redesign (`rev`, 2026-10-20; see the last section).

---

## 1. Problem Statement & Scope

### 1.1 The Vision
Modern engineering environments are rarely limited to one machine or one agent:
- A developer may run **Claude Code** on a MacBook for swift UI iteration, **Antigravity CLI (agy)** for core architecture, and a **local DeepSeek / Qwen model via Ollama** on a separate dedicated Linux GPU server for high-throughput code review or test generation.
- Furthermore, on a single machine, developers frequently run **multiple parallel sessions** (e.g. Session 1: iOS frontend agent, Session 2: Rust core agent, Session 3: background test runner).

The developer needs a **unified collaboration room (Mesh Room)** where:
1. **N sessions on the same host** can be dynamically checked into a conference room.
2. **Cross-server federation** allows bridging sessions across different remote hosts (e.g. `macbook` + `gpu-box` + `cloud-vps`) over secure SSH/Tailscale tunnels coordinated by the client.
3. Every agent can see, critique, and build upon the work of others under human oversight without file-write race conditions or runaway token loops.

### 1.2 Invariants & Architectural Rules
1. **Strict No-Emoji Mandate**: Zero emojis in all interfaces, headers, cards, badges, and code. All iconography relies exclusively on Apple's **SF Symbols** (`server.rack`, `terminal.fill`, `cpu.fill`, `person.fill`, `network`, `lock.fill`, `checkmark`, `xmark`) and clean ASCII/Unicode typographic markers (`●`, `○`, `⬡`, `❯`, `⎿`, `[✓]`, `[✕]`).
2. **Gradual Phased Implementation**:
   - **Phase 1**: Dual-Session Bridge (Claude + agy manual relay on one host).
   - **Phase 2**: Driver & Reviewer (Lead developer + automated cross-auditor).
   - **Phase 3**: MCP Peer Bridge (On-demand consultation tool).
   - **Phase 4**: Local N-Session Mesh Room (Arbitrary sessions on a single host with turn limits).
   - **Phase 5**: Cross-Server Mesh Federation (Multi-host coordination via client-side relay).
3. **Zero Battery Drain**: Room coordination operates over existing `SshSessionHandle` channels. Backgrounding closes client TCP sockets instantly. Background tmux sessions continue on their respective hosts.
4. **Distributed Workspace Leases**: To eliminate filesystem corruption, write operations (`Edit`, `Write`, `git commit`) require an exclusive lease on the target repository folder.

---

## 2. Visual Identity & UI Design System (No Emojis)

### 2.1 Participant & Host Visual Matrix

| Entity | Type | Glyphs | Color Coding | SF Symbol | Addressing Key |
| :--- | :--- | :---: | :--- | :--- | :--- |
| **User (You)** | Human | `❯` | Accent Tint (`Color.appAccent`) | `person.fill` | `@user` / `You` |
| **Claude Code** | Agent | `●` | Amber / Rust (`#F59E0B` / `#D97706`) | `terminal.fill` | `@<host>/claude` |
| **Antigravity (agy)** | Agent | `○` | Cyan / Sky (`#38BDF8` / `#0284C7`) | `cpu.fill` | `@<host>/agy` |
| **Local LLM (Ollama)** | Agent | `⬡` | Purple / Violet (`#A855F7` / `#7C3AED`) | `brain.fill` | `@<host>/ollama` |
| **Custom Shell** | Shell | `>_` | Monochrome Muted (`#64748B`) | `terminal` | `@<host>/<name>` |
| **Remote Host** | Machine | `H:` | Secondary Label | `server.rack` | `<host-slug>` |

---

### 2.2 Wireframe 1: Multi-Session Room Header & Active Participants

```text
┌───────────────────────────────────────────────────────────┐
│ [ < Rooms ]   Mesh: Architecture Review             [ ⚙ ] │
│               2 Hosts · 3 Agents Active · ssh-app         │
├───────────────────────────────────────────────────────────┤
│ Active: [● mac/claude] [○ mac/agy] [⬡ gpu/deepseek] [ + ] │ <-- Горизонтальный скролл участников
├───────────────────────────────────────────────────────────┤
│                                                           │
│ 14:10:02 ❯ User (to @all):                                │
│ Нужно согласовать архитектуру кросс-серверного брокера.   │
│ ───────────────────────────────────────────────────────── │
│                                                           │
│ 14:10:20 ● mac/claude:                                    │
│ Предлагаю использовать легковесный binary protocol через  │
│ именованные пайпы на хосте и клиентский SSH ретранслятор. │
│ [ ↳ Pass to @all ]       [ ↳ Pass to @gpu ]      [ Copy ] │
│ ───────────────────────────────────────────────────────── │
│                                                           │
│ 14:10:45 ⬡ gpu/deepseek-r1:                               │
│ Замечание по масштабированию: клиентский ретранслятор     │
│ создаёт узкое горлышко при передаче больших diffs.        │
│ Для крупных логов хосты должны поднимать прямое mTLS 1.3  │
│ соединение с пинингом SHA-256 отпечатка сертификата,      │
│ независимо от того, какая сеть под ними (WAN, IP или TS). │
│ [ ↳ Pass to @claude ]                            [ Copy ] │
│ ───────────────────────────────────────────────────────── │
│                                                           │
│ 14:11:10 ○ mac/agy:                                       │
│ Поддерживаю предложение deepseek по mTLS 1.3.             │
│ Давайте зафиксируем контракт: Client-Relay для сообщений, │
│ mTLS 1.3 сокет для тяжелых артефактов и diffs.           │
│                                                           │
├───────────────────────────────────────────────────────────┤
│ Target: [ All (3) ]  [ mac/claude ]  [ gpu/deepseek ]     │
├───────────────────────────────────────────────────────────┤
│ [Esc] [Tab] [^C] [↑] [↓]                                  │
│ [paperclip] ❯ @all Утверждаем схему с прямым сокетом...[⏎]│
└───────────────────────────────────────────────────────────┘
```

---

### 2.3 Wireframe 2: Participant Drawer / Session Picker Sheet (`RoomParticipantsSheetView`)

Modal sheet invoked by tapping `[ + ]` or `[ ⚙ ]` in the room header:

```text
┌───────────────────────────────────────────────────────────┐
│ Room Participants & Sessions                      [ Done ]│
├───────────────────────────────────────────────────────────┤
│ CONNECTED HOSTS & OPEN SESSIONS                           │
│                                                           │
│ server.rack macbook-pro.tailnet (Primary)                 │
│   [✓] ● claude-code-ui        Claude 3.7   (Driver: Write)│
│   [✓] ○ antigravity-arch      agy 1.4      (Auditor: Read)│
│   [ ] >_ build-terminal       zsh          (Worker)       │
│                                                           │
│ server.rack linux-gpu-rig.tailnet                         │
│   [✓] ⬡ deepseek-r1-qwen      Ollama 32B   (Consultant)   │
│   [ ] >_ tensor-train-log     bash         (Observer)     │
│                                                           │
│ server.rack vps-deploy.tailnet                            │
│   [ ] ● claude-deploy         Claude Code  (Idle)         │
│                                                           │
├───────────────────────────────────────────────────────────┤
│ CONCURRENCY & WRITE LEASES                                │
│ Repository: /Users/ivs/rust/ssh-app                       │
│ Active Write Lock: macbook-pro:claude-code-ui             │
│ [ Release Lock ]           [ Grant to Selected Agent ]    │
├───────────────────────────────────────────────────────────┤
│ TURN LIMITER & CIRCUIT BREAKER                            │
│ Max Autonomous Rounds: [ - ] 3 Rounds [ + ]               │
│ Stop on Consensus: [✓] Enabled                            │
└───────────────────────────────────────────────────────────┘
```

---

## 3. Cross-Server Architecture & Communication Topology

```mermaid
flowchart TD
    subgraph Client ["iOS App / Rust Core (Client Hub)"]
        Mesh["Mesh Room Coordinator"]
        Router["Multi-Host Router & Turn Engine"]
        Timeline["Unified Stream & Deduplicator"]
    end

    subgraph HostA ["Host A: MacBook Pro (Tailscale)"]
        SSH_A["SSH Session A"]
        tmux_claude["tmux: claude-code"]
        tmux_agy["tmux: agy-arch"]
    end

    subgraph HostB ["Host B: Linux GPU Server (Tailscale)"]
        SSH_B["SSH Session B"]
        tmux_ollama["tmux: deepseek-qwen (Ollama)"]
    end

    subgraph HostC ["Host C: Cloud VPS"]
        SSH_C["SSH Session C"]
        tmux_deploy["tmux: deploy-worker"]
    end

    Mesh --> Router
    Router <-->|"SshSessionHandle A"| SSH_A
    Router <-->|"SshSessionHandle B"| SSH_B
    Router <-->|"SshSessionHandle C"| SSH_C

    SSH_A <--> tmux_claude & tmux_agy
    SSH_B <--> tmux_ollama
    SSH_C <--> tmux_deploy

    Router --> Timeline
```

### 3.1 Why Client-Coordinated Federation is Optimal
1. **Zero Host-to-Host Keys Required**: Host A and Host B do not need SSH keys to each other. Both machines only trust the developer's client (iPhone/Mac) via their existing Keychain Ed25519 keys.
2. **Firewall Traversal**: Each host is already connected to the client over its own encrypted SSH tunnel (e.g. over Tailscale). The client simply pipes the output of Host A as formatted input into Host B.
3. **Auditing & Inspection**: Every message traveling between Host A and Host B passes through the client's router, ensuring human visibility, rate limiting, and approval checks.

### 3.2 Direct Host-to-Host Data Plane: mTLS 1.3 with Certificate Fingerprint Pinning
Tailscale, VPNs, or direct public IP addresses only provide Layer 3/4 packet transport. They do not provide application-layer identity, capability tokens, or granular authorization. Relying on network-level reachability alone is insecure.

For high-bandwidth data transfers (large git diffs, build artifacts, test logs) between Host A and Host B, the mesh establishes a direct **Mutual TLS 1.3 (mTLS)** stream without relying on public Certificate Authorities (CA):

```mermaid
sequenceDiagram
    participant A as Host A (Sender)
    participant C as Client (ssh-app iOS / Rust Core)
    participant B as Host B (Receiver)

    Note over A,C: Secure SSH Control Channel
    Note over C,B: Secure SSH Control Channel

    C->>A: Command: Prepare mTLS 1.3 Listener for Session
    A->>A: Generate ephemeral Ed25519 keypair & self-signed X.509 cert
    A->>A: Bind dynamic TCP listener (0.0.0.0:<port>)
    A-->>C: Response: { port, fingerprint_sha256, lease_token }

    C->>B: Command: Connect mTLS 1.3 to Host A (IP, port, expected_fingerprint, lease_token)
    B->>B: Generate ephemeral client Ed25519 cert

    Note over B,A: Direct TCP Socket over WAN, Tailscale, or LAN
    B->>A: TLS 1.3 Handshake (Client Hello + Client Cert)
    A->>B: TLS 1.3 Handshake (Server Hello + Server Cert)
    B->>B: Verify Server Cert matches expected_fingerprint
    A->>A: Verify Client Cert & validate lease_token
    Note over B,A: Secure, Authenticated, Encrypted mTLS 1.3 Stream
    A-->>B: Stream diffs / artifacts at line-rate
    A->>A: Tear down listener on transfer completion or timeout (60s)
```

#### Cryptographic Invariants of the mTLS 1.3 Data Plane
1. **Zero Trust in Network Layer**: Even if the connection passes through public internet, local untrusted Wi-Fi, or misconfigured VPN routes, traffic is encrypted with TLS 1.3 (AES-GCM / ChaCha20-Poly1305) with Perfect Forward Secrecy.
2. **Out-of-Band Fingerprint Validation**: The SHA-256 fingerprint of the ephemeral certificate is transported via the pre-authenticated, SSHSIG-verified SSH control channel. Man-in-the-Middle (MITM) attacks are mathematically impossible without compromising the client's Keychain private key.
3. **Short-Lived Leases**: Ephemeral listeners bind only for the duration of the transfer and terminate after 60 seconds of inactivity or upon transfer completion.
4. **Fallback Mechanism**: If direct IP/TCP reachability between Host A and Host B is blocked by symmetric NAT or restrictive egress policies, the data stream transparently falls back to client-relayed SSH port forwarding (`channel_open_direct_tcpip`).

---

### 3.3 Double-NAT & Disconnected Client Traversal: All Three Connectivity Strategies
When Host A (MacBook on home Wi-Fi) and Host B (MacBook/PC in an office/cafe across town) are both behind private NATs with no public static IP, and the iPhone is asleep in a pocket, direct inbound TCP connections fail.

The architecture implements **all three traversal strategies**, cascading from lowest latency to universal reach:

```mermaid
flowchart TD
    Start["Попытка соединения Host A <--> Host B"] --> S1{"Стратегия 1: P2P STUN Hole Punching"}
    S1 -- "Успех (Cone NAT)" --> P2P["Прямой P2P mTLS 1.3 туннель (Пинг мин, $0)"]
    S1 -- "Отказ (Symmetric NAT)" --> S2{"В профилях есть VPS с белым IP?"}
    S2 -- "Да (DigitalOcean и т.д.)" --> VPS["Стратегия 2: Blind TCP Relay через VPS (mTLS E2EE, $0 доп.)"]
    S2 -- "Нет" --> CF["Стратегия 3: Cloudflare WebSocket Relay (mTLS E2EE, $0 / Free Tier)"]
```

#### Strategy 1: Direct P2P NAT Hole Punching (STUN/ICE UDP)
* **Principle**: Neither host needs an open public port. Both hosts leverage public STUN infrastructure (`stun.cloudflare.com:3478`, `stun.l.google.com:19302`) to discover their public-facing reflexive IP:port mappings.
* **Handshake Flow**:
  1. During the 2-second room setup phase, the phone collects reflexive endpoints from Host A and Host B over their control SSH channels.
  2. The phone delivers Host B's endpoint to Host A, and Host A's endpoint to Host B.
  3. Host A and Host B simultaneously send UDP probe bursts to each other.
  4. Both home/office NAT routers record outbound mappings, opening temporary firewall holes in both directions.
  5. A bidirectional P2P UDP tunnel is established directly across the city.
  6. **Security**: **mTLS 1.3** runs inside the UDP packets. Eavesdroppers, ISPs, or STUN servers see only encrypted ciphertext.
* **Characteristics**: Maximum possible speed, minimum network latency, 0 servers required, $0 cost. Operates successfully in 85-90% of residential and mobile Wi-Fi setups.

#### Strategy 2: Blind TCP Relay via VPS (e.g. DigitalOcean in Netherlands)
* **Principle**: Utilizes any cloud virtual server with a public IP that already exists in the developer's `ServerProfilesManager`.
* **Handshake Flow**:
  1. The phone designates the user's configured VPS as the neutral rendezvous node.
  2. Host A initiates an *outbound* TCP connection to `vps.public.ip:9443`.
  3. Host B initiates an *outbound* TCP connection to `vps.public.ip:9443`.
  4. Both NAT routers seamlessly allow these outbound connections.
  5. The helper `ssh-app-host relay` on the VPS pairs the two incoming sockets and bridges them using the zero-copy Linux kernel `splice()` syscall.
  6. **Zero-Knowledge End-to-End Encryption (E2EE)**: The mTLS 1.3 session is negotiated *directly between Host A and Host B* through the bridged pipe. The VPS has zero decryption keys, holds no state in RAM, and cannot inspect or forge message payloads.
* **Characteristics**: 100% bypass of symmetric NAT, carrier-grade NAT (CGNAT), and enterprise firewalls. Runs autonomously for hours while the phone is locked.

#### Strategy 3: Serverless WebSocket Relay via Cloudflare Workers (Zero VPS, $0 Cost)
* **Principle**: When the user has no cloud VPS, the rendezvous bridge runs on the developer's existing Cloudflare account (co-located with the Push Relay from Feature 35) as a serverless WebSocket relay (`scripts/mesh-relay-worker`).
* **Handshake Flow**:
  1. Host A opens an outbound WebSocket connection: `wss://mesh-relay.<subdomain>.workers.dev/room/<id>?token=<ticket>&peer=A`.
  2. Host B opens an outbound WebSocket connection: `wss://mesh-relay.<subdomain>.workers.dev/room/<id>?token=<ticket>&peer=B`.
  3. The Cloudflare Worker accepts both connections, authenticates the capability tickets, and pairs them via standard `WebSocketPair` streaming.
  4. Binary WebSocket frames carry the raw **mTLS 1.3** records end-to-end.
  5. Every 30 seconds, the hosts exchange 2-byte WebSocket Ping/Pong frames to defeat Cloudflare's 100-second idle disconnect threshold.
* **Economics & Official Pricing**:
  * **Free Plan ($0.00 / month)**:
    * 100,000 requests per day (handshake = 1 request; continuous in-socket messages do NOT consume request quota).
    * Egress bandwidth: 100% free ($0 for all data transfer).
    * CPU time: Up to 10ms per event (proxying binary buffers takes <0.05ms).
    * Fits 100% of single-developer multi-laptop collaboration workflows at zero cost.
  * **Paid Plan ($5.00 / month)**:
    * Includes 10,000,000 requests/month + $0.30/million additional requests. Unlimited duration, zero bandwidth fees. Only needed if scaling to commercial multi-tenant SaaS.

#### Summary Comparison of Traversal Strategies

| Parameter | Strategy 1: P2P STUN | Strategy 2: VPS Blind Relay | Strategy 3: Cloudflare WS Relay |
| :--- | :--- | :--- | :--- |
| **Infrastructure Needed** | None ($0) | VPS with public IP | Free Cloudflare Account |
| **Monthly Cost** | **$0.00** | VPS cost (e.g. $4-6/mo) | **$0.00 (Free Tier)** |
| **NAT Penetration** | 85-90% (Cone NAT) | 100% (Any NAT/Firewall) | 100% (Any NAT/Firewall via 443) |
| **Phone Dependent** | **No (100% autonomous)** | **No (100% autonomous)** | **No (100% autonomous)** |
| **Encryption Level** | mTLS 1.3 E2EE | mTLS 1.3 E2EE (Zero Knowledge) | mTLS 1.3 E2EE (Zero Knowledge) |
| **Transfer Latency** | Lowest (Direct line) | Route dependent (via VPS) | Low (Cloudflare global Edge CDN) |

---

## 4. Domain Models & Rust/Swift Protocol Contracts

### 4.1 Swift Types (`MeshModels.swift`)

```swift
public struct MeshHostRef: Identifiable, Hashable {
    public let id: String          // e.g. "macbook-pro"
    public let hostname: String    // e.g. "100.82.14.12"
    public let label: String       // e.g. "MacBook Pro M3"
}

public struct MeshParticipant: Identifiable, Hashable {
    public let id: String          // e.g. "macbook-pro:claude-ui"
    public let host: MeshHostRef
    public let sessionName: String
    public let agentKind: AgentKind // .claudeCode, .antigravity, .localLlm, .customShell
    public var role: MeshRole       // .driver, .auditor, .consultant, .observer
    public var isWriteLockHolder: Bool
}

public enum MeshRole: String, CaseIterable, Identifiable {
    case driver = "driver"         // Has write permission to repo
    case auditor = "auditor"       // Automated code reviewer (read-only)
    case consultant = "consultant" // Provides second opinion on prompts
    case observer = "observer"     // Receives transcript, no output

    public var id: String { rawValue }
    public var title: String {
        switch self {
        case .driver: return "Driver (Write)"
        case .auditor: return "Auditor (Read-Only)"
        case .consultant: return "Consultant"
        case .observer: return "Observer"
        }
    }
}

public struct MeshMessage: Identifiable, Equatable {
    public let id: String
    public let sourceParticipant: MeshParticipant
    public let targetSelector: String // "@all", "@macbook/claude", "@gpu/qwen"
    public let timestamp: Date
    public let text: String
    public let toolCalls: [TranscriptToolCall]
    public let roundIndex: Int
}
```

### 4.2 Rust Core Interface (`crates/core/src/mesh.rs`)

```rust
use crate::error::SshCoreError;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HostSessionTarget {
    pub host_id: String,
    pub session_name: String,
    pub agent_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MeshRoomConfig {
    pub room_id: String,
    pub title: String,
    pub participants: Vec<HostSessionTarget>,
    pub max_rounds: u32,
    pub stop_on_consensus: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MeshDispatchPlan {
    pub target_sessions: Vec<HostSessionTarget>,
    pub formatted_payload: String,
    pub caller_host_id: String,
    pub round: u32,
}

/// Routes user input or cross-agent reply to target sessions across host boundaries.
pub fn plan_mesh_dispatch(
    config: &MeshRoomConfig,
    sender_session: Option<HostSessionTarget>,
    selector: &str, // "@all", "@macbook/claude", etc.
    raw_message: &str,
    current_round: u32,
) -> Result<MeshDispatchPlan, SshCoreError> {
    // Validates round limits, parses @mentions, wraps content in delimiter tags
    // and returns destination list with formatted bracketed-paste bytes.
    todo!()
}
```

---

## 5. Phased Implementation Roadmap

### Phase 1: Dual-Session Bridge (Claude + agy on one host)
- Paired tmux sessions `<session>-claude` and `<session>-agy`.
- Interleaved timeline viewer in the app with `[Pass to other]` relay button.

### Phase 2: Driver & Reviewer Dual Card
- One agent drives (`claude`), second agent audits (`agy`).
- Approval sheet displays side-by-side plan and automated audit verdict.

### Phase 3: MCP Peer Consultation Bridge
- Local stdio MCP server on the host: `consult_peer(target, prompt)`.
- On-demand calls folded into neat tool-call accordions.

### Phase 4: Local N-Session Mesh Room
- User creates a room on a single server, selecting any N active tmux sessions.
- In-memory room coordinator handles round limits and `@session` mentions.

### Phase 5: Cross-Server Federation
- Multi-host coordination in `TerminalSessionViewModel` / `MeshManager`.
- Routes messages between Server A (`SshSessionHandle`) and Server B (`SshSessionHandle`).
- Client-side relay: Server A writes text $\rightarrow$ Client receives turn $\rightarrow$ Client sends to Server B via PTY/stdin.
- Distributed write lease: Only one session across all servers may hold the git write lock for a shared repository.

---

## 6. Security & Concurrency Considerations

1. **Distributed Write Locks**:
   - Before an agent on Host A executes an `Edit` or `Write` tool, it must claim the repository lock file (`.git/ssh-app-mesh.lock`). If Host B operates on a shared network mount (NFS/SSHFS), write attempts without the lease fail immediately.
2. **Circuit Breaker (Anti-Runaway Token Protection)**:
   - Maximum autonomous rounds = 3 (configurable up to 6).
   - If two agents exchange replies without human input for 3 cycles, the room pauses automatically with status `[PAUSE] Round Limit Reached. Human review required.`.
3. **Cross-Host Sanitization**:
   - Messages passing from Host A to Host B are stripped of terminal control codes (`ESC[...]`) and formatted as sanitized markdown text wrapped in `<agent-context host="..." session="...">` containers to prevent prompt injection.
4. **Zero Battery Drain**:
   - Backgrounding the iOS app tears down all SSH TCP channels to all servers. Long-running tasks on Host A and Host B continue unimpeded inside `tmux`. On foregrounding, the client reconnects and syncs timelines across all participating hosts.

---

## Review (`rev`, 2026-10-20)

- [High] Section 3.3 says "Phone Dependent: No", but 3.1 routes every message through the client, and a backgrounded app closes its sockets. Only host-to-host data transfer is autonomous; agent-to-agent rounds are not. Say so.
- [High] "mTLS 1.3 inside UDP" after STUN hole punching needs QUIC or DTLS, not TLS over TCP. The helper has no TLS stack. Choose one transport and a library, or drop direct host-to-host (the SSH relay through the client is enough for phases 1-5).
- [High] The listener must not bind `0.0.0.0` on a laptop; bind the tailnet interface only.
- [High] `.git/ssh-app-mesh.lock` blocks nothing: the agents' own `Edit` and `Write` do not read it. Enforcement is only a `PreToolUse` hook that denies writes without the lease, or read-only reviewers.
- [Medium] The `<agent-context>` wrapper does not stop prompt injection between agents. State that cross-agent text is untrusted input and every relayed turn is shown to the user before it is sent on.
- [Medium] "85-90 % of NATs" and the Cloudflare plan limits are unsourced. Verify against the provider before relying on them.
- [Medium] Phases 4-5 do not fit one milestone; ship phases 1-2 first.

## Binding corrections after `rev` (2026-10-20)

These replace conflicting text above.

1. **Not phone-independent.** Every agent-to-agent round goes through the client (3.1); a backgrounded app closes its sockets, so rounds pause until the app is foreground again. "Phone Dependent: No" is true only for host-to-host file transfer. The text says so.
2. **One transport, or none.** Direct host-to-host (STUN hole punching, mTLS in UDP) is dropped from the first releases: it needs QUIC or DTLS and the helper has no such stack. The SSH relay through the client is the transport for phases 1-2. If direct transfer returns later, it is a separate spec that names the library.
3. **Listener binds the tailnet interface only**, never `0.0.0.0` on a laptop.
4. **Write exclusion is enforced by a `PreToolUse` hook** that denies `Edit`/`Write` in a leased path without the lease, or reviewers run read-only. A lock file alone blocks nothing.
5. **Cross-agent text is untrusted input.** The `<agent-context>` wrapper is a label, not a defence; every relayed turn is shown to the user and sent on only after approval (or an explicit per-mesh "auto-relay" switch that is off by default).
6. **Unsourced numbers** (NAT share, Cloudflare limits) are removed until checked against the provider.
7. **Scope:** ship phases 1-2 (two agents, relayed through the client, user in the loop). Phases 4-5 are separate milestones.
