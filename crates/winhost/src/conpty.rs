//! ConPTY backend for the multiplexer server.

use ssh_app_mux::server::{Pty, PtyBackend, SpawnSpec};
use ssh_app_mux::winquote::quote_arg;
use std::ffi::c_void;
use std::fs::File;
use std::io::{self, Read, Write};
use std::mem::{size_of, zeroed};
use std::os::windows::io::FromRawHandle;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Arc;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::Console::{
    ClosePseudoConsole, CreatePseudoConsole, ResizePseudoConsole, COORD, HPCON,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, InitializeProcThreadAttributeList,
    TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, INFINITE, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
    STARTUPINFOEXW,
};

const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: usize = 0x0002_0016;

struct Shared {
    hpc: AtomicIsize,
    process: isize,
}

impl Shared {
    /// Closing the pseudo console ends the output pipe, so the reader sees EOF. Idempotent.
    fn close_console(&self) {
        let h = self.hpc.swap(0, Ordering::SeqCst);
        if h != 0 {
            // SAFETY: `h` was returned by CreatePseudoConsole and is closed once (swap above).
            unsafe { ClosePseudoConsole(h) };
        }
    }
}

pub struct ConPty {
    shared: Arc<Shared>,
    reader: Option<File>,
    writer: Option<File>,
}

impl Pty for ConPty {
    fn reader(&mut self) -> io::Result<Box<dyn Read + Send>> {
        self.reader
            .take()
            .map(|f| Box::new(f) as Box<dyn Read + Send>)
            .ok_or_else(|| io::Error::other("reader taken"))
    }

    fn writer(&mut self) -> io::Result<Box<dyn Write + Send>> {
        self.writer
            .take()
            .map(|f| Box::new(f) as Box<dyn Write + Send>)
            .ok_or_else(|| io::Error::other("writer taken"))
    }

    fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        let h = self.shared.hpc.load(Ordering::SeqCst);
        if h == 0 {
            return Err(io::Error::other("console closed"));
        }
        let size = COORD {
            X: cols as i16,
            Y: rows as i16,
        };
        // SAFETY: `h` is a live pseudo console handle.
        let hr = unsafe { ResizePseudoConsole(h, size) };
        if hr == 0 {
            Ok(())
        } else {
            Err(io::Error::other(format!("ResizePseudoConsole {hr:#x}")))
        }
    }

    fn kill(&self) {
        // SAFETY: the process handle stays valid until the waiter thread closes it, which only
        // happens after the process ended.
        unsafe { TerminateProcess(self.shared.process as HANDLE, 1) };
        self.shared.close_console();
    }
}

pub struct ConPtyBackend;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A program found on `PATH` (first hit), e.g. `pwsh.exe`.
fn which(exe: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(exe))
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
}

/// PowerShell 7 if installed, then Windows PowerShell, then `cmd.exe`: found, never assumed.
fn command_line(spec: &SpawnSpec) -> String {
    let shell = which("pwsh.exe")
        .or_else(|| which("powershell.exe"))
        .map(|p| (p, true))
        .unwrap_or_else(|| {
            (
                std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into()),
                false,
            )
        });
    let (exe, is_ps) = shell;
    let mut line = quote_arg(&exe);
    match (&spec.command, is_ps) {
        (Some(c), true) => line.push_str(&format!(" -NoLogo -NoExit -Command {}", quote_arg(c))),
        (None, true) => line.push_str(" -NoLogo"),
        (Some(c), false) => line.push_str(&format!(" /k {}", quote_arg(c))),
        (None, false) => {}
    }
    line
}

impl PtyBackend for ConPtyBackend {
    fn spawn(&self, spec: &SpawnSpec) -> io::Result<Box<dyn Pty>> {
        // SAFETY: plain Win32 calls; every handle created here is either closed on failure or
        // owned by a `File` / `Shared` afterwards.
        unsafe {
            let (mut in_r, mut in_w, mut out_r, mut out_w): (HANDLE, HANDLE, HANDLE, HANDLE) =
                (null_mut(), null_mut(), null_mut(), null_mut());
            if CreatePipe(&mut in_r, &mut in_w, null(), 0) == 0 {
                return Err(io::Error::last_os_error());
            }
            if CreatePipe(&mut out_r, &mut out_w, null(), 0) == 0 {
                let e = io::Error::last_os_error();
                CloseHandle(in_r);
                CloseHandle(in_w);
                return Err(e);
            }
            let mut hpc: HPCON = 0;
            let size = COORD {
                X: spec.cols as i16,
                Y: spec.rows as i16,
            };
            let hr = CreatePseudoConsole(size, in_r, out_w, 0, &mut hpc);
            CloseHandle(in_r);
            CloseHandle(out_w);
            if hr != 0 {
                CloseHandle(in_w);
                CloseHandle(out_r);
                return Err(io::Error::other(format!("CreatePseudoConsole {hr:#x}")));
            }

            let mut bytes = 0usize;
            InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut bytes);
            let mut attr_buf = vec![0u8; bytes];
            let attrs = attr_buf.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
            let fail = |hpc: HPCON, in_w: HANDLE, out_r: HANDLE| {
                ClosePseudoConsole(hpc);
                CloseHandle(in_w);
                CloseHandle(out_r);
            };
            if InitializeProcThreadAttributeList(attrs, 1, 0, &mut bytes) == 0 {
                let e = io::Error::last_os_error();
                fail(hpc, in_w, out_r);
                return Err(e);
            }
            if UpdateProcThreadAttribute(
                attrs,
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
                hpc as *const c_void,
                size_of::<HPCON>(),
                null_mut(),
                null(),
            ) == 0
            {
                let e = io::Error::last_os_error();
                DeleteProcThreadAttributeList(attrs);
                fail(hpc, in_w, out_r);
                return Err(e);
            }

            let mut si: STARTUPINFOEXW = zeroed();
            si.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
            si.lpAttributeList = attrs;
            let mut pi: PROCESS_INFORMATION = zeroed();
            let mut cmd = wide(&command_line(spec));
            let cwd = spec.start_dir.as_deref().map(wide);
            let ok = CreateProcessW(
                null(),
                cmd.as_mut_ptr(),
                null(),
                null(),
                0,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                null(),
                cwd.as_ref().map_or(null(), |c| c.as_ptr()),
                &si.StartupInfo,
                &mut pi,
            );
            let err = io::Error::last_os_error();
            DeleteProcThreadAttributeList(attrs);
            if ok == 0 {
                fail(hpc, in_w, out_r);
                return Err(err);
            }
            CloseHandle(pi.hThread);

            let shared = Arc::new(Shared {
                hpc: AtomicIsize::new(hpc),
                process: pi.hProcess as isize,
            });
            let waiter = Arc::clone(&shared);
            std::thread::spawn(move || {
                WaitForSingleObject(waiter.process as HANDLE, INFINITE);
                waiter.close_console();
                CloseHandle(waiter.process as HANDLE);
            });
            Ok(Box::new(ConPty {
                shared,
                reader: Some(File::from_raw_handle(out_r)),
                writer: Some(File::from_raw_handle(in_w)),
            }))
        }
    }
}
