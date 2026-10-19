//! Command line of `ssh-app-host.exe mux ...`.

use crate::conpty::ConPtyBackend;
use crate::pipe;
use ssh_app_mux::frame::{Decoder, Frame};
use ssh_app_mux::server::{bad_name, Server, PROTOCOL_VERSION};
use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows_sys::Win32::System::Console::{
    GetConsoleMode, GetConsoleScreenBufferInfo, GetStdHandle, SetConsoleMode,
    CONSOLE_SCREEN_BUFFER_INFO, ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT,
    ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
    STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};

const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

pub fn run(args: Vec<String>) -> i32 {
    let rest: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match rest.as_slice() {
        ["mux", "serve"] => serve(),
        ["mux", "ensure"] => ensure().map(|_| 0),
        ["mux", "attach", "--spec", spec] => attach(spec),
        ["mux", "list"] => manage("list"),
        ["mux", "kill", "--session", s] => manage(&format!("kill {s}")),
        ["mux", "detach", "--session", s] => manage(&format!("detach {s}")),
        ["mux", "capture", "--session", s] => manage(&format!("capture {s}")),
        ["mux", "capture", "--session", s, "--full"] => manage(&format!("capture {s} full")),
        _ => Err(
            "usage: mux serve|ensure|attach --spec HEX|list|kill|detach|capture --session NAME"
                .into(),
        ),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn serve() -> Result<i32, String> {
    let server = Server::new(Arc::new(ConPtyBackend));
    pipe::serve(server).map_err(|e| format!("server: {e}"))?;
    Ok(0)
}

/// Starts the server (detached, no window) unless the pipe already answers.
fn ensure() -> Result<(), String> {
    if pipe::connect().is_ok() {
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::process::Command::new(exe)
        .args(["mux", "serve"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .map_err(|e| format!("could not start the server: {e}"))?;
    for _ in 0..50 {
        if pipe::connect().is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err("the server did not start".into())
}

fn manage(line: &str) -> Result<i32, String> {
    let name_ok = line.split_whitespace().nth(1).is_none_or(|n| !bad_name(n));
    if !name_ok {
        return Err("bad session name".into());
    }
    if line == "list" {
        // Listing must not start a server just to say "nothing".
        if pipe::connect().is_err() {
            return Ok(0);
        }
    } else if pipe::connect().is_err() {
        return Err("no mux server is running".into());
    }
    let reply = pipe::control(line).map_err(|e| e.to_string())?;
    print!("{reply}");
    Ok(i32::from(reply.starts_with("error")))
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || s.len() > 20_000 {
        return None;
    }
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok())
        .collect()
}

fn console_size() -> (u16, u16) {
    // SAFETY: a zeroed info struct is a valid out-buffer.
    unsafe {
        let mut info: CONSOLE_SCREEN_BUFFER_INFO = std::mem::zeroed();
        if GetConsoleScreenBufferInfo(GetStdHandle(STD_OUTPUT_HANDLE), &mut info) != 0 {
            let w = (info.srWindow.Right - info.srWindow.Left + 1).max(1) as u16;
            let h = (info.srWindow.Bottom - info.srWindow.Top + 1).max(1) as u16;
            return (w, h);
        }
    }
    (80, 24)
}

/// Puts the console in raw, VT mode and restores the old modes when dropped.
struct RawConsole {
    saved: Vec<(isize, u32)>,
}

impl RawConsole {
    fn enter() -> RawConsole {
        let mut saved = Vec::new();
        // SAFETY: standard console mode calls on the process's own handles.
        unsafe {
            let input = GetStdHandle(STD_INPUT_HANDLE);
            let output = GetStdHandle(STD_OUTPUT_HANDLE);
            let mut mode = 0;
            if GetConsoleMode(input, &mut mode) != 0 {
                saved.push((input as isize, mode));
                let raw = (mode
                    & !(ENABLE_ECHO_INPUT | ENABLE_LINE_INPUT | ENABLE_PROCESSED_INPUT))
                    | ENABLE_VIRTUAL_TERMINAL_INPUT;
                SetConsoleMode(input, raw);
            }
            if GetConsoleMode(output, &mut mode) != 0 {
                saved.push((output as isize, mode));
                SetConsoleMode(
                    output,
                    mode | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
                );
            }
        }
        RawConsole { saved }
    }
}

impl Drop for RawConsole {
    fn drop(&mut self) {
        for (h, mode) in &self.saved {
            // SAFETY: restoring modes read earlier from the same handles.
            unsafe { SetConsoleMode(*h as _, *mode) };
        }
    }
}

/// Connects this process's console (the one sshd's ConPTY gave the SSH channel) to a session.
fn attach(spec_hex: &str) -> Result<i32, String> {
    let bytes = hex_decode(spec_hex).ok_or("bad spec")?;
    let spec: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| "bad spec")?;
    let session = spec["session"]
        .as_str()
        .ok_or("spec without session")?
        .to_string();
    if bad_name(&session) {
        return Err("bad session name".into());
    }
    let text = |k: &str| spec[k].as_str().map(str::to_string);
    ensure()?;
    let pipe_file = pipe::connect().map_err(|e| e.to_string())?;
    let mut reader = pipe_file.try_clone().map_err(|e| e.to_string())?;
    let writer: Arc<Mutex<File>> = Arc::new(Mutex::new(pipe_file));
    let (cols, rows) = console_size();
    let send = {
        let writer = Arc::clone(&writer);
        move |f: Frame| -> bool {
            let mut w = writer.lock().unwrap_or_else(|e| e.into_inner());
            w.write_all(&f.encode()).and_then(|_| w.flush()).is_ok()
        }
    };
    if !send(Frame::Hello {
        version: PROTOCOL_VERSION,
        session,
        cols,
        rows,
        command: text("command"),
        start_dir: text("start_dir"),
    }) {
        return Err("could not talk to the server".into());
    }

    let _raw = RawConsole::enter();

    // Keyboard -> session. End of input means the SSH connection is gone: detach, keep the session.
    let input_send = send.clone();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut buf = [0u8; 4096];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) | Err(_) => {
                    input_send(Frame::Detach);
                    std::process::exit(0);
                }
                Ok(n) => {
                    if !input_send(Frame::Input(buf[..n].to_vec())) {
                        std::process::exit(0);
                    }
                }
            }
        }
    });

    // Window size changes (the SSH client's window_change resizes this console).
    let resize_send = send.clone();
    std::thread::spawn(move || {
        let mut last = (cols, rows);
        loop {
            std::thread::sleep(Duration::from_millis(250));
            let now = console_size();
            if now != last {
                last = now;
                resize_send(Frame::Resize {
                    cols: now.0,
                    rows: now.1,
                });
            }
        }
    });

    // Session -> screen.
    let mut dec = Decoder::default();
    let mut buf = [0u8; 16 * 1024];
    let mut stdout = std::io::stdout().lock();
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(0);
        }
        dec.push(&buf[..n]);
        loop {
            match dec.next_frame() {
                Ok(Some(Frame::Output(b))) => {
                    let _ = stdout.write_all(&b);
                    let _ = stdout.flush();
                }
                Ok(Some(Frame::Exit(code))) => return Ok(code.max(0)),
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => return Err("bad data from the server".into()),
            }
        }
    }
}
