#!/usr/bin/env python3
"""ssh-app agent hook (Claude Code + Google Antigravity).

Usage (configured by the app, never by hand):
    ssh-app-hook <claude|antigravity> <EventName>

Reads the agent's hook JSON on stdin, asks the local ssh-app approval broker
(Unix socket, mode 0600, line-delimited JSON) and prints the agent-specific
decision JSON on stdout.

Failure policy: if the broker is absent, untrusted, errors, or times out, the hook has
NO opinion and the agent applies its own native policy (Claude: no output;
Antigravity: "allow", which defers to the default policy). It never force-approves
(`auto_approve` only follows an explicit phone approval), and only an explicit phone
"deny" blocks.
"""
import json
import os
import socket
import stat
import sys
import time

SOCKET = os.environ.get("SSH_APP_SOCKET") or os.path.expanduser("~/.ssh-app/run/control.sock")
TIMEOUT = float(os.environ.get("SSH_APP_HOOK_TIMEOUT", "120"))
MAX_SUMMARY = 2000
ANTIGRAVITY_CMD_KEYS = ("CommandLine", "commandLine", "command", "Command", "TargetFile", "AbsolutePath", "path")


def summarize(tool_input):
    if isinstance(tool_input, dict):
        for key in ("command",) + ANTIGRAVITY_CMD_KEYS + ("file_path",):
            val = tool_input.get(key)
            if isinstance(val, str) and val:
                return val[:MAX_SUMMARY]
        return json.dumps(tool_input, sort_keys=True)[:MAX_SUMMARY]
    return str(tool_input)[:MAX_SUMMARY]


def socket_trusted():
    """The socket must be ours and private: owned by us, no group/other access, in a 0700 dir."""
    try:
        st = os.lstat(SOCKET)
        parent = os.lstat(os.path.dirname(SOCKET) or ".")
    except OSError:
        return False
    uid = os.geteuid()
    return (
        stat.S_ISSOCK(st.st_mode)
        and st.st_uid == uid
        and not st.st_mode & 0o077
        and stat.S_ISDIR(parent.st_mode)
        and parent.st_uid == uid
        and not parent.st_mode & 0o077
    )


def ask_broker(payload, wait):
    """Returns True/False for an explicit decision, None when there is no decision."""
    if not socket_trusted():
        return None
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    budget = TIMEOUT if wait else 3
    deadline = time.monotonic() + budget
    s.settimeout(budget)
    try:
        s.connect(SOCKET)
        s.sendall(json.dumps(payload).encode() + b"\n")
        if not wait:
            return None
        buf = b""
        while b"\n" not in buf and len(buf) < 65536:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return None  # overall deadline: a slow-drip broker must not stall the agent
            s.settimeout(remaining)
            chunk = s.recv(4096)
            if not chunk:
                break
            buf += chunk
        resp = json.loads(buf.decode().strip() or "{}")
        approved = resp.get("approved")
        return approved if isinstance(approved, bool) else None
    except Exception:
        return None
    finally:
        s.close()


def as_dict(value):
    return value if isinstance(value, dict) else {}


def main():
    agent = sys.argv[1] if len(sys.argv) > 1 else ""
    event = sys.argv[2] if len(sys.argv) > 2 else ""
    try:
        data = as_dict(json.load(sys.stdin))
    except Exception:
        data = {}

    if agent == "claude":
        tool = str(data.get("tool_name", ""))
        summary = summarize(data.get("tool_input", {}))
        cwd = str(data.get("cwd", ""))
        session = str(data.get("session_id", ""))
    elif agent == "antigravity":
        call = as_dict(data.get("toolCall"))
        tool = str(call.get("name", ""))
        summary = summarize(call.get("args", {}))
        paths = data.get("workspacePaths")
        cwd = str(paths[0]) if isinstance(paths, list) and paths else ""
        session = str(data.get("conversationId", ""))
    else:
        return 0

    base = {"agent": agent, "event": event, "tool": tool, "command": summary, "cwd": cwd, "session": session}

    if event in ("PermissionRequest", "PreToolUse"):
        approved = ask_broker(dict(base, method="request_approval"), wait=True)
        if agent == "claude":
            if approved is None:
                return 0  # no opinion -> Claude shows its normal permission prompt
            decision = {"behavior": "allow"} if approved else {
                "behavior": "deny", "message": "Denied from ssh-app on your phone"}
            out = {"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": decision}}
        else:
            if approved is None:
                out = {"decision": "allow", "reason": "ssh-app broker unavailable; deferring to default policy"}
            elif approved:
                out = {"decision": "auto_approve", "reason": "Approved from ssh-app"}
            else:
                out = {"decision": "deny", "reason": "Denied from ssh-app on your phone"}
        sys.stdout.write(json.dumps(out))
        return 0

    # Notification / Stop and other passive events: fire-and-forget.
    ask_broker(dict(base, method="notify"), wait=False)
    if agent == "antigravity" and event == "Stop":
        sys.stdout.write(json.dumps({"decision": "stop"}))
    return 0


def safe_main():
    """Any unexpected error means "no opinion" (exit 0, no decision), never a crash/traceback."""
    try:
        return main()
    except Exception:
        return 0


if __name__ == "__main__":
    sys.exit(safe_main())
