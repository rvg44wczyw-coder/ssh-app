# TODO

## Agent relay room (M40) - branch `feature/mesh`

Carry the final answer of one agent into another agent's tab (draft stash and restore, bracketed paste, labelled and
sanitised message), with a clean room feed without thinking and tool calls, round limits and profile confirmation.

- Branch: `feature/mesh` (merged there together with M26; not pushed). Work happens there, not on `main`.
- Spec: `docs/features/51-two-agent-relay.md` (on `feature/mesh`). Overview with diagrams: `docs/architecture/agent-relay-overview.md` (on `main`).
- Status: spec written, not reviewed, no code.
- Next: phase 0 live checks (clear keys, long paste, message while busy, timing) in a private tmux with `claude-personal` and `agy`; then core, host `relay-send`, bridge, iOS room screen.

## Multi-agent mesh (M26) - branch `feature/mesh`

Rooms of N agent sessions on one host and across hosts (driver and reviewer, peer consultation, write leases enforced by a
hook, round limits, cross-host federation through the client). Large, with a lot of separate work.

- Branch: `feature/mesh` (local, not pushed). The spec and the roadmap entry were removed from `main` and live only there.
- Spec: `docs/features/37-multi-agent-trio-chat.md` (on the branch).
- Contains: Agent relay room (M40, merged into this branch) as its phase 1-2 base; the Mesh module of M35 (`docs/features/46-modular-configuration-and-power-settings.md`, stays on `main`).
- Phase 3 (MCP peer) must be reconsidered: MCP was not pursued (`docs/architecture/mcp-channel-study.md`).
