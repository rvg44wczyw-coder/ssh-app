//! `ssh-app-host.exe` for native Windows (feature 34). Only `mux` exists so far:
//!
//! * `mux ensure`  starts the per-user server if it is not running (detached, no window)
//! * `mux serve`   the server (named pipe, ConPTY sessions)
//! * `mux attach --spec <hex>`  connects the SSH channel's console to a session
//! * `mux list | kill --session NAME | detach --session NAME`
//!
//! Everything OS-specific is behind `cfg(windows)`; the logic (screen model, framing, sessions) is in
//! `ssh-app-mux` and is tested on every platform.

#[cfg(windows)]
mod cli;
#[cfg(windows)]
mod conpty;
#[cfg(windows)]
mod pipe;

#[cfg(windows)]
fn main() {
    std::process::exit(cli::run(std::env::args().skip(1).collect()));
}

#[cfg(not(windows))]
fn main() {
    eprintln!("ssh-app-host-win only runs on Windows");
    std::process::exit(2);
}
