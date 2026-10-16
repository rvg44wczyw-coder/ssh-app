//! Integration tests for the exec-based `SshSessionHandle` methods (telemetry, file browser,
//! history) against a throwaway `sshd` bound to 127.0.0.1 on an ephemeral port.
//! Skipped (with a message) when `sshd`, `ssh-keygen` or `tmux` are not available.

use ssh_core::{
    generate_keypair, SessionConfig, SessionState, SshSessionCallback, SshSessionHandle,
};
use std::net::TcpListener;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Noop;
impl SshSessionCallback for Noop {
    fn on_state_changed(&self, _state: SessionState) {}
    fn on_data_received(&self, _data: Vec<u8>) {}
    fn on_error(&self, _message: String) {}
}

struct Sshd {
    child: Child,
    port: u16,
    private_key: String,
    _dir: tempfile::TempDir,
}

impl Drop for Sshd {
    fn drop(&mut self) {
        // The session's tmux server lives on this same host; don't leave it behind.
        let _ = Command::new("tmux")
            .args(["kill-session", "-t", &format!("itest-{}", self.port)])
            .stderr(Stdio::null())
            .status();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn have(bin: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {bin}")])
        .stdout(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn sshd_path() -> Option<PathBuf> {
    ["/usr/sbin/sshd", "/usr/bin/sshd", "/opt/homebrew/sbin/sshd"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
}

fn start_sshd() -> Option<Sshd> {
    let sshd = sshd_path()?;
    if !have("ssh-keygen") || !have("tmux") {
        return None;
    }
    let dir = tempfile::tempdir().ok()?;
    let host_key = dir.path().join("host_key");
    let ok = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(&host_key)
        .status()
        .ok()?
        .success();
    if !ok {
        return None;
    }
    let keys = generate_keypair().ok()?;
    let authorized = dir.path().join("authorized_keys");
    std::fs::write(&authorized, format!("{}\n", keys.public_key_openssh)).ok()?;

    // Reserve an ephemeral loopback port, then hand it to sshd.
    let port = TcpListener::bind("127.0.0.1:0")
        .ok()?
        .local_addr()
        .ok()?
        .port();
    let cfg = dir.path().join("sshd_config");
    std::fs::write(
        &cfg,
        format!(
            "Port {port}\nListenAddress 127.0.0.1\nHostKey {}\nAuthorizedKeysFile {}\n\
             PidFile {}\nUsePAM no\nStrictModes no\nPubkeyAuthentication yes\n\
             PasswordAuthentication no\nKbdInteractiveAuthentication no\n",
            host_key.display(),
            authorized.display(),
            dir.path().join("sshd.pid").display()
        ),
    )
    .ok()?;
    let child = Command::new(sshd)
        .args(["-D", "-e", "-f"])
        .arg(&cfg)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return Some(Sshd {
                child,
                port,
                private_key: keys.private_key_openssh,
                _dir: dir,
            });
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

fn connect(server: &Sshd) -> Arc<SshSessionHandle> {
    let handle = SshSessionHandle::new(SessionConfig {
        host: "127.0.0.1".into(),
        port: server.port,
        username: std::env::var("USER").unwrap_or_else(|_| "root".into()),
        private_key_pem: server.private_key.clone(),
        session_name: format!("itest-{}", server.port),
        initial_cols: 80,
        initial_rows: 24,
        command: None,
    });
    handle.connect(Box::new(Noop)).expect("connect");
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if handle.state() == SessionState::Connected {
            return handle;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("session did not reach Connected: {:?}", handle.state());
}

fn fixture(root: &Path) -> PathBuf {
    let work = root.join("work");
    let ssh = root.join("home/.ssh");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::write(work.join("hello.txt"), "line1\nline2\n").unwrap();
    std::fs::write(work.join("blob.bin"), [0x89, b'P', 0, 1, 2]).unwrap();
    std::fs::write(work.join(".hidden"), "h").unwrap();
    std::fs::create_dir(work.join("sub")).unwrap();
    std::fs::write(ssh.join("id_rsa"), "PRIVATE").unwrap();
    symlink(ssh.join("id_rsa"), work.join("notes.txt")).unwrap();
    symlink(&ssh, work.join("innocent")).unwrap();
    work
}

#[test]
fn exec_based_methods_against_loopback_sshd() {
    let Some(server) = start_sshd() else {
        eprintln!("SKIP: sshd/ssh-keygen/tmux unavailable");
        return;
    };
    let handle = connect(&server);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    // Canonicalize: on macOS /var -> /private/var, and `pwd -P` reports the real path.
    let root = tmp.path().canonicalize().unwrap();
    let work = fixture(&root);
    let work_s = work.to_str().unwrap().to_string();

    rt.block_on(async {
        // --- telemetry ---
        let snap = handle.fetch_host_telemetry().await.expect("telemetry");
        assert!(snap.cpu_cores >= 1);
        assert!(snap.memory_total_mb > 0);

        // --- directory listing ---
        let listing = handle
            .list_remote_directory(Some(work_s.clone()), false)
            .await
            .expect("list");
        assert_eq!(listing.current_path, work_s);
        let names: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"hello.txt") && names.contains(&"sub"));
        assert!(!names.contains(&".hidden"), "hidden files must be filtered");
        let with_hidden = handle
            .list_remote_directory(Some(work_s.clone()), true)
            .await
            .expect("list hidden");
        assert!(with_hidden.entries.iter().any(|e| e.name == ".hidden"));

        // A symlinked directory pointing into .ssh must be refused after resolution.
        let via_link = format!("{work_s}/innocent");
        assert!(handle
            .list_remote_directory(Some(via_link), true)
            .await
            .is_err());
        // Direct access to a blocked directory is refused before any command runs.
        assert!(handle
            .list_remote_directory(Some(format!("{}/home/.ssh", root.display())), true)
            .await
            .is_err());

        // --- file preview ---
        let ok = handle
            .read_remote_file_preview(format!("{work_s}/hello.txt"), 4096)
            .await
            .expect("preview");
        assert_eq!(ok.content, "line1\nline2\n");
        assert!(!ok.is_binary && !ok.is_truncated);

        let bin = handle
            .read_remote_file_preview(format!("{work_s}/blob.bin"), 4096)
            .await
            .expect("binary preview");
        assert!(bin.is_binary);

        // Symlink named innocently, pointing at a private key: must not leak.
        let leaked = handle
            .read_remote_file_preview(format!("{work_s}/notes.txt"), 4096)
            .await;
        assert!(leaked.is_err(), "symlink to id_rsa must be blocked");

        // Directories are not regular files.
        assert!(handle
            .read_remote_file_preview(format!("{work_s}/sub"), 4096)
            .await
            .map(|p| !p.content.contains("PRIVATE"))
            .unwrap_or(true));

        // Missing file.
        assert!(handle
            .read_remote_file_preview(format!("{work_s}/nope.txt"), 4096)
            .await
            .is_err());
    });

    handle.disconnect();
}
