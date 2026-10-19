//! The per-user named pipe: its name carries the user's SID and its DACL lets only that SID in,
//! remote clients are refused, and the first instance is created with `FIRST_PIPE_INSTANCE` so a
//! process that squatted the name earlier makes the server fail instead of serving through it.

use ssh_app_mux::frame::Frame;
use ssh_app_mux::server::Server;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::windows::io::FromRawHandle;
use std::ptr::null_mut;
use std::sync::Arc;
use std::time::Duration;
use windows_sys::Win32::Foundation::{
    CloseHandle, LocalFree, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// How long the server stays up without any session before it exits by itself.
const IDLE_EXIT: Duration = Duration::from_secs(120);

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The current user's SID as `S-1-5-21-...`.
pub fn current_sid() -> io::Result<String> {
    // SAFETY: standard token query; buffers are sized by the first call and freed/closed here.
    unsafe {
        let mut token: HANDLE = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut len = 0u32;
        GetTokenInformation(token, TokenUser, null_mut(), 0, &mut len);
        let mut buf = vec![0u8; len as usize];
        let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len);
        CloseHandle(token);
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut sid_w: *mut u16 = null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut sid_w) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut n = 0;
        while *sid_w.add(n) != 0 {
            n += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(sid_w, n));
        LocalFree(sid_w.cast());
        Ok(s)
    }
}

pub fn pipe_name() -> io::Result<String> {
    Ok(format!(r"\\.\pipe\ssh-app-mux-{}", current_sid()?))
}

/// Connects to the server's pipe.
pub fn connect() -> io::Result<File> {
    OpenOptions::new().read(true).write(true).open(pipe_name()?)
}

/// Runs the server: one thread per client, until the idle timeout.
pub fn serve(server: Arc<Server>) -> io::Result<()> {
    let name = wide(&pipe_name()?);
    let sddl = wide(&format!("D:P(A;;GA;;;{})", current_sid()?));
    let mut descriptor = null_mut();
    // SAFETY: the SDDL string is NUL-terminated; the descriptor is freed never (process lifetime).
    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };

    let watcher = Arc::clone(&server);
    std::thread::spawn(move || {
        let mut idle = Duration::ZERO;
        loop {
            std::thread::sleep(Duration::from_secs(5));
            if watcher.list().is_empty() {
                idle += Duration::from_secs(5);
                if idle >= IDLE_EXIT {
                    std::process::exit(0);
                }
            } else {
                idle = Duration::ZERO;
            }
        }
    });

    let mut first = true;
    loop {
        let mut open_mode = PIPE_ACCESS_DUPLEX;
        if first {
            open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        // SAFETY: valid NUL-terminated name and attributes that outlive the call.
        let handle = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                open_mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                64 * 1024,
                64 * 1024,
                0,
                &attributes,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        first = false;
        // SAFETY: `handle` is a fresh pipe instance.
        let connected = unsafe { ConnectNamedPipe(handle, null_mut()) } != 0
            || io::Error::last_os_error().raw_os_error() == Some(ERROR_PIPE_CONNECTED as i32);
        if !connected {
            // SAFETY: closing the instance we created.
            unsafe { CloseHandle(handle) };
            continue;
        }
        // SAFETY: the handle is owned from here on by the `File`.
        let file = unsafe { File::from_raw_handle(handle) };
        let server = Arc::clone(&server);
        std::thread::spawn(move || {
            let Ok(read_half) = file.try_clone() else {
                return;
            };
            // A malformed client is dropped: the pipe closes, nothing is echoed back.
            let _ = server.serve_client(read_half, Box::new(file));
        });
    }
}

/// Sends one management line and returns the reply text.
pub fn control(line: &str) -> io::Result<String> {
    use ssh_app_mux::frame::Decoder;
    use std::io::Read;
    let mut f = connect()?;
    f.write_all(&Frame::Control(line.to_string()).encode())?;
    let mut dec = Decoder::default();
    let mut buf = [0u8; 4096];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            return Err(io::Error::other("server closed the pipe"));
        }
        dec.push(&buf[..n]);
        match dec.next_frame() {
            Ok(Some(Frame::Reply(t))) => return Ok(t),
            Ok(Some(_)) | Ok(None) => {}
            Err(_) => return Err(io::Error::other("bad reply")),
        }
    }
}
