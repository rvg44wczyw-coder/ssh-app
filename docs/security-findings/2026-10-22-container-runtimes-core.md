# Security review (hacker): container runtimes core, feature 51

Date: 2026-10-22. Branch `rust-core-dev/container-runtimes` at `50d004f`.
Scope: `crates/core/src/container_runtime.rs`, the `docker.rs` changes, `host_api/docker_ops.rs`.

## Scope

Attacker model: a **container (or pod, or instance) whose metadata the attacker controls** on a host the
user manages from the phone (a shared box, a compromised build container, a hostile image). The attacker
can set names, labels, annotations, statuses; they cannot run commands as the SSH user. A second model:
a hostile host answering the exec channel with arbitrary text.

Applied: input validation of everything parsed from host output, spoofing of what the phone shows,
command injection through identifiers, DoS by large or deep output. Ruled out: crypto, keys, MITM
and replay (no new protocol; the transport is the existing SSH session), downgrade (no negotiation).

Live tests actually run (all local, nothing touched the real Docker's containers, the real tmux server
or a cluster): a separate scratch crate linked to the branch's core, feeding crafted host output into the
public parsers, release build:

- unit separator inside a label value (Podman/nerdctl list);
- newline inside a label value carrying a forged, complete list line;
- bidi override in a name; ANSI/OSC sequences in an error line;
- a 3 MB `[[[[` JSON depth bomb as a pod list; a 3 MB list of 12,884 pods with `restartCount = u64::MAX`;
  1,000,000 Incus lines; a 400,000-line forged stack verdict; the Docker marker text inside a JSON field;
- the branch's own property test (4,000 random identifiers through every builder, scripts run with fake
  binaries and a canary file) was re-run: no injected code ran.

Also checked live against Docker (read-only `docker ps --format`): the template action
`{{json (print (.Label "a") (.Label "b"))}}` is valid and JSON-escapes control characters.

## Findings

1. **InputValidation (spoofing and hiding), Medium.** `family_template()` prints label values raw, and
   `parse_family_line` splits on U+001F and on newlines. Label values may contain both.
   *Hide:* a label `p<US>q` makes the line 10 fields long, so the whole container is dropped from the
   Podman/nerdctl list (live: 2 containers in, 1 shown).
   *Forge:* a label `p\n<8-field forged line>` is completed by the real line's own `<US>SERVICE=` tail into
   a valid 9-field line: the attacker's real container disappears and a forged entry
   `prod-db / running / "Up 9 days"` is shown instead (live, id and state chosen by the attacker). The
   Docker projects section has the same flaw but can only attach project/service text to an existing id.
   Impact: a hostile container can evade the phone's view of the host, or make a fake healthy-looking
   database. Actions on a forged id fail harmlessly ("no such container") or hit an id the attacker
   named, which is a container that exists and is validated, so this is deception, not code execution.
   *Fix direction:* have the runtime JSON-escape the two label values in the template
   (`{{json (print (.Label "com.docker.compose.project") (.Label "io.podman.compose.project"))}}`,
   verified valid on Docker), parse them as JSON strings, and treat a line that does not parse exactly as
   an "unreadable entry" that is counted and shown ("N entries could not be read") instead of silently
   dropped. Keep the labels as the last fields; reject duplicate ids (first wins).
2. **InputValidation, Low.** Lines dropped for being the wrong shape are not counted anywhere, so any
   future parse mismatch on a real host also hides containers without a trace. Covered by the same fix
   (the "could not be read" count); `DockerContainersSnapshot` needs a field for it.

Not findings, tested: depth bomb rejected in 7 ms (serde's recursion limit); 12,884 pods parsed in 34 ms
and cut to 200 with the true total; 1M Incus lines in 0.2 s; stack-verdict flood in 8 ms and ids not given to
the script are ignored; bidi overrides and ANSI/OSC sequences are stripped by `clean`; the marker as a
whole line (fixed in review) holds against a marker inside a JSON field; no identifier that passes a
validator contains anything but `[A-Za-z0-9._/-]`.

## Verdict

Findings (highest severity Medium): 1 and 2 to fix before merge.
