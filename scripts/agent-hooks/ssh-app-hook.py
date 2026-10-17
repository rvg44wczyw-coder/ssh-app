#!/usr/bin/env python3
"""ssh-app agent hook (Claude Code + Google Antigravity) and approval queue (Feature 13).

Usage (configured by the app, never by hand):
    ssh-app-hook <claude|codex|antigravity|grok> <EventName>     agent hook (reads the hook JSON on stdin)
    ssh-app-hook ctl <list|decide|ack|status>          used by the app over SSH (data on stdin)

An approval request is a file in ~/.ssh-app/run/pending (0700 dirs, 0600 files). The phone
answers with a decision file signed (OpenSSH SSHSIG) by its Ed25519 key; this process verifies
the signature with `ssh-keygen -Y verify` against ~/.ssh-app/allowed_signers using its OWN copy
of the request, and only then prints the agent-specific decision JSON.

Failure policy: if the queue is absent/untrusted, ssh-keygen is missing, nothing valid arrives
in time, or anything errors, the hook has NO opinion and the agent applies its own native
policy (Claude: no output; Antigravity: "ask", which prompts in the terminal; "allow" would auto-approve). It never
force-approves (`allow` is only printed after a verified phone approval); only a verified phone
"deny" blocks.
"""
import hashlib
import json
import os
import re
import shutil
import signal
import stat
import subprocess
import sys
import time

HOME_DIR = os.environ.get("SSH_APP_HOME") or os.path.expanduser("~/.ssh-app")
RUN = os.path.join(HOME_DIR, "run")
PENDING = os.path.join(RUN, "pending")
DECISIONS = os.path.join(RUN, "decisions")
ALLOWED = os.path.join(HOME_DIR, "allowed_signers")
GUARD = os.path.join(HOME_DIR, "guard.json")
SESSIONS = os.path.join(RUN, "sessions")
try:
    TIMEOUT = float(os.environ.get("SSH_APP_HOOK_TIMEOUT", "120"))
except ValueError:
    TIMEOUT = 120.0
MAX_COMMAND = 2000
MAX_FIELD = 512
MAX_FILE = 16384
MAX_SIG = 8192
MAX_PENDING_FILES = 100
MAX_LIST = 20
MAX_DECISIONS_PER_ID = 5
MAX_AGE = 120  # a decision may be at most this many seconds newer than its request
NOTIFY_TTL = 600
NAMESPACE = "ssh-app-approval"
PRINCIPAL = "ssh-app"
ID_RE = re.compile(r"^[0-9a-f]{32}$")
PENDING_RE = re.compile(r"^([0-9a-f]{32})\.json$")
ANTIGRAVITY_CMD_KEYS = ("CommandLine", "commandLine", "command", "Command", "TargetFile", "AbsolutePath", "path")


def clean(value, limit):
    """Valid UTF-8 text (lone surrogates replaced), truncated to `limit` characters."""
    text = value if isinstance(value, str) else str(value)
    return text.encode("utf-8", "replace").decode("utf-8")[:limit]


def summarize(tool_input):
    if isinstance(tool_input, dict):
        for key in ("command",) + ANTIGRAVITY_CMD_KEYS + ("file_path",):
            val = tool_input.get(key)
            if isinstance(val, str) and val:
                return clean(val, MAX_COMMAND)
        return clean(json.dumps(tool_input, sort_keys=True), MAX_COMMAND)
    return clean(tool_input, MAX_COMMAND)


def request_digest(agent, tool, cwd, command):
    """Binds everything the user is shown (Feature 13 section 4.1). Every field is length-prefixed
    (`<utf8 byte length>:<bytes>\\n`) so no split between fields can collide with another."""
    h = hashlib.sha256()
    for field in (agent, tool, cwd, command):
        raw = field.encode("utf-8")
        h.update(b"%d:" % len(raw))
        h.update(raw)
        h.update(b"\n")
    return h.hexdigest()


def private_dir(path):
    """Directory owned by us with no group/other access, not a symlink."""
    try:
        st = os.lstat(path)
    except OSError:
        return False
    return stat.S_ISDIR(st.st_mode) and st.st_uid == os.geteuid() and not st.st_mode & 0o077


def queue_trusted():
    return all(private_dir(d) for d in (HOME_DIR, RUN, PENDING, DECISIONS))


def allowed_signers_trusted():
    try:
        st = os.lstat(ALLOWED)
    except OSError:
        return False
    return stat.S_ISREG(st.st_mode) and st.st_uid == os.geteuid() and not st.st_mode & 0o022


def keygen_path():
    for cand in ("/usr/bin/ssh-keygen", "/usr/local/bin/ssh-keygen"):
        if os.access(cand, os.X_OK):
            return cand
    return shutil.which("ssh-keygen")


def read_small(path, limit=MAX_FILE):
    """Reads a regular file (no symlinks) of at most `limit` bytes, else None."""
    try:
        # O_NONBLOCK: opening a FIFO for reading must not block waiting for a writer.
        fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK | getattr(os, "O_NOFOLLOW", 0))
    except OSError:
        return None
    try:
        st = os.fstat(fd)
        if not stat.S_ISREG(st.st_mode) or st.st_uid != os.geteuid() or st.st_size > limit:
            return None
        return os.read(fd, limit + 1)
    except OSError:
        return None
    finally:
        os.close(fd)


def write_new(path, data):
    """Creates `path` exclusively with mode 0600."""
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        os.write(fd, data)
    finally:
        os.close(fd)


def unlink(path):
    try:
        os.unlink(path)
    except OSError:
        pass


def sweep_stale(now):
    """Removes leftovers of killed hooks and unacknowledged notifications."""
    for directory in (PENDING, DECISIONS, RUN):  # RUN holds only transient .sig-* files
        try:
            names = os.listdir(directory)
        except OSError:
            continue
        for name in names:
            path = os.path.join(directory, name)
            try:
                st = os.lstat(path)
                if stat.S_ISREG(st.st_mode) and now - st.st_mtime > NOTIFY_TTL + MAX_AGE:
                    os.unlink(path)
            except OSError:
                pass


def verify_decision(req, raw):
    """True/False for a verified decision, None if the file is not acceptable."""
    try:
        dec = json.loads(raw.decode("utf-8"))
    except Exception:
        return None
    if not isinstance(dec, dict) or dec.get("id") != req["id"]:
        return None
    approved, dts, sig = dec.get("approved"), dec.get("ts"), dec.get("signature")
    if not isinstance(approved, bool) or isinstance(dts, bool) or not isinstance(dts, int):
        return None
    if not isinstance(sig, str) or not sig or len(sig) > MAX_SIG:
        return None
    if not (req["ts"] <= dts <= int(time.time()) and dts - req["ts"] <= MAX_AGE):
        return None
    keygen = keygen_path()
    if not keygen or not allowed_signers_trusted():
        return None
    message = "SSH_APP_APPROVAL_V1:%s:%s:%s:%d:%s" % (
        req["id"], req["command_sha256"], req["nonce"], dts, "true" if approved else "false")
    sigfile = os.path.join(RUN, ".sig-%s" % os.urandom(8).hex())
    try:
        write_new(sigfile, sig.encode("utf-8", "replace"))
        proc = subprocess.run(
            [keygen, "-Y", "verify", "-f", ALLOWED, "-I", PRINCIPAL, "-n", NAMESPACE, "-s", sigfile],
            input=message.encode("utf-8"), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            timeout=5)
        return approved if proc.returncode == 0 else None
    except Exception:
        return None
    finally:
        unlink(sigfile)


def make_request(kind, base, now):
    agent = clean(base["agent"], MAX_FIELD)
    tool = clean(base["tool"], MAX_FIELD)
    cwd = clean(base["cwd"], MAX_FIELD)
    command = clean(base["command"], MAX_COMMAND)
    ttl = int(TIMEOUT) if kind == "approval" else NOTIFY_TTL
    return {
        "v": 1, "id": os.urandom(16).hex(), "kind": kind, "agent": agent,
        "event": clean(base["event"], MAX_FIELD), "tool": tool, "command": command, "cwd": cwd,
        "session": clean(base["session"], MAX_FIELD), "ts": now, "nonce": os.urandom(16).hex(),
        "command_sha256": request_digest(agent, tool, cwd, command), "expires": now + ttl,
    }


def submit(kind, base, wait):
    """Queues a request. For approvals waits for a verified decision: True/False, else None."""
    if not queue_trusted():
        return None
    now = int(time.time())
    sweep_stale(now)
    try:
        if len(os.listdir(PENDING)) >= MAX_PENDING_FILES:
            return None
    except OSError:
        return None
    req = make_request(kind, base, now)
    path = os.path.join(PENDING, req["id"] + ".json")
    tmp = os.path.join(PENDING, ".tmp-" + req["id"])
    try:
        write_new(tmp, json.dumps(req).encode("utf-8"))
        os.rename(tmp, path)
    except OSError:
        unlink(tmp)
        return None
    if not wait:
        return None  # notify entries are removed by the phone (ack) or the stale sweep
    deadline = time.monotonic() + TIMEOUT
    prefix = req["id"] + "."
    try:
        while time.monotonic() < deadline:
            try:
                names = sorted(n for n in os.listdir(DECISIONS) if n.startswith(prefix))
            except OSError:
                return None
            for name in names:
                if not re.match(r"^[0-9a-f]{32}\.[0-9a-f]{16}\.json$", name):
                    continue
                dpath = os.path.join(DECISIONS, name)
                raw = read_small(dpath)
                unlink(dpath)
                if raw is None:
                    continue
                verdict = verify_decision(req, raw)
                if verdict is not None:
                    return verdict
            time.sleep(0.5)
        return None
    finally:
        unlink(path)
        try:
            for name in os.listdir(DECISIONS):
                if name.startswith(prefix):
                    unlink(os.path.join(DECISIONS, name))
        except OSError:
            pass


# ---------------------------------------------------------------------------------------------
# Host guard: deny rules kept in ~/.ssh-app/guard.json ({"deny": ["Bash(rm:*)", "Read(~/.ssh/**)"]}),
# checked here before anything reaches the phone. Same rule shape as the agents' permission rules.
# A rule that cannot be understood is skipped; the guard only ever blocks, never approves.
# ---------------------------------------------------------------------------------------------
RULE_RE = re.compile(r"^([A-Za-z_][A-Za-z0-9_]*)(?:\((.*)\))?$", re.S)
SHELL_TOOLS = ("Bash", "run_command", "bash", "shell")
EDIT_TOOLS = ("Edit", "Write", "MultiEdit", "NotebookEdit", "write_to_file", "replace_file_content", "edit_file")
READ_TOOLS = ("Read", "view_file", "read_file")
MAX_RULES = 200


def load_guard():
    """The parsed guard file ({} when absent, not ours, or not a JSON object)."""
    raw = read_small(GUARD, 65536)
    if raw is None or not private_dir(HOME_DIR):
        return {}
    try:
        data = json.loads(raw.decode("utf-8"))
    except Exception:
        return {}
    return data if isinstance(data, dict) else {}


def guard_rules(guard, key):
    rules = guard.get(key)
    if not isinstance(rules, list):
        return []
    return [r for r in rules if isinstance(r, str) and 0 < len(r) <= 400][:MAX_RULES]


def guard_text(value):
    return clean(value, 2000) if isinstance(value, str) else ""


def tool_kind(tool):
    if tool in SHELL_TOOLS:
        return "Bash"
    if tool in EDIT_TOOLS:
        return "Edit"
    if tool in READ_TOOLS:
        return "Read"
    return tool


def rule_tool_matches(name, tool):
    """`Edit` also covers Write/MultiEdit (as in Claude's rules); `Read` covers read tools."""
    return name == tool or name == tool_kind(tool)


def glob_regex(pattern):
    out, i = [], 0
    while i < len(pattern):
        c = pattern[i]
        if pattern.startswith("**/", i):
            out.append("(?:.*/)?")
            i += 3
            continue
        if c == "*":
            if pattern.startswith("**", i):
                out.append(".*")
                i += 2
                continue
            out.append("[^/]*")
        elif c == "?":
            out.append("[^/]")
        else:
            out.append(re.escape(c))
        i += 1
    return re.compile("^" + "".join(out) + "$", re.S)


def shell_segments(command):
    return [seg.strip() for seg in re.split(r"\n|;|&&|\|\||\||&", command) if seg.strip()]


def bash_spec_matches(spec, command):
    for seg in [command] + shell_segments(command):
        if spec.endswith(":*"):
            prefix = spec[:-2]
            if seg == prefix or seg.startswith(prefix + " "):
                return True
        elif glob_regex(spec).match(seg):
            return True
    return False


def path_spec_matches(spec, path, cwd):
    if not path:
        return False
    home = os.path.expanduser("~")
    spec = home + spec[1:] if spec == "~" or spec.startswith("~/") else spec
    if not spec.startswith("/"):
        spec = os.path.join(cwd or "", spec[2:] if spec.startswith("./") else spec)
    target = path if path.startswith("/") else os.path.join(cwd or "", path)
    pattern = glob_regex(os.path.normpath(spec) if "*" not in spec else spec)
    candidates = {os.path.normpath(target)}
    try:
        candidates.add(os.path.realpath(target))
    except OSError:
        pass
    return any(pattern.match(c) for c in candidates)


def guard_match(rules, tool, tool_input, cwd):
    """The first rule of `rules` that matches this tool call, else None."""
    if not rules:
        return None
    ti = tool_input if isinstance(tool_input, dict) else {}
    command = ""
    path = ""
    for key in ("command",) + ANTIGRAVITY_CMD_KEYS[:4]:
        if isinstance(ti.get(key), str) and ti[key]:
            command = ti[key]
            break
    for key in ("file_path", "path", "AbsolutePath", "TargetFile", "notebook_path"):
        if isinstance(ti.get(key), str) and ti[key]:
            path = ti[key]
            break
    for rule in rules:
        m = RULE_RE.match(rule.strip())
        if not m:
            continue
        name, spec = m.group(1), m.group(2)
        if not rule_tool_matches(name, tool):
            continue
        if spec is None or spec == "":
            return rule
        kind = tool_kind(tool)
        try:
            if kind == "Bash" and bash_spec_matches(spec, command):
                return rule
            if kind in ("Edit", "Read") and path_spec_matches(spec, path, cwd):
                return rule
        except re.error:
            continue
    return None


def allow_output(agent, event, reason):
    """The agent-specific "allow without asking" answer, or None where the agent cannot do that."""
    if agent in ("claude", "codex"):
        if event == "PermissionRequest":
            return {"hookSpecificOutput": {"hookEventName": "PermissionRequest",
                                           "decision": {"behavior": "allow"}}}
        return {"hookSpecificOutput": {"hookEventName": event, "permissionDecision": "allow",
                                       "permissionDecisionReason": reason}}
    if agent == "antigravity":
        return {"decision": "allow", "reason": reason}
    return None


def ask_output(agent, event, reason):
    """Hand the decision to the user even where the agent's mode would not ask. None = go on to
    the phone as usual (PermissionRequest only fires when the agent asks anyway)."""
    if agent == "claude" and event == "PreToolUse":
        return {"hookSpecificOutput": {"hookEventName": event, "permissionDecision": "ask",
                                       "permissionDecisionReason": reason}}
    if agent == "antigravity":
        return {"decision": "ask", "reason": reason}
    return None


def deny_output(agent, event, reason):
    if agent in ("claude", "codex"):
        if event == "PermissionRequest":
            return {"hookSpecificOutput": {"hookEventName": "PermissionRequest",
                                           "decision": {"behavior": "deny", "message": reason}}}
        return {"hookSpecificOutput": {"hookEventName": event, "permissionDecision": "deny",
                                       "permissionDecisionReason": reason}}
    return {"decision": "deny", "reason": reason}


# ---------------------------------------------------------------------------------------------
# Session registry: SessionStart notes which tmux session an agent session runs in, SessionEnd
# removes it. The app reads it with `ctl sessions`.
# ---------------------------------------------------------------------------------------------
def session_file(agent, session):
    return os.path.join(SESSIONS, hashlib.sha256(("%s\0%s" % (agent, session)).encode("utf-8")).hexdigest()[:32] + ".json")


def tmux_session_name():
    if not os.environ.get("TMUX"):
        return ""
    try:
        out = subprocess.run(["tmux", "display-message", "-p", "#{session_name}"], capture_output=True,
                             timeout=3, text=True)
        return clean(out.stdout.strip(), 100) if out.returncode == 0 else ""
    except Exception:
        return ""


def register_session(agent, event, session, cwd):
    if not session or not private_dir(HOME_DIR):
        return
    try:
        os.makedirs(RUN, mode=0o700, exist_ok=True)
        os.makedirs(SESSIONS, mode=0o700, exist_ok=True)
        if not private_dir(SESSIONS):
            return
        path = session_file(agent, session)
        if event == "SessionEnd":
            unlink(path)
            return
        if len(os.listdir(SESSIONS)) >= MAX_PENDING_FILES:
            return
        record = {"v": 1, "agent": agent, "session": clean(session, MAX_FIELD), "cwd": clean(cwd, MAX_FIELD),
                  "tmux": tmux_session_name(), "ts": int(time.time())}
        tmp = os.path.join(SESSIONS, ".tmp-" + os.urandom(8).hex())
        write_new(tmp, json.dumps(record).encode("utf-8"))
        os.rename(tmp, path)
    except OSError:
        pass


def list_sessions():
    items = []
    if not private_dir(SESSIONS):
        return items
    cutoff = int(time.time()) - 7 * 86400
    for name in sorted(os.listdir(SESSIONS))[:MAX_PENDING_FILES]:
        if not re.match(r"^[0-9a-f]{32}\.json$", name):
            continue
        raw = read_small(os.path.join(SESSIONS, name))
        try:
            rec = json.loads(raw.decode("utf-8")) if raw else None
        except Exception:
            rec = None
        if isinstance(rec, dict) and isinstance(rec.get("ts"), int) and rec["ts"] >= cutoff:
            items.append({k: rec.get(k) for k in ("agent", "session", "cwd", "tmux", "ts")})
        elif rec is not None:
            unlink(os.path.join(SESSIONS, name))
    return items


def as_dict(value):
    return value if isinstance(value, dict) else {}


def main():
    agent = sys.argv[1] if len(sys.argv) > 1 else ""
    event = sys.argv[2] if len(sys.argv) > 2 else ""
    if agent == "ctl":
        return ctl(event)
    try:
        data = as_dict(json.load(sys.stdin))
    except Exception:
        data = {}

    if agent in ("claude", "codex"):  # Codex uses Claude's payload and PermissionRequest reply
        tool = str(data.get("tool_name", ""))
        tool_input = data.get("tool_input", {})
        summary = summarize(tool_input)
        cwd = str(data.get("cwd", ""))
        session = str(data.get("session_id", ""))
    elif agent == "antigravity":
        call = as_dict(data.get("toolCall"))
        tool = str(call.get("name", ""))
        tool_input = call.get("args", {})
        summary = summarize(tool_input)
        paths = data.get("workspacePaths")
        cwd = str(paths[0]) if isinstance(paths, list) and paths else ""
        session = str(data.get("conversationId", ""))
    elif agent == "grok":
        tool = str(data.get("toolName", ""))
        tool_input = data.get("toolInput", {})
        summary = summarize(tool_input)
        cwd = str(data.get("cwd", ""))
        session = str(data.get("sessionId", ""))
    else:
        return 0

    base = {"agent": agent, "event": event, "tool": tool, "command": summary, "cwd": cwd, "session": session}

    if event in ("SessionStart", "SessionEnd"):
        register_session(agent, event, session, cwd)
        text = guard_text((as_dict(load_guard().get("context"))).get("SessionStart"))
        if event == "SessionStart" and text and agent == "claude":
            sys.stdout.write(json.dumps({"hookSpecificOutput": {"hookEventName": "SessionStart",
                                                                "additionalContext": text}}))
        return 0

    if event == "UserPromptSubmit":
        # Text the user wrote in the app is added to the prompt's context. Never the prompt itself.
        text = guard_text((as_dict(load_guard().get("context"))).get("UserPromptSubmit"))
        if text and agent == "claude":
            sys.stdout.write(json.dumps({"hookSpecificOutput": {"hookEventName": "UserPromptSubmit",
                                                                "additionalContext": text}}))
        return 0

    if event in ("PermissionRequest", "PreToolUse"):
        guard = load_guard()
        hit = guard_match(guard_rules(guard, "deny"), tool, tool_input, cwd)
        if hit is not None:
            reason = "Blocked by a host guard rule: " + clean(hit, 120)
            sys.stdout.write(json.dumps(deny_output(agent, event, reason)))
            return 0
        hit = guard_match(guard_rules(guard, "ask"), tool, tool_input, cwd)
        if hit is not None:
            out = ask_output(agent, event, "A host guard rule asks you first: " + clean(hit, 120))
            if out is not None:
                sys.stdout.write(json.dumps(out))
                return 0
        else:
            hit = guard_match(guard_rules(guard, "allow"), tool, tool_input, cwd)
            if hit is not None:
                out = allow_output(agent, event, "Allowed by a host guard rule: " + clean(hit, 120))
                if out is not None:
                    sys.stdout.write(json.dumps(out))
                    return 0
        approved = submit("approval", base, wait=True)
        if agent == "grok":
            # Grok can only block: deny is explicit, everything else (including a phone approval)
            # lets the tool continue into Grok's own flow; failures fail open on its side anyway.
            if approved is False:
                sys.stdout.write(json.dumps({"decision": "deny", "reason": "Denied from ssh-app on your phone"}))
            return 0
        if agent in ("claude", "codex"):
            if approved is None:
                return 0  # no opinion -> Claude shows its normal permission prompt
            if event == "PreToolUse":
                out = {"hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "allow" if approved else "deny",
                    "permissionDecisionReason": "Approved from ssh-app" if approved else "Denied from ssh-app on your phone"}}
            else:
                decision = {"behavior": "allow"} if approved else {
                    "behavior": "deny", "message": "Denied from ssh-app on your phone"}
                out = {"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": decision}}
        else:
            if approved is None:
                # "allow" would auto-approve (agy docs); "ask" hands the decision back to the user.
                out = {"decision": "ask", "reason": "ssh-app broker unavailable; asking you in the terminal"}
            elif approved:
                out = {"decision": "allow", "reason": "Approved from ssh-app"}
            else:
                out = {"decision": "deny", "reason": "Denied from ssh-app on your phone"}
        sys.stdout.write(json.dumps(out))
        return 0

    # Stop: the user may have asked for one more round (once per stop; the flag below is what
    # keeps this from looping when the agent stops again after that round).
    if event == "Stop" and agent == "claude" and data.get("stop_hook_active") is not True:
        message = guard_text(as_dict(load_guard().get("on_stop")).get("message"))
        if message:
            sys.stdout.write(json.dumps({"decision": "block", "reason": message}))
            return 0

    # Notification / Stop and other passive events: fire-and-forget.
    submit("notify", base, wait=False)
    if agent == "antigravity" and event == "Stop":
        sys.stdout.write(json.dumps({"decision": "stop"}))
    return 0


def reply(obj):
    sys.stdout.write(json.dumps(obj))
    return 0


def read_stdin_json():
    raw = sys.stdin.buffer.read(MAX_FILE + 1)
    if len(raw) > MAX_FILE:
        return None
    try:
        return as_dict(json.loads(raw.decode("utf-8")))
    except Exception:
        return None


def load_pending(name, now):
    """A valid, unexpired pending file as a dict, else None."""
    match = PENDING_RE.match(name)
    if not match:
        return None
    raw = read_small(os.path.join(PENDING, name))
    if raw is None:
        return None
    try:
        req = json.loads(raw.decode("utf-8"))
    except Exception:
        return None
    if not isinstance(req, dict) or req.get("id") != match.group(1) or req.get("kind") not in ("approval", "notify"):
        return None
    ints = ("ts", "expires")
    if any(isinstance(req.get(k), bool) or not isinstance(req.get(k), int) for k in ints):
        return None
    if any(not isinstance(req.get(k), str) for k in ("agent", "event", "tool", "command", "cwd", "session", "nonce", "command_sha256")):
        return None
    if req["expires"] <= now:
        return None
    return req


def ctl(sub):
    now = int(time.time())
    if sub == "status":
        pending = 0
        if queue_trusted():
            try:
                pending = len([n for n in os.listdir(PENDING) if PENDING_RE.match(n)])
            except OSError:
                pass
        return reply({"ok": True, "ssh_keygen": bool(keygen_path()),
                      "allowed_signers": allowed_signers_trusted(), "pending": pending})
    if not queue_trusted():
        return reply({"ok": False, "error": "approval queue is missing or not private"})
    if sub == "sessions":
        return reply({"host_time": now, "items": list_sessions()})
    if sub == "list":
        items = []
        for name in os.listdir(PENDING):
            req = load_pending(name, now)
            if req:
                items.append(req)
        items.sort(key=lambda r: (r["ts"], r["id"]))
        return reply({"host_time": now, "items": items[:MAX_LIST]})
    body = read_stdin_json()
    if body is None:
        return reply({"ok": False, "error": "invalid request"})
    rid = body.get("id")
    if not isinstance(rid, str) or not ID_RE.match(rid):
        return reply({"ok": False, "error": "invalid id"})
    req = load_pending(rid + ".json", now)
    if sub == "ack":
        if req and req["kind"] == "notify":
            unlink(os.path.join(PENDING, rid + ".json"))
        return reply({"ok": True})
    if sub == "decide":
        approved, dts, sig = body.get("approved"), body.get("ts"), body.get("signature")
        if not req or req["kind"] != "approval":
            return reply({"ok": False, "error": "request is gone or expired"})
        if (not isinstance(approved, bool) or isinstance(dts, bool) or not isinstance(dts, int)
                or not isinstance(sig, str) or not sig or len(sig) > MAX_SIG):
            return reply({"ok": False, "error": "invalid decision"})
        try:
            if len([n for n in os.listdir(DECISIONS) if n.startswith(rid + ".")]) >= MAX_DECISIONS_PER_ID:
                return reply({"ok": False, "error": "too many decisions for this request"})
            data = json.dumps({"v": 1, "id": rid, "approved": approved, "ts": dts, "signature": sig})
            final = os.path.join(DECISIONS, "%s.%s.json" % (rid, os.urandom(8).hex()))
            tmp = os.path.join(DECISIONS, ".tmp-%s" % os.urandom(8).hex())
            write_new(tmp, data.encode("utf-8"))
            os.rename(tmp, final)
        except OSError:
            return reply({"ok": False, "error": "could not write the decision"})
        return reply({"ok": True})
    return reply({"ok": False, "error": "unknown subcommand"})


def safe_main():
    """Any unexpected error means "no opinion" (exit 0, no decision), never a crash/traceback."""
    try:
        return main()
    except Exception:
        if len(sys.argv) > 1 and sys.argv[1] == "ctl":
            return reply({"ok": False, "error": "internal error"})
        return 0


def _terminate(signum, frame):
    raise SystemExit(0)  # lets `finally` remove our pending file when the agent kills us


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, _terminate)
    signal.signal(signal.SIGHUP, _terminate)
    sys.exit(safe_main())
