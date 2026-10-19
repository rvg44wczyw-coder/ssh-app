//! Session registry and client handling of the multiplexer, independent of the operating system:
//! a [`PtyBackend`] starts a shell in a pseudo console (ConPTY on Windows, a Unix pty in the tests),
//! and a client is any `Read + Write` byte stream carrying [`Frame`]s (a named pipe on Windows).
//!
//! One client at a time is attached to a session; a new attach replaces the old one (like
//! `tmux attach -d`). The server keeps feeding the screen model while nobody is attached, so a later
//! attach is repainted with the current screen.

use crate::frame::{Decoder, Frame, FrameError};
use crate::screen::Screen;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

pub const PROTOCOL_VERSION: u8 = 1;
pub const MAX_SESSIONS: usize = 32;
pub const SCROLLBACK_LINES: usize = 10_000;

pub struct SpawnSpec {
    pub cols: u16,
    pub rows: u16,
    /// A command line to run instead of an interactive shell (`None` = the shell).
    pub command: Option<String>,
    pub start_dir: Option<String>,
}

pub trait Pty: Send + Sync + 'static {
    fn reader(&mut self) -> io::Result<Box<dyn Read + Send>>;
    fn writer(&mut self) -> io::Result<Box<dyn Write + Send>>;
    fn resize(&self, cols: u16, rows: u16) -> io::Result<()>;
    fn kill(&self);
}

pub trait PtyBackend: Send + Sync + 'static {
    fn spawn(&self, spec: &SpawnSpec) -> io::Result<Box<dyn Pty>>;
}

type Sink = Box<dyn Write + Send>;

struct Inner {
    screen: Screen,
    client: Option<(u64, Sink)>,
    exit: Option<i32>,
}

struct Session {
    inner: Mutex<Inner>,
    input: Mutex<Box<dyn Write + Send>>,
    pty: Box<dyn Pty>,
    created: u64,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, PartialEq, Eq)]
pub struct SessionInfo {
    pub name: String,
    pub attached: bool,
    pub created: u64,
}

pub struct Server {
    backend: Arc<dyn PtyBackend>,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    next_client: Mutex<u64>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Server {
    pub fn new(backend: Arc<dyn PtyBackend>) -> Arc<Server> {
        Arc::new(Server {
            backend,
            sessions: Mutex::new(HashMap::new()),
            next_client: Mutex::new(0),
        })
    }

    fn get(&self, name: &str) -> Option<Arc<Session>> {
        lock(&self.sessions).get(name).cloned()
    }

    pub fn has(&self, name: &str) -> bool {
        self.get(name)
            .is_some_and(|s| lock(&s.inner).exit.is_none())
    }

    /// Starts a session unless one with the name is running. Returns whether it was created.
    pub fn create(&self, name: &str, spec: &SpawnSpec) -> io::Result<bool> {
        let mut sessions = lock(&self.sessions);
        sessions.retain(|_, s| lock(&s.inner).exit.is_none());
        if sessions.contains_key(name) {
            return Ok(false);
        }
        if sessions.len() >= MAX_SESSIONS {
            return Err(io::Error::other("too many sessions"));
        }
        let mut pty = self.backend.spawn(spec)?;
        let mut reader = pty.reader()?;
        let writer = pty.writer()?;
        let session = Arc::new(Session {
            inner: Mutex::new(Inner {
                screen: Screen::new(spec.cols as usize, spec.rows as usize, SCROLLBACK_LINES),
                client: None,
                exit: None,
            }),
            input: Mutex::new(writer),
            pty,
            created: now(),
        });
        let pump = Arc::clone(&session);
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut inner = lock(&pump.inner);
                        inner.screen.feed(&buf[..n]);
                        let frame = Frame::Output(buf[..n].to_vec()).encode();
                        if let Some((_, sink)) = inner.client.as_mut() {
                            if sink.write_all(&frame).and_then(|_| sink.flush()).is_err() {
                                inner.client = None;
                            }
                        }
                    }
                }
            }
            let mut inner = lock(&pump.inner);
            inner.exit = Some(0);
            if let Some((_, sink)) = inner.client.as_mut() {
                let _ = sink.write_all(&Frame::Exit(0).encode());
                let _ = sink.flush();
            }
            inner.client = None;
        });
        sessions.insert(name.to_string(), session);
        Ok(true)
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        let mut out: Vec<SessionInfo> = lock(&self.sessions)
            .iter()
            .filter(|(_, s)| lock(&s.inner).exit.is_none())
            .map(|(n, s)| SessionInfo {
                name: n.clone(),
                attached: lock(&s.inner).client.is_some(),
                created: s.created,
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Lines `name|windows|attached|created` like `tmux list-sessions -F`.
    pub fn list_lines(&self) -> String {
        self.list()
            .iter()
            .map(|s| format!("{}|1|{}|{}\n", s.name, u8::from(s.attached), s.created))
            .collect()
    }

    pub fn kill(&self, name: &str) -> bool {
        let Some(s) = lock(&self.sessions).remove(name) else {
            return false;
        };
        s.pty.kill();
        let mut inner = lock(&s.inner);
        inner.exit.get_or_insert(-1);
        inner.client = None;
        true
    }

    /// Detaches whoever is attached; the session keeps running.
    pub fn detach(&self, name: &str) -> bool {
        self.get(name)
            .is_some_and(|s| lock(&s.inner).client.take().is_some())
    }

    pub fn capture(&self, name: &str, with_scrollback: bool) -> Option<String> {
        self.get(name)
            .map(|s| lock(&s.inner).screen.text(with_scrollback))
    }

    pub fn title(&self, name: &str) -> Option<String> {
        self.get(name)
            .map(|s| lock(&s.inner).screen.title().to_string())
    }

    pub fn send_input(&self, name: &str, bytes: &[u8]) -> bool {
        let Some(s) = self.get(name) else {
            return false;
        };
        let mut w = lock(&s.input);
        w.write_all(bytes).and_then(|_| w.flush()).is_ok()
    }

    /// Serves one client until it detaches, disconnects or the session ends. `sink` is the write
    /// half of the stream, `source` the read half. The first frame must be `Hello`.
    pub fn serve_client(
        &self,
        mut source: impl Read,
        mut sink: Sink,
        spec: &SpawnSpec,
    ) -> Result<(), FrameError> {
        let mut dec = Decoder::default();
        let mut buf = [0u8; 4096];
        let name = loop {
            if let Some(f) = dec.next_frame()? {
                match f {
                    Frame::Hello { version, session } if version == PROTOCOL_VERSION => {
                        break session
                    }
                    _ => return Err(FrameError::Malformed("expected hello")),
                }
            }
            match source.read(&mut buf) {
                Ok(0) | Err(_) => return Ok(()),
                Ok(n) => dec.push(&buf[..n]),
            }
        };
        if crate::server::bad_name(&name) {
            return Err(FrameError::Malformed("session name"));
        }
        if self.create(&name, spec).is_err() {
            let _ = sink.write_all(&Frame::Exit(-2).encode());
            return Ok(());
        }
        let session = self
            .get(&name)
            .ok_or(FrameError::Malformed("session vanished"))?;
        let id = {
            let mut n = lock(&self.next_client);
            *n += 1;
            *n
        };
        {
            let mut inner = lock(&session.inner);
            if let Some(code) = inner.exit {
                let _ = sink.write_all(&Frame::Exit(code).encode());
                return Ok(());
            }
            let (cols, rows) = (spec.cols, spec.rows);
            if inner.client.is_none() && inner.screen.size() != (cols as usize, rows as usize) {
                inner.screen.resize(cols as usize, rows as usize);
                let _ = session.pty.resize(cols, rows);
            }
            let redraw = Frame::Output(inner.screen.redraw()).encode();
            if sink.write_all(&redraw).and_then(|_| sink.flush()).is_err() {
                return Ok(());
            }
            inner.client = Some((id, sink));
        }
        let result = loop {
            loop {
                match dec.next_frame() {
                    Ok(Some(Frame::Input(b))) => {
                        let mut w = lock(&session.input);
                        let _ = w.write_all(&b).and_then(|_| w.flush());
                    }
                    Ok(Some(Frame::Resize { cols, rows })) => {
                        if cols as usize <= crate::screen::MAX_COLS
                            && rows as usize <= crate::screen::MAX_ROWS
                        {
                            lock(&session.inner)
                                .screen
                                .resize(cols as usize, rows as usize);
                            let _ = session.pty.resize(cols, rows);
                        }
                    }
                    Ok(Some(Frame::Detach)) => {
                        self.drop_client(&session, id);
                        return Ok(());
                    }
                    Ok(Some(_)) => {}
                    Ok(None) => break,
                    Err(e) => {
                        self.drop_client(&session, id);
                        return Err(e);
                    }
                }
            }
            // Replaced by a newer client (or the session ended): this one is done.
            if lock(&session.inner).client.as_ref().map(|c| c.0) != Some(id) {
                break Ok(());
            }
            match source.read(&mut buf) {
                Ok(0) | Err(_) => break Ok(()),
                Ok(n) => dec.push(&buf[..n]),
            }
        };
        self.drop_client(&session, id);
        result
    }

    fn drop_client(&self, session: &Session, id: u64) {
        let mut inner = lock(&session.inner);
        if inner.client.as_ref().map(|c| c.0) == Some(id) {
            inner.client = None;
        }
    }
}

/// Session names are the same alphabet the app validates (`[A-Za-z0-9_.-]`, 1-64, no leading `-`).
pub fn bad_name(name: &str) -> bool {
    name.is_empty()
        || name.len() > 64
        || name.starts_with('-')
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::fd::{FromRawFd, OwnedFd};
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command};
    use std::sync::mpsc;
    use std::time::Duration;

    struct UnixPty {
        master: OwnedFd,
        child: Mutex<Child>,
    }

    impl Pty for UnixPty {
        fn reader(&mut self) -> io::Result<Box<dyn Read + Send>> {
            Ok(Box::new(std::fs::File::from(self.master.try_clone()?)))
        }
        fn writer(&mut self) -> io::Result<Box<dyn Write + Send>> {
            Ok(Box::new(std::fs::File::from(self.master.try_clone()?)))
        }
        fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
            use std::os::fd::AsRawFd;
            let ws = libc::winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            // SAFETY: a valid fd and a valid winsize.
            let rc = unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &ws) };
            if rc == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }
        fn kill(&self) {
            let mut c = lock(&self.child);
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    struct UnixBackend;

    impl PtyBackend for UnixBackend {
        fn spawn(&self, spec: &SpawnSpec) -> io::Result<Box<dyn Pty>> {
            let mut master = 0;
            let mut slave = 0;
            let mut ws = libc::winsize {
                ws_row: spec.rows,
                ws_col: spec.cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            // SAFETY: out-pointers are valid; fds are wrapped right away.
            let rc = unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut ws,
                )
            };
            if rc != 0 {
                return Err(io::Error::last_os_error());
            }
            let (master, slave) =
                unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
            let mut cmd = Command::new("/bin/sh");
            if let Some(c) = &spec.command {
                cmd.arg("-c").arg(c);
            }
            cmd.env("PS1", "$ ")
                .env_remove("TMUX")
                .env_remove("TMUX_PANE");
            cmd.stdin(slave.try_clone()?)
                .stdout(slave.try_clone()?)
                .stderr(slave);
            // SAFETY: only async-signal-safe calls between fork and exec.
            unsafe {
                cmd.pre_exec(|| {
                    libc::setsid();
                    libc::ioctl(0, libc::TIOCSCTTY as _, 0);
                    Ok(())
                });
            }
            let child = cmd.spawn()?;
            Ok(Box::new(UnixPty {
                master,
                child: Mutex::new(child),
            }))
        }
    }

    fn spec(cmd: Option<&str>) -> SpawnSpec {
        SpawnSpec {
            cols: 40,
            rows: 6,
            command: cmd.map(str::to_string),
            start_dir: None,
        }
    }

    /// A client over an in-memory pair of channels.
    struct Chan(mpsc::Receiver<Vec<u8>>, Vec<u8>);
    impl Read for Chan {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            if self.1.is_empty() {
                match self.0.recv_timeout(Duration::from_secs(10)) {
                    Ok(v) => self.1 = v,
                    Err(_) => return Ok(0),
                }
            }
            let n = out.len().min(self.1.len());
            out[..n].copy_from_slice(&self.1[..n]);
            self.1.drain(..n);
            Ok(n)
        }
    }
    struct Tx(mpsc::Sender<Vec<u8>>);
    impl Write for Tx {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0
                .send(b.to_vec())
                .map_err(|_| io::Error::other("closed"))?;
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct Client {
        to_server: mpsc::Sender<Vec<u8>>,
        from_server: mpsc::Receiver<Vec<u8>>,
        done: mpsc::Receiver<Result<(), FrameError>>,
    }

    fn connect(server: &Arc<Server>, name: &str, sp: SpawnSpec) -> Client {
        let (to_server, server_rx) = mpsc::channel();
        let (server_tx, from_server) = mpsc::channel();
        let (done_tx, done) = mpsc::channel();
        let s = Arc::clone(server);
        std::thread::spawn(move || {
            let r = s.serve_client(Chan(server_rx, Vec::new()), Box::new(Tx(server_tx)), &sp);
            let _ = done_tx.send(r);
        });
        to_server
            .send(
                Frame::Hello {
                    version: PROTOCOL_VERSION,
                    session: name.into(),
                }
                .encode(),
            )
            .unwrap();
        Client {
            to_server,
            from_server,
            done,
        }
    }

    impl Client {
        fn send(&self, f: Frame) {
            self.to_server.send(f.encode()).unwrap();
        }
        /// Everything received until `needle` shows up in the output text (or a timeout).
        fn wait_for(&self, needle: &str) -> String {
            let mut dec = Decoder::default();
            let mut text = String::new();
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while std::time::Instant::now() < deadline {
                if let Ok(chunk) = self.from_server.recv_timeout(Duration::from_millis(100)) {
                    dec.push(&chunk);
                    while let Ok(Some(f)) = dec.next_frame() {
                        if let Frame::Output(b) = f {
                            text.push_str(&String::from_utf8_lossy(&b));
                        }
                    }
                    if text.contains(needle) {
                        return text;
                    }
                }
            }
            panic!("did not see {needle:?} in {text:?}");
        }
    }

    #[test]
    fn typing_output_capture_detach_and_repaint() {
        let server = Server::new(Arc::new(UnixBackend));
        let c = connect(&server, "s1", spec(None));
        c.send(Frame::Input(b"echo hello-$((20+22))\n".to_vec()));
        c.wait_for("hello-42");
        assert!(server.has("s1"));
        assert_eq!(server.list_lines().lines().count(), 1);
        assert!(server.list_lines().starts_with("s1|1|1|"));
        assert!(server.capture("s1", true).unwrap().contains("hello-42"));
        c.send(Frame::Detach);
        assert!(c.done.recv_timeout(Duration::from_secs(5)).unwrap().is_ok());
        assert!(server.list_lines().starts_with("s1|1|0|"));
        // Output while nobody is attached still reaches the screen.
        assert!(server.send_input("s1", b"echo while-away\n"));
        for _ in 0..50 {
            if server.capture("s1", false).unwrap().contains("while-away") {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(server.capture("s1", false).unwrap().contains("while-away"));
        // A new client is repainted with the current screen.
        let c2 = connect(&server, "s1", spec(None));
        c2.wait_for("while-away");
        assert!(server.kill("s1"));
        assert!(!server.has("s1"));
    }

    #[test]
    fn a_second_client_replaces_the_first_and_resize_reaches_the_program() {
        let server = Server::new(Arc::new(UnixBackend));
        let a = connect(&server, "s2", spec(None));
        a.send(Frame::Input(b"echo ready\n".to_vec()));
        a.wait_for("ready");
        let b = connect(&server, "s2", spec(None));
        b.wait_for("ready");
        b.send(Frame::Resize {
            cols: 100,
            rows: 30,
        });
        b.send(Frame::Input(b"stty size\n".to_vec()));
        b.wait_for("30 100");
        assert_eq!(server.list().len(), 1);
        // Hello for a name with a bad alphabet is refused.
        let bad = connect(&server, "../x", spec(None));
        assert!(bad
            .done
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .is_err());
        server.kill("s2");
    }

    #[test]
    fn a_session_that_exits_reports_it_and_is_forgotten() {
        let server = Server::new(Arc::new(UnixBackend));
        let c = connect(&server, "s3", spec(Some("echo bye")));
        c.wait_for("bye");
        assert!(c
            .done
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .is_ok());
        assert!(!server.has("s3"));
        assert!(server.list().is_empty());
        // Hostile first frame.
        let (tx, rx) = mpsc::channel();
        let (stx, _srx) = mpsc::channel();
        tx.send(Frame::Input(b"x".to_vec()).encode()).unwrap();
        let r = server.serve_client(Chan(rx, Vec::new()), Box::new(Tx(stx)), &spec(None));
        assert!(r.is_err());
    }

    #[test]
    fn session_limit_is_enforced() {
        let server = Server::new(Arc::new(UnixBackend));
        for i in 0..MAX_SESSIONS {
            assert!(server
                .create(&format!("n{i}"), &spec(Some("sleep 30")))
                .unwrap());
        }
        assert!(server
            .create("one-too-many", &spec(Some("sleep 30")))
            .is_err());
        for i in 0..MAX_SESSIONS {
            server.kill(&format!("n{i}"));
        }
    }
}
