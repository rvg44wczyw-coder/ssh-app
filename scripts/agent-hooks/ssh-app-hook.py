#!/usr/bin/env python3
"""ssh-app agent hook (Claude Code + Google Antigravity) and approval queue (Feature 13).

Usage (configured by the app, never by hand):
    ssh-app-hook <claude|codex|antigravity|grok> <EventName>     agent hook (reads the hook JSON on stdin)
    ssh-app-hook ctl <list|wait|decide|ack|status>          used by the app over SSH (data on stdin)

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
import shlex
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
try:
    WAIT_MAX = min(20.0, max(0.1, float(os.environ.get("SSH_APP_WAIT_MAX", "20"))))  # `ctl wait` holds the call at most this long
except ValueError:
    WAIT_MAX = 20.0
WAIT_STEP = 0.4
MAX_DECISIONS_PER_ID = 5
MAX_AGE = 120  # a decision may be at most this many seconds newer than its request
NOTIFY_TTL = 600
NAMESPACE = "ssh-app-approval"
PRINCIPAL = "ssh-app"
ID_RE = re.compile(r"^[0-9a-f]{32}$")
PENDING_RE = re.compile(r"^([0-9a-f]{32})\.json$")
# SHA-256 of the typed helper (`ssh-app-host`, feature 25) this script may hand over to. The app
# substitutes it at install time; empty (as in the repository) means "always run the Python below".
HELPER_SHA256 = ""
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
    if not ssh_verify(message.encode("utf-8"), sig):
        return None
    if approved and isinstance(dec.get("trust"), dict):
        try:
            store_trust(req, dec["trust"], dts)
        except Exception:
            pass
    return approved


def ssh_verify(message, sig):
    """True when `sig` is the paired phone key's SSHSIG over `message` (bytes)."""
    keygen = keygen_path()
    if not keygen or not allowed_signers_trusted():
        return False
    sigfile = os.path.join(RUN, ".sig-%s" % os.urandom(8).hex())
    try:
        write_new(sigfile, sig.encode("utf-8", "replace"))
        proc = subprocess.run(
            [keygen, "-Y", "verify", "-f", ALLOWED, "-I", PRINCIPAL, "-n", NAMESPACE, "-s", sigfile],
            input=message, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5)
        return proc.returncode == 0
    except Exception:
        return False
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
SHELL_TOOLS = ("Bash", "run_command", "bash", "shell", "run_shell_command")
EDIT_TOOLS = ("Edit", "Write", "MultiEdit", "NotebookEdit", "write_to_file", "replace_file_content", "edit_file",
              # Qwen Code, Crush and OpenCode (lower-case names)
              "write_file", "replace", "edit", "write", "multiedit", "patch", "apply_patch")
READ_TOOLS = ("Read", "view_file", "read_file",
              # Qwen Code, Crush and OpenCode
              "read_many_files", "view", "read")
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


WRAPPERS = ("sudo", "env", "command", "exec", "nohup", "time", "nice", "xargs", "builtin", "doas", "stdbuf", "timeout")
WRAPPER_VALUE_FLAGS = ("-u", "-g", "-n", "-s", "-C", "-p", "-h", "-U", "-t", "-k", "-i", "-o", "-e", "-a", "-I", "-L", "-P")
SHELLS = ("sh", "bash", "zsh", "dash", "ksh", "fish")


def shell_segments(command):
    return [seg.strip() for seg in re.split(r"\n|;|&&|\|\||\||&", command) if seg.strip()]


def peel(segment, depth=0):
    """Command lines a segment really runs: whitespace normalised, `sudo`/`env VAR=x`/`xargs`...
    peeled off, the program name stripped of its path, quotes and case, `sh -c "..."` and
    `$(...)` / backticks looked into. Best effort: it removes the cheap ways around a prefix rule,
    not every way (a rule is not a sandbox)."""
    out = []
    if depth > 4:
        return out
    inner = re.findall(r"\$\(([^()]*)\)", segment) + re.findall(r"`([^`]*)`", segment)
    for sub in inner:
        for seg in shell_segments(sub):
            out.extend(peel(seg, depth + 1))
    try:
        toks = shlex.split(segment, posix=True)
    except ValueError:
        toks = segment.split()
    while toks:
        head = toks[0]
        if "=" in head and re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", head):
            toks = toks[1:]
        elif os.path.basename(head).lower() in WRAPPERS:
            toks = toks[1:]
            while toks and toks[0].startswith("-"):
                flag = toks[0]
                toks = toks[1:]
                # Options that take a value (`sudo -u root`, `nice -n 5`, `timeout -s KILL 5`).
                if flag in WRAPPER_VALUE_FLAGS and toks:
                    toks = toks[1:]
            if os.path.basename(head).lower() == "timeout" and toks and re.match(r"^[0-9.]+[smhd]?$", toks[0]):
                toks = toks[1:]
        else:
            break
    if not toks:
        return out
    prog = os.path.basename(toks[0]).lower()
    if prog in SHELLS:
        for i, t in enumerate(toks[1:], 1):
            if t.startswith("-") and "c" in t.lstrip("-") and i + 1 < len(toks):
                for seg in shell_segments(toks[i + 1]):
                    out.extend(peel(seg, depth + 1))
                break
    out.append(" ".join([prog] + toks[1:]))
    return out


def command_candidates(command):
    cands = []
    for seg in [command] + shell_segments(command):
        cands.append(re.sub(r"\s+", " ", seg).strip())
        cands.extend(peel(seg))
    return cands


def bash_spec_matches(spec, command, raw_only=False):
    rx = None if spec.endswith(":*") else glob_regex(spec)
    cands = [re.sub(r"\s+", " ", command).strip()] if raw_only else command_candidates(command)
    for cand in cands:
        if rx is None:
            prefix = spec[:-2]
            low, pl = cand.lower(), prefix.lower()
            first = pl.split(" ", 1)[0]
            # Program names are compared without case (the default macOS volume ignores it),
            # the rest of the prefix as typed.
            if cand == prefix or cand.startswith(prefix + " "):
                return True
            if " " not in prefix and (low == pl or low.startswith(pl + " ")):
                return True
        elif rx.match(cand):
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


PATH_KEYS = ("file_path", "path", "AbsolutePath", "TargetFile", "notebook_path",
             "filePath", "absolute_path")  # the last two: OpenCode, Qwen Code
GUARD_MAX_INPUT = 32768  # longer text is never fed to shlex (its cost grows quadratically)


def input_too_big(tool_input):
    ti = tool_input if isinstance(tool_input, dict) else {}
    return any(isinstance(v, str) and len(v) > GUARD_MAX_INPUT for v in ti.values())


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
    for key in PATH_KEYS:
        if isinstance(ti.get(key), str) and ti[key]:
            path = ti[key]
            break
    # Tools that take several paths (Qwen's read_many_files): every one is checked.
    paths = [path] if path else []
    for key in ("paths", "file_paths"):
        if isinstance(ti.get(key), list):
            paths += [x for x in ti[key] if isinstance(x, str) and x][:50]
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
            if kind in ("Edit", "Read") and any(path_spec_matches(spec, p, cwd) for p in paths):
                return rule
        except re.error:
            continue
    return None


SAFE_REDIRECTS = re.compile(r"\d*>>?\s*/dev/null|\d*>&\d|&>\s*/dev/null")


def tool_command(ti):
    for key in ("command",) + ANTIGRAVITY_CMD_KEYS[:4]:
        if isinstance(ti.get(key), str) and ti[key]:
            return ti[key]
    return ""


def unsafe_for_allow(command):
    """Shell syntax that makes a command more than the words of its segments: output redirection
    into a file (an edit), process substitution. Such a command is never allowed without asking."""
    plain = SAFE_REDIRECTS.sub(" ", command)
    return ">" in plain or "<(" in plain


def segment_allowed(rules, tool, seg, depth=0):
    """Name of the allow rule that covers this one command line, else None. Allow looks at what
    was typed (no peeling of `sudo`/`env`/`sh -c`: those are exactly what must not borrow the
    permission of the command inside them), and every `$(...)` / backtick inside must be allowed too."""
    if depth > 4:
        return None
    hit = None
    for rule in rules:
        m = RULE_RE.match(rule.strip())
        if not m or not rule_tool_matches(m.group(1), tool):
            continue
        spec = m.group(2)
        if spec is None or spec == "":
            hit = rule
            break
        try:
            if bash_spec_matches(spec, seg, raw_only=True):
                hit = rule
                break
        except re.error:
            continue
    if hit is None:
        return None
    for sub in re.findall(r"\$\(([^()]*)\)", seg) + re.findall(r"`([^`]*)`", seg):
        for inner in shell_segments(sub):
            if segment_allowed(rules, tool, inner, depth + 1) is None:
                return None
    return hit


def guard_allows(rules, tool, tool_input, cwd):
    """The allow rule that covers this tool call, else None. Unlike deny/ask (any matching part is
    enough), allow needs EVERY part of a shell command covered: `cargo test; rm -rf ~` must not ride
    on the `cargo test` rule."""
    if not rules:
        return None
    if tool_kind(tool) != "Bash":
        return guard_match(rules, tool, tool_input, cwd)
    ti = tool_input if isinstance(tool_input, dict) else {}
    command = tool_command(ti)
    # `2>&1` and `>/dev/null` are not edits and must not split the line at their `&`.
    segs = shell_segments(SAFE_REDIRECTS.sub(" ", command))
    if not segs:
        return None
    # A bare `Bash` rule is the user's own "everything" and keeps meaning that.
    for rule in rules:
        m = RULE_RE.match(rule.strip())
        if m and rule_tool_matches(m.group(1), tool) and not m.group(2):
            return rule
    if unsafe_for_allow(command):
        return None
    first = None
    for seg in segs:
        hit = segment_allowed(rules, tool, seg)
        if hit is None:
            return None
        first = first or hit
    return first


# ---------------------------------------------------------------------------------------------
# Auto-approval by category (feature 50). The app compiles its policy into `auto_rules`
# (guard.json): [{"agents": [..]?, "root": "/p"?, "allow": [..], "deny": [..], "edits": bool}].
# Entries with a root apply only to a session whose cwd is inside that folder, and only to a call
# that stays inside it (section 4.5 of the spec). Nothing here reads the repository's own files.
# ---------------------------------------------------------------------------------------------
AUTO_MAX_ENTRIES = 100
AUTO_MAX_WORDS = 200
AUTO_PROTECTED = (".git", ".claude", ".gemini", ".agents", ".ssh-app", ".mcp.json")
AUTO_PATH_INPUT_KEYS = PATH_KEYS + ("pattern", "glob")
AUTOLOG = os.path.join(RUN, "auto.log")
AUTOLOG_MAX = 200000


SENSITIVE = (".ssh", ".aws", ".gnupg", ".env", ".pem", ".key", ".kube", ".netrc", ".npmrc", ".pypirc",
             ".docker", ".git-credentials", "id_rsa", "id_ed25519", "id_ecdsa", "keychain",
             ".claude", ".gemini", ".codex", ".grok", ".mcp.json")
GIT_DIR_RE = re.compile(r"(?:^|[/\s'\"=])\.git(?:$|[/\s'\"])")


def touches_sensitive(text):
    """Credentials, agent settings and the .git folder are never reached by an automatic allow,
    wherever they are named (a word may hide them inside a longer path)."""
    low = text.lower()
    return any(t in low for t in SENSITIVE) or GIT_DIR_RE.search(low) is not None


def auto_entries(guard, agent):
    raw = guard.get("auto_rules")
    out = []
    if not isinstance(raw, list):
        return out
    for e in raw[:AUTO_MAX_ENTRIES]:
        if not isinstance(e, dict):
            continue
        agents = e.get("agents")
        if agents is not None and not (isinstance(agents, list) and agent in agents):
            continue
        root = e.get("root")
        if root is not None and not (isinstance(root, str) and root.startswith("/") and len(root) <= 400):
            continue
        out.append({"root": root, "allow": guard_rules(e, "allow"), "deny": guard_rules(e, "deny"),
                    "edits": e.get("edits") is True})
    return out


def real(path):
    try:
        return os.path.realpath(path)
    except (OSError, ValueError):
        return None


def inside(root, path):
    """True when the real path `path` is root itself or below it (both already resolved)."""
    return path == root or path.startswith(root.rstrip("/") + "/")


def project_root(entries, cwd):
    """The most specific root (resolved) that contains cwd, else None."""
    here = real(cwd) if cwd else None
    if not here:
        return None
    best = None
    for e in entries:
        if e["root"] is None:
            continue
        r = real(e["root"])
        if r and inside(r, here) and (best is None or len(r) > len(best)):
            best = r
    return best


def word_stays_inside(root, cwd, word):
    """False for a word that names a place outside the project (or cannot be resolved)."""
    if "$" in word or "`" in word:
        return False
    for part in ([word] + [word.split("=", 1)[1]] if "=" in word else [word]):
        if not part or part.startswith("-") and "=" not in word:
            continue
        looks_like_path = part.startswith(("/", "~", ".")) or "/" in part or ".." in part
        target = os.path.expanduser(part) if part.startswith("~") else part
        if not looks_like_path and not os.path.lexists(os.path.join(cwd, target)):
            continue
        r = real(target if target.startswith("/") else os.path.join(cwd, target))
        if r is None or not inside(root, r):
            return False
    return True


def bash_stays_inside(root, cwd, command):
    words = []
    for seg in shell_segments(SAFE_REDIRECTS.sub(" ", command)):
        try:
            words += shlex.split(seg, posix=True)
        except ValueError:
            return False
        if len(words) > AUTO_MAX_WORDS:
            return False
    return all(word_stays_inside(root, cwd, w) for w in words)


def tool_paths(ti):
    paths = []
    for key in AUTO_PATH_INPUT_KEYS:
        if isinstance(ti.get(key), str) and ti[key]:
            paths.append(ti[key])
    for key in ("paths", "file_paths"):
        if isinstance(ti.get(key), list):
            paths += [x for x in ti[key] if isinstance(x, str) and x][:50]
    return paths


def paths_stay_inside(root, cwd, paths):
    return all(word_stays_inside(root, cwd, p) for p in paths)


def edit_target_ok(root, cwd, paths):
    """Edit tools: every file inside the project and not in a protected place."""
    if not paths:
        return False
    for p in paths:
        if "$" in p:
            return False
        full = os.path.expanduser(p) if p.startswith("~") else p
        r = real(full if full.startswith("/") else os.path.join(cwd, full))
        if r is None or not inside(root, r):
            return False
        rel = os.path.relpath(r, root).split(os.sep)
        if any(c in AUTO_PROTECTED for c in rel):
            return False
    return True


HEREDOC_OPEN = re.compile(r'"\$\(cat <<([\'"])([A-Za-z_][A-Za-z0-9_]*)\1\n')
COMMIT_MESSAGE_HEAD = re.compile(r"^\s*git\s(?:.*\s)?(?:commit|tag)\s(?:.*\s)?(?:-[A-Za-z]*m\s|--message[=\s])$")


def inert_commit_messages(command):
    """`git commit -m "$(cat <<'EOF' ... EOF\n)"` is how agents write a long message. With a quoted
    delimiter the body is never expanded or run, and it is only the value of -m / --message of a
    git commit or tag: it is replaced by a placeholder, so the lines of the message are not taken
    for commands. Anything else (another command that would run the text, a different shape) stays
    as it is and is judged as typed."""
    out, i = [], 0
    while True:
        m = HEREDOC_OPEN.search(command, i)
        if not m:
            out.append(command[i:])
            return "".join(out)
        head = "".join(out) + command[i:m.start()]
        last = re.split(r"\n|;|&&|\|\||\||&", head)[-1]
        end = "\n" + m.group(2) + "\n"
        k = command.find(end, m.end() - 1)
        close = k + len(end) if k >= 0 else -1
        if COMMIT_MESSAGE_HEAD.match(last) and close >= 0 and command.startswith(')"', close):
            out.append(command[i:m.start()] + '"MSG"')
            i = close + 2
        else:
            out.append(command[i:m.end()])
            i = m.end()


def with_inert_messages(tool, tool_input):
    """The tool input with the commit messages of its command made inert (a copy)."""
    if tool_kind(tool) != "Bash" or not isinstance(tool_input, dict):
        return tool_input
    ti = dict(tool_input)
    for key in ("command",) + ANTIGRAVITY_CMD_KEYS[:4]:
        if isinstance(ti.get(key), str):
            ti[key] = inert_commit_messages(ti[key])
    return ti


def auto_decision(guard, agent, session, tool, tool_input, cwd):
    """("deny", rule) | ("allow", rule, root) | None for a tool call, from the compiled auto rules."""
    tool_input = with_inert_messages(tool, tool_input)
    entries = auto_entries(guard, agent)
    if not entries:
        return None
    for e in entries:
        hit = guard_match(e["deny"], tool, tool_input, cwd)
        if hit is not None:
            return ("deny", hit)
    ti = tool_input if isinstance(tool_input, dict) else {}
    kind = tool_kind(tool)
    root = project_root(entries, cwd)
    project_rules, free_rules, edits = [], [], False
    for e in entries:
        if e["root"] is None:
            free_rules += e["allow"]
        elif root is not None and real(e["root"]) == root:
            project_rules += e["allow"]
            edits = edits or e["edits"]
    if kind == "Edit":
        if edits and root and edit_target_ok(root, cwd, tool_paths(ti)):
            return ("allow", "project file edits", root)
        return trust_decision(guard, agent, session, tool, tool_input, cwd)
    if touches_sensitive(tool_command(ti) if kind == "Bash" else " ".join(tool_paths(ti))):
        return None
    if root is not None and project_rules:
        hit = guard_allows(project_rules + free_rules, tool, tool_input, cwd)
        stays = (bash_stays_inside(root, cwd, tool_command(ti)) if kind == "Bash"
                 else paths_stay_inside(root, cwd, tool_paths(ti)))
        if hit is not None and stays:
            return ("allow", hit, root)
    # Read-only inspection anywhere: the floor (deny) has already run, no project boundary applies.
    hit = guard_allows(free_rules, tool, tool_input, cwd)
    if hit is not None:
        return ("allow", hit, root)
    return trust_decision(guard, agent, session, tool, tool_input, cwd)


TRUST = os.path.join(RUN, "trust")
TRUST_MAX = 50
TRUST_HARD_CAP = 86400


def trust_cap(guard):
    n = guard.get("trust_max_secs")
    return n if isinstance(n, int) and not isinstance(n, bool) and 60 <= n <= TRUST_HARD_CAP else TRUST_HARD_CAP


def trust_message(t):
    """SSH_APP_TRUST_V1 plus the fields, each as `<utf8 length>:<bytes>\n`, so no split between
    fields can collide with another and no approval can be mistaken for a trust."""
    fields = [t["id"], t["nonce"], str(t["ts"]), str(t["expires"]), t["agent"], t["session"], t["root"],
              "true" if t["edits"] else "false", hashlib.sha256("\n".join(t["rules"]).encode("utf-8")).hexdigest()]
    out = b"SSH_APP_TRUST_V1:"
    for f in fields:
        raw = f.encode("utf-8")
        out += b"%d:" % len(raw) + raw + b"\n"
    return out


def trust_rules_ok(rules):
    if not isinstance(rules, list) or not 0 < len(rules) <= 60:
        return False
    for r in rules:
        if not isinstance(r, str) or not 0 < len(r) <= 400:
            return False
        m = RULE_RE.match(r.strip())
        if not m or (tool_kind(m.group(1)) == "Bash" and not m.group(2)):
            return False
    return True


def read_trust_blob(path):
    raw = read_small(path, 20000)
    if raw is None:
        return None
    try:
        t = json.loads(raw.decode("utf-8"))
    except Exception:
        return None
    if not isinstance(t, dict):
        return None
    need = {"id": str, "nonce": str, "agent": str, "session": str, "root": str, "signature": str}
    for k, typ in need.items():
        if not isinstance(t.get(k), typ):
            return None
    for k in ("ts", "expires"):
        if not isinstance(t.get(k), int) or isinstance(t.get(k), bool):
            return None
    if not isinstance(t.get("edits"), bool) or not trust_rules_ok(t.get("rules")):
        return None
    if not ID_RE.match(t["id"]) or not t["root"].startswith("/") or not t["session"]:
        return None
    return t


def trust_valid(t, guard):
    """Signature, expiry and cap; the host clock decides."""
    now = int(time.time())
    if not (t["ts"] < t["expires"] and now < t["expires"] and t["expires"] - t["ts"] <= trust_cap(guard)):
        return False
    return len(t["signature"]) <= MAX_SIG and ssh_verify(trust_message(t), t["signature"])


def store_trust(req, t, dts):
    guard = load_guard()
    blob = {"v": 1, "id": req["id"], "nonce": req["nonce"], "ts": dts, "expires": t.get("expires"),
            "agent": req["agent"], "session": req["session"], "root": t.get("root"),
            "edits": t.get("edits"), "rules": t.get("rules"), "signature": t.get("signature")}
    if t.get("agent") != req["agent"] or t.get("session") != req["session"] or not req["session"]:
        return
    path_ok = os.path.join(TRUST, ".tmp-" + os.urandom(8).hex())
    try:
        os.makedirs(TRUST, mode=0o700, exist_ok=True)
        if not private_dir(TRUST):
            return
        data = json.dumps(blob).encode("utf-8")
        write_new(path_ok, data)
        parsed = read_trust_blob(path_ok)
        if parsed is None or not trust_valid(parsed, guard):
            return
        sweep_trusts(guard)
        if len([n for n in os.listdir(TRUST) if n.endswith(".json")]) >= TRUST_MAX:
            return
        os.rename(path_ok, os.path.join(TRUST, req["id"] + ".json"))
    except OSError:
        pass
    finally:
        unlink(path_ok)


def sweep_trusts(guard):
    """Deletes expired and unreadable trust files; returns the valid ones (id -> blob)."""
    out = {}
    try:
        names = sorted(os.listdir(TRUST))[:TRUST_MAX * 2]
    except OSError:
        return out
    now = int(time.time())
    for name in names:
        path = os.path.join(TRUST, name)
        if not name.endswith(".json") or name.startswith("."):
            continue
        t = read_trust_blob(path)
        if t is None or t["expires"] <= now or name != t["id"] + ".json":
            unlink(path)
            continue
        out[t["id"]] = t
    return out


def drop_trusts(agent, session):
    """A trust ends with the agent session it was given for."""
    for t in sweep_trusts({}).values():
        if t["agent"] == agent and t["session"] == session:
            unlink(os.path.join(TRUST, t["id"] + ".json"))


def project_listed(guard, root):
    """The trust's root must still be a project of the policy."""
    auto = guard.get("auto")
    projects = auto.get("projects") if isinstance(auto, dict) else None
    if not isinstance(projects, list):
        return False
    want = real(root)
    return any(isinstance(p, dict) and isinstance(p.get("root"), str) and real(p["root"]) == want
               for p in projects[:60])


def trust_decision(guard, agent, session, tool, tool_input, cwd):
    """("allow", reason, root, id) when a signed, unexpired trust of this agent session covers the call."""
    if not session or not cwd or not os.path.isdir(TRUST) or not private_dir(TRUST):
        return None
    ti = tool_input if isinstance(tool_input, dict) else {}
    here = real(cwd)
    kind = tool_kind(tool)
    for t in sweep_trusts(guard).values():
        if t["agent"] != agent or t["session"] != session:
            continue
        root = real(t["root"])
        if not here or not root or not inside(root, here) or not project_listed(guard, t["root"]):
            continue
        if kind == "Edit":
            ok = t["edits"] and edit_target_ok(root, cwd, tool_paths(ti))
            hit = "project file edits"
        elif touches_sensitive(tool_command(ti) if kind == "Bash" else " ".join(tool_paths(ti))):
            continue
        else:
            hit = guard_allows(t["rules"], tool, tool_input, cwd)
            ok = hit is not None and (bash_stays_inside(root, cwd, tool_command(ti)) if kind == "Bash"
                                      else paths_stay_inside(root, cwd, tool_paths(ti)))
        if ok and trust_valid(t, guard):
            return ("allow", "trusted %s: %s" % (t["id"][:8], hit), root, t["id"])
    return None


def auto_log(agent, session, tool, summary, decision, rule, root):
    """One line per automatic decision, newest last, trimmed by size. Best effort."""
    try:
        if not private_dir(HOME_DIR):
            return
        os.makedirs(RUN, mode=0o700, exist_ok=True)
        line = json.dumps({"ts": int(time.time()), "agent": clean(agent, 40), "session": clean(session, 80),
                           "tool": clean(tool, 60), "summary": clean(summary, 200), "decision": decision,
                           "rule": clean(rule, 120), "root": clean(root or "", 300)}) + "\n"
        try:
            if os.path.getsize(AUTOLOG) > AUTOLOG_MAX:
                with open(AUTOLOG, "rb") as f:
                    keep = f.read()[-AUTOLOG_MAX // 2:]
                keep = keep[keep.find(b"\n") + 1:]
                fd = os.open(AUTOLOG, os.O_WRONLY | os.O_TRUNC | os.O_NOFOLLOW, 0o600)
                os.write(fd, keep)
                os.close(fd)
        except OSError:
            pass
        fd = os.open(AUTOLOG, os.O_WRONLY | os.O_APPEND | os.O_CREAT | getattr(os, "O_NOFOLLOW", 0), 0o600)
        try:
            os.write(fd, line.encode("utf-8"))
        finally:
            os.close(fd)
    except OSError:
        pass


# Agents that speak Claude's hook protocol (payload fields and reply shapes). Qwen Code documents
# the same `permissionDecision` / `PermissionRequest` replies.
CLAUDE_STYLE = ("claude", "codex", "qwen")


def allow_output(agent, event, reason):
    """The agent-specific "allow without asking" answer, or None where the agent cannot do that."""
    if agent in CLAUDE_STYLE:
        if event == "PermissionRequest":
            return {"hookSpecificOutput": {"hookEventName": "PermissionRequest",
                                           "decision": {"behavior": "allow"}}}
        return {"hookSpecificOutput": {"hookEventName": event, "permissionDecision": "allow",
                                       "permissionDecisionReason": reason}}
    if agent in ("antigravity", "opencode"):
        return {"decision": "allow", "reason": reason}
    if agent == "crush":
        return {"version": 1, "decision": "allow", "reason": reason}
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
    if agent == "crush":
        return {"version": 1, "decision": "deny", "reason": reason}
    if agent in CLAUDE_STYLE:
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


def zellij_session_name():
    """The Zellij session this process runs in (Zellij sets ZELLIJ and ZELLIJ_SESSION_NAME in every
    pane), or "" when it is not in one or the name has characters Zellij does not allow."""
    name = os.environ.get("ZELLIJ_SESSION_NAME", "")
    if "ZELLIJ" in os.environ and 0 < len(name) <= 64 and name[0] != "-" and all(c.isascii() and (c.isalnum() or c in "_-") for c in name):
        return name
    return ""


def tmux_session_name():
    """The multiplexer session the agent runs in: tmux's, else Zellij's, else "". The registry keeps
    it in one field either way; tmux wins when both are set."""
    if not os.environ.get("TMUX"):
        return zellij_session_name()
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


LIMITS = os.path.join(RUN, "limits")
STATUS_MAX_INPUT = 262144


def num(value):
    return value if isinstance(value, (int, float)) and not isinstance(value, bool) else None


def limit_window(raw):
    raw = as_dict(raw)
    pct = num(raw.get("used_percentage"))
    if pct is None:
        return None
    out = {"used_percentage": max(0, min(100, pct))}
    reset = num(raw.get("resets_at"))
    if reset is not None:
        out["resets_at"] = int(reset)
    return out


def statusline(prev_hex):
    """Claude Code `statusLine` wrapper: records `rate_limits`, then prints the user's own status
    line (when one was wrapped) or a short limits line. Never fails the status line."""
    raw = sys.stdin.buffer.read(STATUS_MAX_INPUT + 1)
    try:
        data = as_dict(json.loads(raw.decode("utf-8"))) if len(raw) <= STATUS_MAX_INPUT else {}
    except Exception:
        data = {}
    limits = as_dict(data.get("rate_limits"))
    windows = {k: limit_window(limits.get(k)) for k in ("five_hour", "seven_day")}
    windows = {k: v for k, v in windows.items() if v}
    cfg = os.environ.get("CLAUDE_CONFIG_DIR") or os.path.expanduser("~/.claude")
    cfg = os.path.realpath(os.path.expanduser(cfg))
    if windows and all(private_dir(d) for d in (HOME_DIR, RUN)):
        try:
            os.makedirs(LIMITS, mode=0o700, exist_ok=True)
            if private_dir(LIMITS):
                name = hashlib.sha256(cfg.encode("utf-8", "replace")).hexdigest()[:16]
                tmp = os.path.join(LIMITS, "." + name + ".tmp")
                unlink(tmp)
                write_new(tmp, json.dumps({"dir": cfg, "ts": int(time.time()), **windows}).encode())
                os.replace(tmp, os.path.join(LIMITS, name + ".json"))
        except OSError:
            pass
    prev = None
    if prev_hex:
        try:
            prev = bytes.fromhex(prev_hex).decode("utf-8")
        except ValueError:
            prev = None
    if prev:
        try:
            out = subprocess.run(prev, shell=True, input=raw, capture_output=True, timeout=5).stdout
            sys.stdout.buffer.write(out)
        except Exception:
            pass
        return 0
    parts = []
    for key, label in (("five_hour", "5h"), ("seven_day", "7d")):
        if key in windows:
            parts.append("%s %d%%" % (label, round(windows[key]["used_percentage"])))
    model = as_dict(data.get("model")).get("display_name")
    if isinstance(model, str) and model:
        parts.insert(0, clean(model, 40))
    sys.stdout.write(" · ".join(parts))
    return 0


def main():
    global TIMEOUT
    agent = sys.argv[1] if len(sys.argv) > 1 else ""
    event = sys.argv[2] if len(sys.argv) > 2 else ""
    if agent == "ctl":
        return ctl(event)
    if agent == "claude" and event == "statusline":
        return statusline(sys.argv[3] if len(sys.argv) > 3 else "")
    garbled = False
    try:
        loaded = json.load(sys.stdin)
        garbled = not isinstance(loaded, dict)
        data = as_dict(loaded)
    except Exception:
        data, garbled = {}, True

    if agent in CLAUDE_STYLE:  # Codex and Qwen Code use Claude's payload and PermissionRequest reply
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
    elif agent == "crush":
        # {"event","session_id","cwd","tool_name","tool_input"} on stdin (Crush hooks docs).
        tool = str(data.get("tool_name", ""))
        tool_input = data.get("tool_input", {})
        summary = summarize(tool_input)
        cwd = str(data.get("cwd", ""))
        session = str(data.get("session_id", ""))
    elif agent == "opencode":
        # Sent by the generated plugin: {"tool","args","cwd","session"}.
        tool = str(data.get("tool", ""))
        tool_input = data.get("args", {})
        summary = summarize(tool_input)
        cwd = str(data.get("cwd", ""))
        session = str(data.get("session", ""))
    elif agent == "grok":
        tool = str(data.get("toolName", ""))
        tool_input = data.get("toolInput", {})
        summary = summarize(tool_input)
        cwd = str(data.get("cwd", ""))
        session = str(data.get("sessionId", ""))
    else:
        return 0

    base = {"agent": agent, "event": event, "tool": tool, "command": summary, "cwd": cwd, "session": session}

    # Antigravity has no SessionStart hook: whatever event comes first notes the tmux session, so
    # the phone knows which tab a request or a notice is about.
    if session and event not in ("SessionStart", "SessionEnd") and not os.path.exists(session_file(agent, session)):
        register_session(agent, "", session, cwd)

    if event in ("SessionStart", "SessionEnd"):
        register_session(agent, event, session, cwd)
        if event == "SessionEnd" and session:
            drop_trusts(agent, session)
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
        if garbled:
            # Not JSON (or not an object): the phone would be asked about a blank request that
            # stands for the real tool call. Nothing legitimate sends this: fail closed.
            sys.stdout.write(json.dumps(deny_output(agent, event, "Blocked: the hook payload could not be read")))
            return 0
        guard = load_guard()
        wait = guard.get("approval_wait_secs")
        if ("SSH_APP_HOOK_TIMEOUT" not in os.environ and isinstance(wait, int)
                and not isinstance(wait, bool) and 5 <= wait <= 120):
            TIMEOUT = float(wait)
        if input_too_big(tool_input) and (any(guard_rules(guard, k) for k in ("deny", "ask", "allow")) or guard.get("auto_rules")):
            # Rules cannot be checked reliably (and cheaply) on this much text: fail closed.
            sys.stdout.write(json.dumps(deny_output(
                agent, event, "Blocked: the tool input is longer than %d characters, too long to check against the host guard rules" % GUARD_MAX_INPUT)))
            return 0
        hit = guard_match(guard_rules(guard, "deny"), tool, tool_input, cwd)
        if hit is not None:
            reason = "Blocked by a host guard rule: " + clean(hit, 120)
            sys.stdout.write(json.dumps(deny_output(agent, event, reason)))
            return 0
        auto = auto_decision(guard, agent, session, tool, tool_input, cwd)
        if auto is not None and auto[0] == "deny":
            auto_log(agent, session, tool, summary, "deny", auto[1], None)
            sys.stdout.write(json.dumps(deny_output(agent, event, "Blocked by an auto-approval category: " + clean(auto[1], 120))))
            return 0
        hit = guard_match(guard_rules(guard, "ask"), tool, tool_input, cwd)
        if hit is not None:
            out = ask_output(agent, event, "A host guard rule asks you first: " + clean(hit, 120))
            if out is not None:
                sys.stdout.write(json.dumps(out))
                return 0
        else:
            hit = guard_allows(guard_rules(guard, "allow"), tool, tool_input, cwd)
            if hit is not None:
                out = allow_output(agent, event, "Allowed by a host guard rule: " + clean(hit, 120))
                if out is not None:
                    sys.stdout.write(json.dumps(out))
                    return 0
            elif auto is not None:
                out = allow_output(agent, event, "Allowed automatically: " + clean(auto[1], 120))
                if out is not None:
                    auto_log(agent, session, tool, summary, "trusted" if len(auto) > 3 else "allow", auto[1], auto[2])
                    sys.stdout.write(json.dumps(out))
                    return 0
        # Antigravity's own allow list (permissions.allow in its settings): a plain command on it runs
        # without asking in the terminal, so the phone is not asked either.
        if (agent == "antigravity" and tool == "run_command"
                and guard_match(guard_rules(guard, "ask"), tool, tool_input, cwd) is None
                and agy_allows(summary)):
            return 0
        approved = submit("approval", base, wait=True)
        if agent == "grok":
            # Grok can only block: deny is explicit, everything else (including a phone approval)
            # lets the tool continue into Grok's own flow; failures fail open on its side anyway.
            if approved is False:
                sys.stdout.write(json.dumps({"decision": "deny", "reason": "Denied from ssh-app on your phone"}))
            return 0
        if agent == "crush":
            # Exit 0 with a decision; no answer lets Crush run its own permission flow.
            if approved is not None:
                sys.stdout.write(json.dumps(allow_output(agent, event, "Approved from ssh-app") if approved
                                            else deny_output(agent, event, "Denied from ssh-app on your phone")))
            return 0
        if agent == "opencode":
            # The plugin throws on deny; anything else continues into OpenCode's own flow.
            # OpenCode allows most tools by default, so no answer must not mean "go ahead".
            if approved is True:
                out = {"decision": "allow", "reason": "Approved from ssh-app"}
            elif approved is False:
                out = {"decision": "deny", "reason": "Denied from ssh-app on your phone"}
            else:
                out = {"decision": "deny", "reason": "No answer from your phone in time (or the ssh-app approval queue is not set up); blocked"}
            sys.stdout.write(json.dumps(out))
            return 0
        if agent in CLAUDE_STYLE:
            if approved is None:
                return 0  # no opinion -> the agent shows its normal permission prompt
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

    # Notification / Stop and other passive events: fire-and-forget. An Antigravity stop whose
    # transcript does not end with the model's own answer is a failure, not "finished".
    if agent == "antigravity" and event == "Stop" and not agy_ended_with_answer(session):
        submit("notify", dict(base, event="StopFailed"), wait=False)
        sys.stdout.write(json.dumps({"decision": "stop"}))
        return 0
    submit("notify", base, wait=False)
    if agent == "antigravity" and event == "Stop":
        sys.stdout.write(json.dumps({"decision": "stop"}))
    return 0


def reply(obj):
    sys.stdout.write(json.dumps(obj))
    return 0


# ---------------------------------------------------------------------------------------------
# Antigravity: its own allow list and the end of a conversation (ports of ssh-app-host's hook).
# ---------------------------------------------------------------------------------------------
def agy_allow_entries():
    path = os.path.join(os.path.expanduser("~"), ".gemini", "antigravity-cli", "settings.json")
    raw = read_small(path, 1 << 20)
    try:
        allow = json.loads(raw.decode("utf-8"))["permissions"]["allow"] if raw else []
    except Exception:
        return []
    out = []
    for e in allow if isinstance(allow, list) else []:
        if isinstance(e, str) and e.startswith("command(") and e.endswith(")"):
            out.append(e[len("command("):-1])
    return out


def agy_allows(command):
    """A plain command (no chaining, pipes, substitution, redirection, nothing cut off) that equals an
    allow entry or continues it after a space; an entry ending in `*` is a prefix."""
    c = command.strip()
    if not c or c.endswith("\u2026") or any(ch in c for ch in ";&|`$<>\n\r(){}"):
        return False
    for e in agy_allow_entries():
        e = e.strip()
        if not e:
            continue
        if e.endswith("*"):
            prefix = e[:-1].strip()
            if prefix and c.startswith(e[:-1].rstrip()):
                return True
        elif c == e or c.startswith(e + " "):
            return True
    return False


def agy_ended_with_answer(session):
    """Whether the conversation's transcript ends with the model's reply. Unreadable: assume yes."""
    if not session or not re.match(r"^[A-Za-z0-9_-]+$", session):
        return True
    path = os.path.join(os.path.expanduser("~"), ".gemini", "antigravity-cli", "brain", session,
                        ".system_generated", "logs", "transcript.jsonl")
    try:
        with open(path, "rb") as f:
            f.seek(0, 2)
            size = f.tell()
            window = 256 * 1024
            f.seek(max(0, size - window))
            text = f.read(window).decode("utf-8", "replace")
    except OSError:
        return True
    last = None
    for line in reversed(text.splitlines()):
        t = line.strip()
        if t.startswith("{") and t.endswith("}"):
            last = t
            break
    if last is None:
        return True
    try:
        v = json.loads(last)
    except Exception:
        return True
    content = v.get("content")
    return (v.get("type") == "PLANNER_RESPONSE" and v.get("status") == "DONE"
            and isinstance(content, str) and content.strip() != "")


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


def pending_items(now):
    items = []
    for name in os.listdir(PENDING):
        req = load_pending(name, now)
        if req:
            items.append(req)
    items.sort(key=lambda r: (r["ts"], r["id"]))
    return items[:MAX_LIST]


def trust_items():
    guard = load_guard()
    items = []
    for t in sweep_trusts(guard).values():
        if trust_valid(t, guard):
            items.append({"id": t["id"], "agent": clean(t["agent"], 40), "session": clean(t["session"], 80),
                          "root": clean(t["root"], 300), "expires": t["expires"], "edits": t["edits"],
                          "rules": [clean(r, 120) for r in t["rules"][:12]]})
    items.sort(key=lambda i: i["expires"])
    return items[:TRUST_MAX]


def auto_log_items():
    raw = read_small(AUTOLOG, AUTOLOG_MAX * 2)
    items = []
    for line in (raw or b"").decode("utf-8", "replace").splitlines()[-200:]:
        try:
            v = json.loads(line)
        except ValueError:
            continue
        if isinstance(v, dict):
            items.append(v)
    return items[-50:]


def ctl(sub):
    now = int(time.time())
    if sub == "status":
        pending = 0
        if queue_trusted():
            try:
                pending = len([n for n in os.listdir(PENDING) if PENDING_RE.match(n)])
            except OSError:
                pass
        return reply({"ok": True, "impl": "python", "ssh_keygen": bool(keygen_path()),
                      "allowed_signers": allowed_signers_trusted(), "pending": pending})
    if not queue_trusted():
        return reply({"ok": False, "error": "approval queue is missing or not private"})
    if sub == "sessions":
        return reply({"host_time": now, "items": list_sessions()})
    if sub == "auto-log":
        return reply({"host_time": now, "items": auto_log_items()})
    if sub == "trusts":
        return reply({"host_time": now, "items": trust_items()})
    if sub == "revoke-all":
        for tid in list(sweep_trusts({})):
            unlink(os.path.join(TRUST, tid + ".json"))
        return reply({"ok": True})
    if sub == "list":
        return reply({"host_time": now, "items": pending_items(now)})
    if sub == "wait":
        # Long poll: answers as soon as the set of pending ids differs from the ids the app
        # already shows (argv[3], sorted and comma-separated), or after WAIT_MAX seconds.
        known = sys.argv[3] if len(sys.argv) > 3 else ""
        deadline = time.time() + WAIT_MAX
        while True:
            now = int(time.time())
            items = pending_items(now)
            if ",".join(sorted(r["id"] for r in items)) != known or time.time() >= deadline:
                return reply({"host_time": now, "items": items})
            time.sleep(WAIT_STEP)
    body = read_stdin_json()
    if body is None:
        return reply({"ok": False, "error": "invalid request"})
    rid = body.get("id")
    if not isinstance(rid, str) or not ID_RE.match(rid):
        return reply({"ok": False, "error": "invalid id"})
    if sub == "revoke":
        unlink(os.path.join(TRUST, rid + ".json"))
        return reply({"ok": True})
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
            decision = {"v": 1, "id": rid, "approved": approved, "ts": dts, "signature": sig}
            trust = body.get("trust")
            if approved and isinstance(trust, dict) and len(json.dumps(trust)) <= 12000:
                decision["trust"] = trust  # verified (and stored) by the waiting hook, not here
            data = json.dumps(decision)
            final = os.path.join(DECISIONS, "%s.%s.json" % (rid, os.urandom(8).hex()))
            tmp = os.path.join(DECISIONS, ".tmp-%s" % os.urandom(8).hex())
            write_new(tmp, data.encode("utf-8"))
            os.rename(tmp, final)
        except OSError:
            return reply({"ok": False, "error": "could not write the decision"})
        return reply({"ok": True})
    return reply({"ok": False, "error": "unknown subcommand"})


def exec_helper(argv):
    """Runs the typed helper in place of this script when it sits next to us and is exactly the
    pinned build (regular file of ours, not a link, not group/other writable, matching digest).
    Returns (to carry on in Python) when anything differs; never raises."""
    if not HELPER_SHA256 or not argv:
        return
    if argv[0] == "ctl":
        helper_args = ["ctl"] + argv[1:]
    elif argv[0] == "claude" and len(argv) > 1 and argv[1] == "statusline":
        helper_args = ["statusline"] + argv[2:]
    else:
        helper_args = ["hook"] + argv
    try:
        path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "ssh-app-host")
        fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK | getattr(os, "O_NOFOLLOW", 0))
        try:
            st = os.fstat(fd)
            if (not stat.S_ISREG(st.st_mode) or st.st_uid != os.geteuid() or st.st_mode & 0o022
                    or not st.st_mode & 0o100 or st.st_size > 64 * 1024 * 1024):
                return
            h = hashlib.sha256()
            while True:
                chunk = os.read(fd, 1 << 20)
                if not chunk:
                    break
                h.update(chunk)
        finally:
            os.close(fd)
        if h.hexdigest() != HELPER_SHA256:
            return
        os.execv(path, [path] + helper_args)
    except Exception:
        return


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
    exec_helper(sys.argv[1:])
    signal.signal(signal.SIGTERM, _terminate)
    signal.signal(signal.SIGHUP, _terminate)
    sys.exit(safe_main())
