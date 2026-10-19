# Windows host: tester guide

Native Windows host support (feature 34, `docs/features/34-native-windows-host.md`) has never run on Windows. Everything below is what to check and what to report. Wine only proved that the exe starts and that the named pipe, `mux list` and `mux kill` answer.

## Needs
- Windows 10 1809+ or 11, x86_64 (Windows 11 ARM runs the x64 exe through emulation).
- The iPhone app build with the Windows blob embedded (`make iphone-full`, owner does this), the PC and the phone on one Tailscale network.

## Setup (in the app: server card, "Host helper", "Windows setup")
1. Run the shown PowerShell snippets as Administrator: OpenSSH Server, `DefaultShell` = cmd.exe, the app's public key in `administrators_authorized_keys` (or the user's file), Tailscale.
2. Add the server in the app and open a tab.

## Checklist (report pass/fail and the exact error text)
1. Host detection shows "Windows"; battery/install buttons hidden.
2. First connect installs `%USERPROFILE%\.ssh-app\bin\ssh-app-host-<12hex>.exe` (certutil; hash check). Does Defender or SmartScreen block it? Which message?
3. A tab opens a PowerShell prompt (PowerShell 7 if installed, else Windows PowerShell, else cmd). Colours, cursor, `dir`, arrow keys, Ctrl+C.
4. Resize: rotate the phone, the prompt reflows (`$Host.UI.RawUI.WindowSize`).
5. Background the app for a minute, return: the same session is re-attached with its screen and scrollback.
6. Two tabs = two isolated sessions. Session list shows both; ending one in the list kills it.
7. A long command (`ping -t 127.0.0.1`) keeps running while the app is in the background.
8. Quoting: a tab whose project folder has spaces, `&`, `%` or `^` in its name.
9. Idle exit: with no sessions the mux server exits after about 2 minutes (`tasklist | findstr ssh-app-host`).
10. Pipe access: from another Windows user, `\\.\pipe\ssh-app-mux-<SID>` is not reachable.

Not expected to work in v1: agent feed, hooks, git sheets, usage, transcript, file browser, kill from the helper (POSIX-shell based). Please do not report these.

## Manual commands (from an SSH session on the PC)
`.ssh-app\bin\ssh-app-host-<12hex>.exe mux list | capture --session NAME | kill --session NAME`
