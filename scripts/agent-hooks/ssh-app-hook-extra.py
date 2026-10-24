#!/usr/bin/env python3
"""ssh-app hook extra-command wrapper.

The agent runs this instead of the plain ssh-app hook when the user added a command to an event:

    ssh-app-hook-extra --cmd <base64url> --order <before|after> --stop <0|1> --timeout <1-60>
                       --agent <name> --event <Event> -- <the ordinary hook command and its arguments>

It reads the event JSON from stdin once, runs the user's command (`/bin/sh -c`, the event JSON on its
stdin, output not used) and the ordinary hook in the chosen order, and passes the hook's stdout and
exit code on. The extra command never changes the hook's answer, with one exception: `--stop 1` with
`--order before` on a blocking event makes a failed or timed-out command deny the action, and the
hook is not run. Failures are noted in ~/.ssh-app/run/extra.log (capped); nothing here ever raises.
"""
import base64
import json
import os
import signal
import subprocess
import sys
import threading

# The extra command sees at most this much of the event; the hook always sees all of it (a cut-off
# event would make the hook fail to parse it and have no opinion, which lets a guarded call through).
MAX_EXTRA_STDIN = 1 << 20
MAX_ERR = 300
MAX_LOG = 64 * 1024
BLOCKING = ("PermissionRequest", "PreToolUse")
CLAUDE_STYLE = ("claude", "codex", "qwen")
HOME_DIR = os.environ.get("SSH_APP_HOME") or os.path.expanduser("~/.ssh-app")
LOG = os.path.join(HOME_DIR, "run", "extra.log")


def log(line):
    """One line, newest last; the file is cut when it grows past the cap. Best effort."""
    try:
        os.makedirs(os.path.dirname(LOG), mode=0o700, exist_ok=True)
        try:
            if os.path.getsize(LOG) > MAX_LOG:
                os.unlink(LOG)
        except OSError:
            pass
        fd = os.open(LOG, os.O_WRONLY | os.O_APPEND | os.O_CREAT | getattr(os, "O_NOFOLLOW", 0), 0o600)
        with os.fdopen(fd, "w") as f:
            f.write("".join(c if c.isprintable() else " " for c in line)[:300] + "\n")
    except Exception:
        pass


def deny_output(agent, event, reason):
    if agent == "crush":
        return {"version": 1, "decision": "deny", "reason": reason}
    if agent in CLAUDE_STYLE:
        if event == "PermissionRequest":
            return {"hookSpecificOutput": {"hookEventName": "PermissionRequest",
                                           "decision": {"behavior": "deny", "message": reason}}}
        return {"hookSpecificOutput": {"hookEventName": event, "permissionDecision": "deny",
                                       "permissionDecisionReason": reason}}
    return {"decision": "deny", "reason": reason}


def parse(argv):
    """The wrapper's own options and the hook command after `--`; None when anything is off."""
    if "--" not in argv:
        return None
    split = argv.index("--")
    opts, inner = argv[:split], argv[split + 1:]
    if not inner or len(opts) % 2:
        return None
    pairs = dict(zip(opts[0::2], opts[1::2]))
    try:
        cmd = base64.urlsafe_b64decode(pairs["--cmd"] + "=" * (-len(pairs["--cmd"]) % 4)).decode("utf-8")
        order = pairs["--order"]
        stop = pairs["--stop"] == "1"
        timeout = float(pairs["--timeout"])
        agent, event = pairs["--agent"], pairs["--event"]
    except Exception:
        return None
    if order not in ("before", "after") or not 1 <= timeout <= 60 or not cmd.strip() or "\n" in cmd:
        return None
    return {"cmd": cmd, "order": order, "stop": stop and order == "before" and event in BLOCKING,
            "timeout": timeout, "agent": agent, "event": event, "inner": inner}


def run_extra(cmd, data, timeout):
    """True when the command exited 0 in time. The whole process group is killed on a timeout; a
    child it leaves behind in the background does not hold the wrapper (stderr is drained by a thread
    that keeps only the first bytes)."""
    try:
        proc = subprocess.Popen(["/bin/sh", "-c", cmd], stdin=subprocess.PIPE, stdout=subprocess.DEVNULL,
                                stderr=subprocess.PIPE, start_new_session=True)
    except Exception as e:
        log("extra: cannot start: %s" % e)
        return False
    kept = bytearray()

    def drain():
        try:
            while True:
                chunk = proc.stderr.read(4096)
                if not chunk:
                    return
                if len(kept) < MAX_ERR:
                    kept.extend(chunk[:MAX_ERR - len(kept)])
        except Exception:
            pass

    reader = threading.Thread(target=drain, daemon=True)
    reader.start()

    def feed():
        try:
            proc.stdin.write(data)
        except Exception:
            pass
        try:
            proc.stdin.close()
        except Exception:
            pass

    writer = threading.Thread(target=feed, daemon=True)
    writer.start()
    try:
        code = proc.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except Exception:
            pass
        proc.wait()
        log("extra: timed out after %ss" % timeout)
        return False
    except Exception as e:
        log("extra: %s" % e)
        return False
    reader.join(0.2)
    if code != 0:
        log("extra: exit %s: %s" % (code, bytes(kept).decode("utf-8", "replace")))
        return False
    return True


def run_hook(inner, data):
    """The ordinary hook with the same stdin; its stdout and exit code are passed on."""
    try:
        proc = subprocess.run(inner, input=data, stdout=subprocess.PIPE)
    except Exception as e:
        log("hook: cannot start: %s" % e)
        return b"", 0
    return proc.stdout, proc.returncode


def main(argv):
    cfg = parse(argv)
    if cfg is None:
        return 0  # no opinion: the agent applies its own policy
    data = sys.stdin.buffer.read()
    extra_data = data[:MAX_EXTRA_STDIN]
    if cfg["order"] == "before":
        ok = run_extra(cfg["cmd"], extra_data, cfg["timeout"])
        if not ok and cfg["stop"]:
            sys.stdout.write(json.dumps(deny_output(cfg["agent"], cfg["event"], "Blocked by the extra hook command")))
            sys.stdout.flush()
            return 0
        out, code = run_hook(cfg["inner"], data)
    else:
        out, code = run_hook(cfg["inner"], data)
        run_extra(cfg["cmd"], extra_data, cfg["timeout"])
    sys.stdout.buffer.write(out)
    sys.stdout.flush()
    return code


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except SystemExit:
        raise
    except Exception:
        sys.exit(0)
