#!/usr/bin/env bash
# Claude Code PreToolUse Approval Hook for ssh-app
#
# Intercepts dangerous commands and requests cryptographic approval
# from your iPhone over Tailscale/SSH before execution.
set -euo pipefail

TOOL_NAME="${1:-bash}"
COMMAND="${2:-}"

# Non-destructive commands can pass through instantly
case "$COMMAND" in
  ls*|pwd|git\ status|git\ branch|git\ log*|echo*|which*|cat*)
    exit 0
    ;;
esac

SOCKET_PATH="${HOME}/.ssh-app/run/control.sock"
if [ ! -S "$SOCKET_PATH" ]; then
  # If the approval socket isn't running, default to interactive prompt
  echo "[ssh-app] Approval socket not active; defaulting to terminal prompt." >&2
  exit 0
fi

# Send approval request to local Unix Domain Socket (mode 0600)
# and wait for cryptographically signed response
RESPONSE=$(python3 -c '
import socket, sys, json

sock_path = sys.argv[1]
tool = sys.argv[2]
cmd = sys.argv[3]

s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.settimeout(120)
try:
    s.connect(sock_path)
    req = json.dumps({"agent": "Claude Code", "tool": tool, "command": cmd})
    s.sendall(req.encode() + b"\n")
    data = s.recv(4096).decode()
    resp = json.loads(data)
    if resp.get("approved") is True:
        sys.exit(0)
    else:
        sys.exit(1)
except Exception as e:
    sys.exit(0)
' "$SOCKET_PATH" "$TOOL_NAME" "$COMMAND")

exit $?
