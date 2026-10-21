# TODO

## Agent relay room (M40) - branch `feature/agent-relay`

Carry the final answer of one agent into another agent's tab (draft stash and restore, bracketed paste, labelled and
sanitised message), with a clean room feed without thinking and tool calls, round limits and profile confirmation.

- Branch: `feature/agent-relay` (not merged, not pushed). Work happens there, not on `main`.
- Spec: `docs/features/51-two-agent-relay.md` (on the branch). Overview with diagrams: `docs/architecture/agent-relay-overview.md` (on `main`).
- Status: spec written, not reviewed, no code.
- Next: phase 0 live checks (clear keys, long paste, message while busy, timing) in a private tmux with `claude-personal` and `agy`; then core, host `relay-send`, bridge, iOS room screen.
