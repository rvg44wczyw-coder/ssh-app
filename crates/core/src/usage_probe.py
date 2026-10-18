import json, os, re, subprocess, sys, time, glob, urllib.request
from datetime import datetime

NOW = time.time()
BUDGET_END = NOW + 15
dirs = [os.path.expanduser(d) for d in sys.argv[1:]] or [os.path.expanduser("~/.claude")]
os.environ["PATH"] = "/opt/homebrew/bin:/usr/local/bin:" + os.environ.get("PATH", "")


def run(cmd):
    try:
        return subprocess.run(cmd, capture_output=True, text=True, timeout=8).stdout
    except Exception:
        return ""


# ---- processes and tmux panes -------------------------------------------------------------
procs = {}
children = {}
for line in run(["ps", "-axo", "pid=,ppid=,args="]).splitlines():
    parts = line.strip().split(None, 2)
    if len(parts) < 3 or not parts[0].isdigit() or not parts[1].isdigit():
        continue
    pid, ppid, args = int(parts[0]), int(parts[1]), parts[2]
    procs[pid] = args
    children.setdefault(ppid, []).append(pid)

AGENTS = [
    ("claude", re.compile(r"(^|/)claude(\s|$)")),
    ("codex", re.compile(r"(^|/)codex(\s|$)")),
    ("agy", re.compile(r"(^|/)agy(\s|$)")),
    ("grok", re.compile(r"(^|/)grok(\s|$)")),
]
OLLAMA = re.compile(r"(^|/)ollama\s+run\s+(\S+)")


def descendants(pid):
    out, stack, seen = [], [pid], set()
    while stack:
        p = stack.pop()
        if p in seen:
            continue
        seen.add(p)
        out.append(p)
        stack.extend(children.get(p, []))
    return out


def detect(pid):
    for p in descendants(pid):
        args = procs.get(p, "")
        m = OLLAMA.search(args)
        if m:
            return "ollama", m.group(2)
        for name, rx in AGENTS:
            if rx.search(args):
                return name, None
    return None, None


# ---- Claude Code transcripts ------------------------------------------------------------------
def slug(path):
    return re.sub(r"[^A-Za-z0-9]", "-", path)


def newest_transcript(cwd):
    best, best_m = None, 0
    for d in dirs:
        for f in glob.glob(os.path.join(d, "projects", slug(cwd), "*.jsonl")):
            try:
                m = os.stat(f).st_mtime
            except OSError:
                continue
            if m > best_m:
                best, best_m = f, m
    return best


def tail_lines(path, limit=1 << 20):
    try:
        with open(path, "rb") as fh:
            fh.seek(0, os.SEEK_END)
            size = fh.tell()
            fh.seek(max(0, size - limit))
            data = fh.read()
    except OSError:
        return []
    lines = data.decode("utf-8", "replace").splitlines()
    return lines[1:] if size > limit else lines


LIMIT_RX = re.compile(r"(usage|rate) limit|limit reached|limit will reset", re.I)
RESET_RX = re.compile(r"\|(\d{10})\b")


def ts_epoch(s):
    try:
        return datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp()
    except Exception:
        return None


STATE_HOME = os.environ.get("SSH_APP_HOME") or os.path.expanduser("~/.ssh-app")
LIMITS_DIR = os.path.join(STATE_HOME, "run", "limits")


def limit_label(minutes):
    if not isinstance(minutes, (int, float)) or minutes <= 0:
        return "limit"
    if minutes % 1440 == 0:
        return "%dd" % (minutes // 1440)
    if minutes % 60 == 0:
        return "%dh" % (minutes // 60)
    return "%dm" % minutes


def recorded_limits(config_dir):
    """Plan limits the Claude status line wrapper recorded for this config directory."""
    want = os.path.realpath(config_dir)
    best = None
    try:
        names = os.listdir(LIMITS_DIR)
    except OSError:
        return []
    for name in names:
        if not name.endswith(".json"):
            continue
        try:
            st = os.lstat(os.path.join(LIMITS_DIR, name))
            if st.st_size > 8192 or st.st_uid != os.geteuid():
                continue
            with open(os.path.join(LIMITS_DIR, name), "r", encoding="utf-8") as fh:
                d = json.load(fh)
        except Exception:
            continue
        if isinstance(d, dict) and d.get("dir") == want and (best is None or d.get("ts", 0) > best.get("ts", 0)):
            best = d
    out = []
    for key, label in (("five_hour", "5h"), ("seven_day", "7d")):
        w = (best or {}).get(key)
        if not isinstance(w, dict) or not isinstance(w.get("used_percentage"), (int, float)):
            continue
        reset = w.get("resets_at")
        reset = int(reset) if isinstance(reset, (int, float)) else None
        if reset is not None and reset < NOW:
            continue  # the window rolled over since this was recorded
        out.append({"label": label, "percent": int(round(max(0, min(100, w["used_percentage"])))), "resets_epoch": reset})
    return out


def claude_state(cwd):
    f = newest_transcript(cwd)
    if not f:
        return {}
    out = {}
    for d in dirs:
        if f.startswith(os.path.join(d, "projects") + os.sep):
            out["limits"] = recorded_limits(d)
            break
    for line in reversed(tail_lines(f)):
        try:
            e = json.loads(line)
        except Exception:
            continue
        msg = e.get("message") if isinstance(e, dict) else None
        if not isinstance(msg, dict):
            continue
        if "context_used" not in out and e.get("type") == "assistant" and isinstance(msg.get("usage"), dict) and not e.get("isSidechain") and not e.get("isApiErrorMessage"):
            u = msg["usage"]
            used = sum(int(u.get(k) or 0) for k in ("input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens", "output_tokens"))
            out["context_used"] = used
            out["context_limit"] = 1000000 if used > 200000 else 200000
            out["model"] = msg.get("model")
        if "limit_note" not in out and (e.get("isApiErrorMessage") or e.get("type") == "assistant"):
            t = e.get("timestamp")
            ts = ts_epoch(t) if isinstance(t, str) else None
            if ts and NOW - ts < 6 * 3600:
                content = msg.get("content")
                text = ""
                if isinstance(content, list):
                    text = " ".join(c.get("text", "") for c in content if isinstance(c, dict))
                elif isinstance(content, str):
                    text = content
                if text and LIMIT_RX.search(text) and (e.get("isApiErrorMessage") or len(text) < 400):
                    out["limit_note"] = text.strip()
                    m = RESET_RX.search(text)
                    if m:
                        out["limit_reset_epoch"] = int(m.group(1))
        if "context_used" in out and "limit_note" in out:
            break
    return out


# ---- Codex rate limits (recorded by Codex itself in its session files) ------------------------
def codex_limits():
    home = os.environ.get("CODEX_HOME") or os.path.expanduser("~/.codex")
    best, best_m = None, 0
    for f in glob.glob(os.path.join(home, "sessions", "*", "*", "*", "rollout-*.jsonl")):
        try:
            m = os.stat(f).st_mtime
        except OSError:
            continue
        if m > best_m:
            best, best_m = f, m
    if not best:
        return []
    for line in reversed(tail_lines(best)):
        if "rate_limits" not in line:
            continue
        try:
            e = json.loads(line)
        except Exception:
            continue
        payload = e.get("payload") if isinstance(e, dict) else None
        rl = (payload or {}).get("rate_limits") if isinstance(payload, dict) else None
        if not isinstance(rl, dict):
            rl = e.get("rate_limits") if isinstance(e, dict) else None
        if not isinstance(rl, dict):
            continue
        at = ts_epoch(e.get("timestamp") or "") or NOW
        out = []
        for key in ("primary", "secondary"):
            w = rl.get(key)
            if not isinstance(w, dict) or not isinstance(w.get("used_percent"), (int, float)):
                continue
            reset = w.get("resets_at")
            if not isinstance(reset, (int, float)) and isinstance(w.get("resets_in_seconds"), (int, float)):
                reset = at + w["resets_in_seconds"]
            reset = int(reset) if isinstance(reset, (int, float)) else None
            if reset is not None and reset < NOW:
                continue
            out.append({"label": limit_label(w.get("window_minutes")), "percent": int(round(max(0, min(100, w["used_percent"])))), "resets_epoch": reset})
        return out
    return []


# ---- rolling window totals ----------------------------------------------------------------
def window_totals():
    five, week = [0, 0, 0, 0, 0], [0, 0, 0, 0, 0]
    seen = set()
    partial = False
    cutoff = NOW - 7 * 86400
    for d in dirs:
        for f in glob.glob(os.path.join(d, "projects", "**", "*.jsonl"), recursive=True):
            if time.time() > BUDGET_END:
                partial = True
                break
            try:
                if os.stat(f).st_mtime < cutoff:
                    continue
                fh = open(f, "r", encoding="utf-8", errors="replace")
            except OSError:
                continue
            with fh:
                for line in fh:
                    if '"usage"' not in line or '"assistant"' not in line:
                        continue
                    try:
                        e = json.loads(line)
                    except Exception:
                        continue
                    msg = e.get("message")
                    if not isinstance(msg, dict) or not isinstance(msg.get("usage"), dict):
                        continue
                    ts = ts_epoch(e.get("timestamp") or "")
                    if not ts or ts < cutoff:
                        continue
                    key = (msg.get("id"), e.get("requestId"))
                    if key != (None, None):
                        if key in seen:
                            continue
                        seen.add(key)
                    u = msg["usage"]
                    vals = [int(u.get(k) or 0) for k in ("input_tokens", "output_tokens", "cache_creation_input_tokens", "cache_read_input_tokens")] + [1]
                    for i, v in enumerate(vals):
                        week[i] += v
                        if NOW - ts <= 5 * 3600:
                            five[i] += v
    return five, week, partial


def ollama_loaded():
    try:
        with urllib.request.urlopen("http://127.0.0.1:11434/api/ps", timeout=2) as r:
            data = json.loads(r.read().decode())
    except Exception:
        return []
    out = []
    for m in data.get("models", []):
        exp = m.get("expires_at")
        out.append({
            "name": m.get("name") or m.get("model") or "",
            "size": int(m.get("size") or 0),
            "size_vram": int(m.get("size_vram") or 0),
            "context_length": m.get("context_length"),
            "expires_epoch": int(ts_epoch(exp)) if isinstance(exp, str) and ts_epoch(exp) else None,
        })
    return out


sessions = []
panes = run(["tmux", "list-panes", "-a", "-F", "#{session_name}\t#{pane_pid}\t#{pane_current_path}"])
seen_sessions = set()
for line in panes.splitlines():
    parts = line.split("\t")
    if len(parts) < 3 or parts[0] in seen_sessions or not parts[1].isdigit():
        continue
    name, pid, cwd = parts[0], int(parts[1]), parts[2]
    agent, arg = detect(pid)
    if agent is None:
        continue
    seen_sessions.add(name)
    s = {"session": name, "agent": agent}
    if agent == "claude":
        s.update(claude_state(cwd))
    elif agent == "codex":
        s["limits"] = codex_limits()
    elif agent == "ollama" and arg:
        s["ollama_model"] = arg
    sessions.append(s)

five, week, partial = window_totals()
keys = ["input", "output", "cache_write", "cache_read", "messages"]
print(json.dumps({
    "sessions": sessions,
    "ollama": ollama_loaded(),
    "five_hour": dict(zip(keys, five)),
    "week": dict(zip(keys, week)),
    "partial": partial,
}))
