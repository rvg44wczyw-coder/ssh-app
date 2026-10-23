# Review and security pass: Ports screen and runtime probes (2026-10-23)

Scope: `crates/core/src/ports.rs`, `host_api/host_info.rs::fetch_listening_ports`, `PortsSection.swift`, the probe in `SessionsHomeView`.

## rev
- The host command is a fixed string (`ss -ltnpH`, or `lsof -nP -iTCP -sTCP:LISTEN` on macOS); nothing from the app is interpolated. Read only.
- The parser is pure: strict port parse (`u16`), control characters stripped, names capped at 60 characters, 500 ports at most, IPv4/IPv6 sockets merged. Tests: Linux and macOS formats, hostile lines, a 2000-line flood. The real `lsof` output of this Mac was checked against the parser's expectations.
- The probe results only decide which buttons are shown; unprobed servers show all buttons as before. Pull to refresh asks again; no timers.

## hacker (local only)
- Program names and ports are display text: never executed or sent as keys. A tap opens the web preview on a port number (`u16`) through the existing tunnel code, which already refuses anything but loopback of the host.
- Info shown (names of listening programs, pids) is what the owner could read with `ss`/`lsof` on the host; it is not logged.
- Verdict: clean.

## Controversial
- Ports of other accounts show without a program name on macOS/Linux when `lsof`/`ss` is not allowed to say. Accepted: the row shows "—".
