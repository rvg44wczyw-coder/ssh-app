//! Integration tests for the exec-based `SshSessionHandle` methods (telemetry, file browser,
//! history) against a throwaway `sshd` bound to 127.0.0.1 on an ephemeral port.
//! Skipped (with a message) when `sshd`, `ssh-keygen` or `tmux` are not available.

use ssh_core::{build_agent_launch_command, AgentKind, AgentLaunchProfile, EnvVar};
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
    home: PathBuf,
    /// Private tmux socket directory: the tests must never touch the developer's real tmux
    /// server (a test-started server would hand its fake `HOME` to every later session).
    tmux_tmp: PathBuf,
    port: u16,
    private_key: String,
    _dir: tempfile::TempDir,
}

impl Sshd {
    /// `tmux` against this server's private socket, and nothing else.
    ///
    /// `$TMUX` (set whenever the tests run inside a tmux pane, e.g. an agent session) outranks
    /// `TMUX_TMPDIR`, so without the explicit `-S` a `kill-server` here killed the developer's real
    /// tmux server and every session in it. The socket path is what tmux itself derives from
    /// `TMUX_TMPDIR` for the sshd-side sessions (`<dir>/tmux-<uid>/default`).
    fn tmux(&self) -> Command {
        use std::os::unix::fs::MetadataExt;
        let uid = std::fs::metadata(&self.tmux_tmp)
            .map(|m| m.uid())
            .expect("private tmux dir");
        let dir = self.tmux_tmp.join(format!("tmux-{uid}"));
        // With an explicit socket tmux does not create the folder itself.
        std::fs::create_dir_all(&dir).expect("private tmux socket dir");
        // tmux refuses a socket folder that is not private (the sshd-side tmux checks it).
        std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .expect("chmod private tmux socket dir");
        let sock = dir.join("default");
        let mut c = Command::new("tmux");
        c.env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("TMUX_TMPDIR", &self.tmux_tmp)
            .arg("-S")
            .arg(sock);
        c
    }
}

impl Drop for Sshd {
    fn drop(&mut self) {
        // The tests' own tmux server (private socket dir); don't leave it behind.
        let _ = self
            .tmux()
            .arg("kill-server")
            .stderr(Stdio::null())
            .status();
        let _ = std::fs::remove_dir_all(&self.tmux_tmp);
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
    // Sessions get a private HOME so installs never touch the developer's real ~/.ssh-app.
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).ok()?;
    // A short path: tmux socket paths are limited to ~100 characters.
    let tmux_tmp = std::env::temp_dir().join(format!("ssh-app-itest-tmux-{port}"));
    std::fs::create_dir_all(&tmux_tmp).ok()?;
    let cfg = dir.path().join("sshd_config");
    std::fs::write(
        &cfg,
        format!(
            "Port {port}\nListenAddress 127.0.0.1\nHostKey {}\nAuthorizedKeysFile {}\n\
             PidFile {}\nUsePAM no\nStrictModes no\nPubkeyAuthentication yes\n\
             PasswordAuthentication no\nKbdInteractiveAuthentication no\nSetEnv HOME={} TMUX_TMPDIR={}\n",
            host_key.display(),
            authorized.display(),
            dir.path().join("sshd.pid").display(),
            home.display(),
            tmux_tmp.display()
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
                home,
                tmux_tmp,
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
        start_dir: None,
        pinned_host_key: None,
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

#[test]
fn agent_hooks_install_flow_against_loopback_sshd() {
    use std::os::unix::fs::PermissionsExt;
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
    let root = tmp.path().canonicalize().unwrap();
    let cfg_dir = root.join("claude-personal");
    let hook_dir = root.join("bin");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    let settings = cfg_dir.join("settings.json");
    let original = r#"{"model":"opus","hooks":{"PermissionRequest":[{"matcher":"Bash","hooks":[{"type":"command","command":"mine"}]}]}}"#;
    std::fs::write(&settings, original).unwrap();
    std::fs::set_permissions(&settings, std::fs::Permissions::from_mode(0o640)).unwrap();
    let cfg = Some(cfg_dir.to_str().unwrap().to_string());
    let hooks = Some(hook_dir.to_str().unwrap().to_string());
    let agent = AgentKind::ClaudeCode;

    rt.block_on(async {
        let st = handle
            .agent_hooks_status(agent, cfg.clone(), hooks.clone())
            .await
            .unwrap();
        assert!(!st.hooks_installed && !st.script_installed);
        assert_eq!(
            st.config_path,
            format!("{}/settings.json", cfg_dir.display())
        );

        handle
            .install_agent_hooks(agent, cfg.clone(), hooks.clone())
            .await
            .expect("install");
        let st = handle
            .agent_hooks_status(agent, cfg.clone(), hooks.clone())
            .await
            .unwrap();
        assert!(st.hooks_installed && st.script_installed);

        // Script is private and executable by owner only; config keeps its mode; backup exists.
        let script = hook_dir.join("ssh-app-hook");
        assert_eq!(
            std::fs::metadata(&script).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert!(std::fs::read_to_string(&script)
            .unwrap()
            .contains("def main"));
        assert_eq!(
            std::fs::metadata(&settings).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert_eq!(
            std::fs::read_to_string(cfg_dir.join("settings.json.ssh-app.bak")).unwrap(),
            original
        );

        // The installed config keeps user settings and hooks.
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(v["model"], "opus");
        assert_eq!(
            v["hooks"]["PermissionRequest"][0]["hooks"][0]["command"],
            "mine"
        );

        // Idempotent.
        let after_first = std::fs::read_to_string(&settings).unwrap();
        handle
            .install_agent_hooks(agent, cfg.clone(), hooks.clone())
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), after_first);

        // Uninstall restores the user's hooks exactly.
        handle
            .uninstall_agent_hooks(agent, cfg.clone())
            .await
            .unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(
            v,
            serde_json::from_str::<serde_json::Value>(original).unwrap()
        );
        assert!(
            !handle
                .agent_hooks_status(agent, cfg.clone(), hooks.clone())
                .await
                .unwrap()
                .hooks_installed
        );

        // Invalid config is refused and left byte-for-byte untouched.
        std::fs::write(&settings, "{ not json").unwrap();
        assert!(handle
            .install_agent_hooks(agent, cfg.clone(), hooks.clone())
            .await
            .is_err());
        assert!(handle
            .uninstall_agent_hooks(agent, cfg.clone())
            .await
            .is_err());
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{ not json");

        // A symlinked settings.json is written through, not replaced.
        let real = root.join("dotfiles-settings.json");
        std::fs::write(&real, "{}").unwrap();
        std::fs::remove_file(&settings).unwrap();
        std::os::unix::fs::symlink(&real, &settings).unwrap();
        handle
            .install_agent_hooks(agent, cfg.clone(), hooks.clone())
            .await
            .unwrap();
        assert!(std::fs::symlink_metadata(&settings)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(std::fs::read_to_string(&real)
            .unwrap()
            .contains("ssh-app-hook"));

        // A pre-planted symlink at the temp-file path must not be followed (clobber attack).
        let victim = root.join("victim.txt");
        std::fs::write(&victim, "VICTIM").unwrap();
        let _ = std::fs::remove_file(root.join("claude-personal/settings.json.ssh-app.tmp"));
        std::os::unix::fs::symlink(
            &victim,
            root.join("claude-personal/settings.json.ssh-app.tmp"),
        )
        .unwrap();
        handle
            .uninstall_agent_hooks(agent, cfg.clone())
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "VICTIM");
        assert!(!std::fs::read_to_string(&real)
            .unwrap()
            .contains("ssh-app-hook"));

        // Antigravity uses hooks.json in its dir.
        let agy_dir = Some(root.join("gemini").to_str().unwrap().to_string());
        handle
            .install_agent_hooks(AgentKind::Antigravity, agy_dir.clone(), hooks.clone())
            .await
            .unwrap();
        let hj = std::fs::read_to_string(root.join("gemini/hooks.json")).unwrap();
        assert!(hj.contains("\"ssh-app\"") && hj.contains("run_command"));

        // Codex (CODEX_HOME/hooks.json) and Grok (own file in its hooks dir) install and remove.
        for (agent, dirname, file, event) in [
            (AgentKind::Codex, "codex", "hooks.json", "PermissionRequest"),
            (AgentKind::Grok, "grok-hooks", "ssh-app.json", "PreToolUse"),
        ] {
            let d = Some(root.join(dirname).to_str().unwrap().to_string());
            handle
                .install_agent_hooks(agent, d.clone(), hooks.clone())
                .await
                .unwrap();
            let text = std::fs::read_to_string(root.join(dirname).join(file)).unwrap();
            let v: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert!(v["hooks"][event].is_array(), "{agent:?}: {text}");
            assert!(
                handle
                    .agent_hooks_status(agent, d.clone(), hooks.clone())
                    .await
                    .unwrap()
                    .hooks_installed
            );
            handle
                .uninstall_agent_hooks(agent, d.clone())
                .await
                .unwrap();
            assert!(
                !handle
                    .agent_hooks_status(agent, d, hooks.clone())
                    .await
                    .unwrap()
                    .hooks_installed
            );
        }
    });

    handle.disconnect();
}

#[test]
fn launch_command_executes_with_literal_env_on_loopback_sshd() {
    let Some(server) = start_sshd() else {
        eprintln!("SKIP: sshd/ssh-keygen/tmux unavailable");
        return;
    };
    let handle = connect(&server);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let profile = |key: &str, value: &str| AgentLaunchProfile {
        id: "t".into(),
        name: "T".into(),
        agent: AgentKind::ClaudeCode,
        binary: "printenv".into(),
        env: vec![EnvVar {
            key: key.into(),
            value: value.into(),
        }],
        args: vec![key.into()],
        working_dir: None,
        git_branch: None,
        permission_mode: None,
        load_shell_rc: false,
    };
    rt.block_on(async {
        // `~/` expands to the remote $HOME (this is what the alias CLAUDE_CONFIG_DIR=... did).
        let cmd =
            build_agent_launch_command(profile("CLAUDE_CONFIG_DIR", "~/.claude-personal")).unwrap();
        // No pty here, so close stdin or the trailing login shell would wait for input forever.
        let out = handle
            .execute_command_on_active_session(format!("( {cmd} ) </dev/null"))
            .await
            .unwrap();
        let home = server.home.display();
        assert_eq!(out.trim(), format!("{home}/.claude-personal"));

        // A hostile value is passed literally: nothing is executed or expanded.
        let hostile = "a'; touch /tmp/ssh-app-pwned; echo '$(id) `id` $HOME";
        let cmd = build_agent_launch_command(profile("FOO", hostile)).unwrap();
        let out = handle
            .execute_command_on_active_session(format!("( {cmd} ) </dev/null"))
            .await
            .unwrap();
        assert_eq!(out.trim(), hostile);
        assert!(!std::path::Path::new("/tmp/ssh-app-pwned").exists());

        // Interactive (rc-loading) mode keeps the same literal-value guarantee. The user's rc
        // may print noise, so only the last line is compared.
        let mut p = profile("FOO", hostile);
        p.load_shell_rc = true;
        let cmd = build_agent_launch_command(p).unwrap();
        let out = handle
            .execute_command_on_active_session(format!("( {cmd} ) </dev/null 2>/dev/null"))
            .await
            .unwrap();
        assert_eq!(out.lines().last().map(str::trim), Some(hostile));
        assert!(!std::path::Path::new("/tmp/ssh-app-pwned").exists());

        // The environment check resolves a binary every shell can find.
        let check = handle
            .check_agent_environment("sh".to_string())
            .await
            .unwrap();
        assert!(check.login_path.is_some() && check.interactive_path.is_some());
        assert!(!check.interactive_timed_out);
        let missing = handle
            .check_agent_environment("definitely-not-installed-xyz".to_string())
            .await
            .unwrap();
        assert!(missing.login_path.is_none() && missing.interactive_path.is_none());

        // Ollama check: succeeds on any host and agrees with what this machine actually has.
        let ollama = handle.check_host_ollama().await.unwrap();
        let local = ["/opt/homebrew/bin/ollama", "/usr/local/bin/ollama"]
            .iter()
            .any(|p| Path::new(p).exists());
        if local {
            assert!(ollama.installed, "{ollama:?}");
        }
        assert!(ollama.models.iter().all(|m| !m.contains(' ')));
    });
    handle.disconnect();
}

#[test]
fn hook_presets_preview_and_custom_install_against_loopback_sshd() {
    use ssh_core::{default_hook_preset, HookPreset, HookPresetKind};
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
    let root = tmp.path().canonicalize().unwrap();
    let cfg_dir = root.join("claude-work");
    let hook_dir = root.join("bin");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    let settings = cfg_dir.join("settings.json");
    let original = r#"{"env":{"ANTHROPIC_API_KEY":"sk-topsecret"},"model":"opus"}"#;
    std::fs::write(&settings, original).unwrap();
    let cfg = Some(cfg_dir.to_str().unwrap().to_string());
    let hooks = Some(hook_dir.to_str().unwrap().to_string());
    let agent = AgentKind::ClaudeCode;
    let custom = HookPreset {
        kind: HookPresetKind::Custom,
        custom_command: "/usr/local/bin/audit --log".into(),
        custom_events: vec!["PreToolUse".into(), "Stop".into()],
        custom_matcher: "Bash".into(),
        approval_matcher: String::new(),
        approval_events: vec![],
    };

    rt.block_on(async {
        // Preview is a dry run and never reveals other settings.
        let preview = handle
            .preview_agent_hooks(agent, cfg.clone(), hooks.clone(), default_hook_preset())
            .await
            .unwrap();
        assert!(preview.contains("PermissionRequest"));
        assert!(!preview.contains("sk-topsecret"));
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), original);
        assert!(!hook_dir.exists());

        // Custom preset: no script written, entries are marked and listed as installed.
        handle
            .install_agent_hooks_with_preset(agent, cfg.clone(), hooks.clone(), custom.clone())
            .await
            .unwrap();
        assert!(!hook_dir.exists());
        let installed = handle
            .installed_agent_hooks(agent, cfg.clone())
            .await
            .unwrap();
        assert_eq!(installed.len(), 2);
        assert_eq!(installed[0].matcher.as_deref(), Some("Bash"));
        assert!(installed[0]
            .command
            .starts_with("/usr/local/bin/audit --log #"));
        let on_disk = std::fs::read_to_string(&settings).unwrap();
        assert!(on_disk.contains("sk-topsecret") && on_disk.contains("\"model\""));

        // Switching preset replaces our entries instead of stacking them.
        let mut notify = default_hook_preset();
        notify.kind = HookPresetKind::NotifyOnly;
        handle
            .install_agent_hooks_with_preset(agent, cfg.clone(), hooks.clone(), notify)
            .await
            .unwrap();
        let installed = handle
            .installed_agent_hooks(agent, cfg.clone())
            .await
            .unwrap();
        let events: Vec<_> = installed.iter().map(|h| h.event.as_str()).collect();
        assert_eq!(events, ["Notification", "Stop"]);

        handle
            .uninstall_agent_hooks(agent, cfg.clone())
            .await
            .unwrap();
        assert!(handle
            .installed_agent_hooks(agent, cfg.clone())
            .await
            .unwrap()
            .is_empty());
        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(after["model"], "opus");
        assert!(after.get("hooks").is_none());

        // A hostile custom command is stored as data and never executed by the app.
        let mut evil = custom;
        evil.custom_command = "true; touch /tmp/ssh-app-pwned-preset".into();
        handle
            .install_agent_hooks_with_preset(agent, cfg.clone(), hooks.clone(), evil)
            .await
            .unwrap();
        assert!(!Path::new("/tmp/ssh-app-pwned-preset").exists());
    });
    handle.disconnect();
}

#[test]
fn phone_approval_roundtrip_against_loopback_sshd() {
    use ssh_core::{default_hook_preset, AgentKind};
    if !have("python3") {
        eprintln!("SKIP: python3 unavailable");
        return;
    }
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
    let cfg_dir = tmp.path().canonicalize().unwrap().join("claude");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    let cfg = Some(cfg_dir.to_str().unwrap().to_string());
    let hook = server.home.join(".ssh-app/bin/ssh-app-hook");

    rt.block_on(async {
        // Before installation the queue does not exist: no items, status says why.
        assert!(handle.list_pending_approvals().await.unwrap().items.is_empty());
        let st = handle.approval_status().await.unwrap();
        assert!(!st.allowed_signers);

        // Installing the default (PhoneApproval) preset writes the script and trusts our key.
        handle
            .install_agent_hooks_with_preset(
                AgentKind::ClaudeCode,
                cfg.clone(),
                None,
                default_hook_preset(),
            )
            .await
            .unwrap();
        assert!(hook.exists());
        let st = handle.approval_status().await.unwrap();
        assert!(st.ssh_keygen && st.allowed_signers, "{st:?}");

        for (approved, expect) in [(true, "allow"), (false, "deny")] {
            let mut child = Command::new("python3")
                .arg("-I")
                .arg(&hook)
                .args(["claude", "PermissionRequest"])
                .env("SSH_APP_HOME", server.home.join(".ssh-app"))
                .env("SSH_APP_HOOK_TIMEOUT", "30")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            {
                use std::io::Write;
                let mut stdin = child.stdin.take().unwrap();
                stdin
                    .write_all(
                        br#"{"tool_name":"Bash","tool_input":{"command":"git push"},"cwd":"/w","session_id":"s"}"#,
                    )
                    .unwrap();
            }
            // The phone sees it...
            let deadline = Instant::now() + Duration::from_secs(10);
            let item = loop {
                let l = handle.list_pending_approvals().await.unwrap();
                if let Some(i) = l.items.into_iter().find(|i| i.kind == "approval") {
                    break i;
                }
                assert!(Instant::now() < deadline, "request never showed up");
                tokio::time::sleep(Duration::from_millis(100)).await;
            };
            assert_eq!((item.command.as_str(), item.tool.as_str()), ("git push", "Bash"));
            // ...and its signed decision releases the hook.
            handle
                .submit_approval_decision(item.id.clone(), approved)
                .await
                .unwrap();
            let out = child.wait_with_output().unwrap();
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(text.contains(expect), "{text}");
            assert!(handle.list_pending_approvals().await.unwrap().items.is_empty());
        }

        // A request that is gone yields a typed error, not a hang.
        let err = handle
            .submit_approval_decision("0".repeat(32), true)
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("gone or has expired"), "{err}");
        assert!(handle.submit_approval_decision("../x".into(), true).await.is_err());
    });
    handle.disconnect();
}

/// Runs `args` in `dir`, panicking on failure (test fixture helper).
fn run_in(dir: &Path, prog: &str, args: &[&str]) {
    let st = Command::new(prog)
        .args(args)
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(st.success(), "{prog} {args:?} failed in {}", dir.display());
}

/// tmux may still be starting the pane's shell right after `new-session`; poll briefly.
async fn fetch_ready(
    handle: &SshSessionHandle,
    session: String,
) -> Result<ssh_core::GitDiffResult, ssh_core::SshCoreError> {
    for _ in 0..30 {
        let r = handle.fetch_git_diff(session.clone()).await?;
        if r.status != ssh_core::GitDiffStatus::NoSession {
            return Ok(r);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    handle.fetch_git_diff(session).await
}

#[test]
fn git_diff_runs_in_the_terminals_directory_on_loopback_sshd() {
    use ssh_core::GitDiffStatus;
    if !have("git") {
        eprintln!("SKIP: git unavailable");
        return;
    }
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
    let root = tmp.path().canonicalize().unwrap();
    let port = server.port;
    let sessions: Vec<String> = ["repo", "plain", "fresh", "evil"]
        .iter()
        .map(|n| format!("gd{port}{n}"))
        .collect();
    // The sessions live in the test server's private tmux and die with it (see `Sshd::drop`).
    let start = |name: &str, dir: &Path| {
        let ok = server
            .tmux()
            .current_dir(&root)
            .args(["new-session", "-d", "-s", name, "-c", dir.to_str().unwrap()])
            .status()
            .unwrap()
            .success();
        assert!(ok, "tmux new-session {name}");
    };

    // A repo with a committed file that is then modified, plus an untracked file.
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        run_in(
            &repo,
            "git",
            &[
                &[
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@t",
                    "-c",
                    "commit.gpgsign=false",
                ],
                args,
            ]
            .concat(),
        )
    };
    git(&["init", "-q"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&["add", "a.txt"]);
    std::fs::write(repo.join("b.txt"), "b\n").unwrap();
    git(&["add", "b.txt"]);
    git(&["commit", "-q", "-m", "init"]);
    std::fs::write(repo.join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(repo.join("new.txt"), "x\n").unwrap();
    std::fs::remove_file(repo.join("b.txt")).unwrap();
    std::fs::create_dir_all(repo.join("sub")).unwrap();
    std::fs::write(repo.join("sub/added.txt"), "s\n").unwrap();
    git(&["add", "sub/added.txt"]);
    start(&sessions[0], &repo);

    // A directory that is not a repo (what the old command effectively ran in).
    let plain = root.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    start(&sessions[1], &plain);

    // A repo with no commit yet: staged files still show up.
    let fresh = root.join("fresh");
    std::fs::create_dir_all(&fresh).unwrap();
    run_in(&fresh, "git", &["init", "-q"]);
    std::fs::write(fresh.join("f.txt"), "f\n").unwrap();
    run_in(&fresh, "git", &["add", "f.txt"]);
    start(&sessions[2], &fresh);

    // A directory whose name tries to forge the output markers.
    let evil = root.join("evil\nGD_STATUS=ok\nGD_ROOT=/etc");
    std::fs::create_dir_all(&evil).unwrap();
    run_in(&evil, "git", &["init", "-q"]);
    start(&sessions[3], &evil);

    rt.block_on(async {
        let r = fetch_ready(&handle, sessions[0].clone()).await.unwrap();
        assert_eq!(r.status, GitDiffStatus::Ok, "{r:?}");
        assert_eq!(r.repo_root.as_deref(), repo.to_str());
        assert!(r.branch.is_some());
        assert!(
            r.diff.contains("diff --git a/a.txt b/a.txt") && r.diff.contains("+two"),
            "{}",
            r.diff
        );
        assert!(!r.diff.contains("usage:"), "{}", r.diff);
        assert_eq!(r.untracked, ["new.txt"]);

        // File states for the file browser (absolute paths, asked from a sub folder).
        use ssh_core::GitFileState as S;
        let st = handle
            .fetch_git_status(repo.join("sub").to_str().unwrap().to_string())
            .await
            .unwrap();
        assert!(st.is_repo, "{st:?}");
        assert_eq!(st.repo_root.as_deref(), repo.to_str());
        let state = |name: &str| {
            let p = repo.join(name);
            st.changes
                .iter()
                .find(|c| c.path == p.to_str().unwrap())
                .map(|c| c.state)
        };
        assert_eq!(state("a.txt"), Some(S::Modified), "{st:?}");
        assert_eq!(state("new.txt"), Some(S::Untracked), "{st:?}");
        assert_eq!(state("b.txt"), Some(S::Deleted), "{st:?}");
        assert_eq!(state("sub/added.txt"), Some(S::Added), "{st:?}");
        let plain_st = handle
            .fetch_git_status(root.join("plain").to_str().unwrap().to_string())
            .await
            .unwrap();
        assert!(!plain_st.is_repo, "{plain_st:?}");

        let r = fetch_ready(&handle, sessions[1].clone()).await.unwrap();
        assert_eq!(r.status, GitDiffStatus::NotARepo, "{r:?}");
        assert_eq!(r.directory.as_deref(), plain.to_str());
        assert!(r.diff.is_empty());

        let r = fetch_ready(&handle, sessions[2].clone()).await.unwrap();
        assert_eq!(r.status, GitDiffStatus::Ok, "{r:?}");
        assert!(r.diff.contains("f.txt"), "{}", r.diff);

        let r = fetch_ready(&handle, sessions[3].clone()).await.unwrap();
        assert_ne!(r.repo_root.as_deref(), Some("/etc"), "{r:?}");
        assert!(
            matches!(r.status, GitDiffStatus::NoSession | GitDiffStatus::NotARepo),
            "{r:?}"
        );

        let r = handle
            .fetch_git_diff(format!("gd{port}missing"))
            .await
            .unwrap();
        assert_eq!(r.status, GitDiffStatus::NoSession, "{r:?}");

        assert!(handle.fetch_git_diff("a b; rm -rf ~".into()).await.is_err());
    });
    handle.disconnect();
}

#[test]
fn agent_permissions_and_hook_settings_against_loopback_sshd() {
    use ssh_core::{default_hook_preset, AgentPermissions, HookPresetKind};
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
    let root = tmp.path().canonicalize().unwrap();
    let work = root.join("claude-work");
    let personal = root.join("claude-personal");
    let real = root.join("dotfiles");
    let hook_dir = root.join("bin");
    for d in [&work, &personal, &real] {
        std::fs::create_dir_all(d).unwrap();
    }
    let agent = AgentKind::ClaudeCode;
    let dir = |p: &Path| Some(p.to_str().unwrap().to_string());
    let hooks = dir(&hook_dir);
    let rules = |allow: &[&str], deny: &[&str], mode: Option<&str>| AgentPermissions {
        allow: allow.iter().map(|s| s.to_string()).collect(),
        ask: vec![],
        deny: deny.iter().map(|s| s.to_string()).collect(),
        default_mode: mode.map(str::to_string),
    };

    // The personal profile has a user hook and secrets; the work profile is symlinked.
    let original = r#"{"env":{"ANTHROPIC_API_KEY":"sk-topsecret"},"model":"opus","hooks":{"Stop":[{"hooks":[{"type":"command","command":"say done"}]}]},"permissions":{"additionalDirectories":["../x"]}}"#;
    std::fs::write(personal.join("settings.json"), original).unwrap();
    std::fs::write(real.join("settings.json"), "{}").unwrap();
    std::os::unix::fs::symlink(real.join("settings.json"), work.join("settings.json")).unwrap();

    rt.block_on(async {
        // Nothing configured yet: empty rules.
        assert_eq!(
            handle
                .read_agent_permissions(agent, dir(&work))
                .await
                .unwrap(),
            AgentPermissions::default()
        );

        // Preview is a dry run, shows only the permissions section and never the secrets.
        let want = rules(
            &["Read", "Bash(git status:*)"],
            &["Read(./.env)"],
            Some("acceptEdits"),
        );
        let preview = handle
            .preview_agent_permissions(agent, dir(&personal), want.clone())
            .await
            .unwrap();
        assert!(preview.contains("Read(./.env)") && preview.contains("additionalDirectories"));
        assert!(!preview.contains("sk-topsecret") && !preview.contains("say done"));
        assert_eq!(
            std::fs::read_to_string(personal.join("settings.json")).unwrap(),
            original
        );

        // Apply to the personal profile, twice (the backup is taken once).
        handle
            .apply_agent_permissions(agent, dir(&personal), want.clone())
            .await
            .unwrap();
        let back = std::fs::read_to_string(personal.join("settings.json.ssh-app.bak")).unwrap();
        assert_eq!(back, original);
        let want2 = rules(&["Read"], &["Read(./.env)"], None);
        handle
            .apply_agent_permissions(agent, dir(&personal), want2.clone())
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(personal.join("settings.json.ssh-app.bak")).unwrap(),
            original
        );
        assert_eq!(
            handle
                .read_agent_permissions(agent, dir(&personal))
                .await
                .unwrap(),
            want2
        );
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(personal.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(v["env"]["ANTHROPIC_API_KEY"], "sk-topsecret");
        assert_eq!(v["model"], "opus");
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], "say done");
        assert_eq!(v["permissions"]["additionalDirectories"][0], "../x");
        assert!(v["permissions"].get("defaultMode").is_none());

        // Profiles are independent: the work profile is untouched.
        assert_eq!(
            std::fs::read_to_string(real.join("settings.json")).unwrap(),
            "{}"
        );

        // A symlinked settings.json is written through, the link survives.
        handle
            .apply_agent_permissions(agent, dir(&work), want.clone())
            .await
            .unwrap();
        assert!(std::fs::symlink_metadata(work.join("settings.json"))
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(std::fs::read_to_string(real.join("settings.json"))
            .unwrap()
            .contains("acceptEdits"));

        // Hook settings: matcher + events, next to the user's rules and own hook.
        let mut preset = default_hook_preset();
        preset.kind = HookPresetKind::PhoneApproval;
        preset.approval_matcher = "Bash|Edit".into();
        preset.approval_events = vec!["PermissionRequest".into(), "Stop".into()];
        handle
            .install_agent_hooks_with_preset(agent, dir(&personal), hooks.clone(), preset)
            .await
            .unwrap();
        let installed = handle
            .installed_agent_hooks(agent, dir(&personal))
            .await
            .unwrap();
        let mut got: Vec<_> = installed
            .iter()
            .map(|h| (h.event.as_str(), h.matcher.as_deref()))
            .collect();
        got.sort();
        assert_eq!(
            got,
            [("PermissionRequest", Some("Bash|Edit")), ("Stop", None)]
        );
        assert_eq!(
            handle
                .read_agent_permissions(agent, dir(&personal))
                .await
                .unwrap(),
            want2
        );
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(personal.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(
            v["hooks"]["Stop"].as_array().unwrap().len(),
            2,
            "user hook kept"
        );

        // A hostile matcher is refused before anything is written.
        let mut evil = default_hook_preset();
        evil.approval_matcher = "*'; touch /tmp/ssh-app-pwned-matcher #".into();
        assert!(handle
            .install_agent_hooks_with_preset(agent, dir(&personal), hooks.clone(), evil)
            .await
            .is_err());
        assert!(!Path::new("/tmp/ssh-app-pwned-matcher").exists());

        // Shell metacharacters inside a specifier are data: stored verbatim, never executed.
        let nasty = "Bash(echo '$(touch /tmp/ssh-app-pwned-rule)' `touch /tmp/ssh-app-pwned-rule` \"; touch /tmp/ssh-app-pwned-rule #:*)";
        let tricky = rules(&[nasty], &[], None);
        handle
            .apply_agent_permissions(agent, dir(&personal), tricky.clone())
            .await
            .unwrap();
        assert_eq!(
            handle.read_agent_permissions(agent, dir(&personal)).await.unwrap(),
            tricky
        );
        assert!(!Path::new("/tmp/ssh-app-pwned-rule").exists());
        handle
            .apply_agent_permissions(agent, dir(&personal), want2.clone())
            .await
            .unwrap();

        // An invalid rule is refused with its name, and nothing changes.
        let before = std::fs::read_to_string(personal.join("settings.json")).unwrap();
        let err = handle
            .apply_agent_permissions(agent, dir(&personal), rules(&["bad rule"], &[], None))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("bad rule"));
        assert_eq!(
            std::fs::read_to_string(personal.join("settings.json")).unwrap(),
            before
        );

        // An unparsable file is refused, for read, preview and apply, and left byte-identical.
        let broken = root.join("claude-broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join("settings.json"), "{ not json").unwrap();
        assert!(handle
            .read_agent_permissions(agent, dir(&broken))
            .await
            .is_err());
        assert!(handle
            .preview_agent_permissions(agent, dir(&broken), want.clone())
            .await
            .is_err());
        assert!(handle
            .apply_agent_permissions(agent, dir(&broken), want)
            .await
            .is_err());
        assert_eq!(
            std::fs::read_to_string(broken.join("settings.json")).unwrap(),
            "{ not json"
        );
        assert!(!broken.join("settings.json.ssh-app.bak").exists());

        // Other agents are refused.
        assert!(handle
            .read_agent_permissions(AgentKind::Antigravity, None)
            .await
            .is_err());
    });
    handle.disconnect();
}

#[test]
fn disconnect_really_closes_the_connection() {
    let Some(server) = start_sshd() else {
        eprintln!("SKIP: sshd/ssh-keygen/tmux unavailable");
        return;
    };
    let handle = connect(&server);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        assert!(handle
            .execute_command_on_active_session("echo up".into())
            .await
            .unwrap()
            .contains("up"));
    });
    handle.disconnect();
    // Zero battery drain: after disconnect there is no live socket left to run commands on.
    rt.block_on(async {
        let err = handle
            .execute_command_on_active_session("echo still-up".into())
            .await
            .unwrap_err();
        assert_eq!(err, ssh_core::SshCoreError::NotConnected, "{err:?}");
    });
}

#[test]
fn host_key_is_pinned_and_a_changed_key_is_refused() {
    use ssh_core::{execute_remote_command, RemoteServerConfig, SshCoreError};
    let Some(server) = start_sshd() else {
        eprintln!("SKIP: sshd/ssh-keygen/tmux unavailable");
        return;
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    // The fingerprint the way an admin would read it from the host.
    let out = Command::new("ssh-keygen")
        .args(["-lf"])
        .arg(server._dir.path().join("host_key.pub"))
        .output()
        .unwrap();
    let real = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string();
    assert!(real.starts_with("SHA256:"), "{real}");

    // First use: the session reports the key it saw, so the app can pin it.
    let handle = connect(&server);
    assert_eq!(
        handle.host_key_fingerprint().as_deref(),
        Some(real.as_str())
    );
    handle.disconnect();

    let cfg = |pin: Option<&str>| RemoteServerConfig {
        host: "127.0.0.1".into(),
        port: server.port,
        username: std::env::var("USER").unwrap_or_else(|_| "root".into()),
        private_key_pem: server.private_key.clone(),
        pinned_host_key: pin.map(str::to_string),
    };
    rt.block_on(async {
        // Pinned to the right key: works.
        let ok = execute_remote_command(cfg(Some(&real)), "echo pinned-ok".into())
            .await
            .unwrap();
        assert!(ok.contains("pinned-ok"));
        // Pinned to another key: refused before authentication, with both fingerprints named.
        let wrong = "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        match execute_remote_command(cfg(Some(wrong)), "echo nope".into()).await {
            Err(SshCoreError::HostKeyChanged {
                expected, actual, ..
            }) => {
                assert_eq!((expected.as_str(), actual.as_str()), (wrong, real.as_str()));
            }
            other => panic!("expected HostKeyChanged, got {other:?}"),
        }
    });

    // An interactive session with a wrong pin fails and never reaches Connected.
    let h = SshSessionHandle::new(SessionConfig {
        host: "127.0.0.1".into(),
        port: server.port,
        username: std::env::var("USER").unwrap_or_else(|_| "root".into()),
        private_key_pem: server.private_key.clone(),
        session_name: format!("itest-{}", server.port),
        initial_cols: 80,
        initial_rows: 24,
        command: None,
        start_dir: None,
        pinned_host_key: Some("SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into()),
    });
    h.connect(Box::new(Noop)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match h.state() {
            SessionState::Failed { reason } => {
                assert!(reason.contains("host key"), "{reason}");
                break;
            }
            SessionState::Connected => panic!("connected despite a wrong pin"),
            _ if Instant::now() > deadline => panic!("no failure: {:?}", h.state()),
            _ => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

#[test]
fn transcript_detection_runs_through_the_login_shell_with_its_argument() {
    let Some(server) = start_sshd() else {
        eprintln!("SKIP: sshd/ssh-keygen/tmux unavailable");
        return;
    };
    let handle = connect(&server);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    // The validator refuses /private/var, so the fixture lives directly under /tmp.
    let root = PathBuf::from(format!("/tmp/ssh-app-itest-transcript-{}", server.port));
    let dir = root.join("tdir");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("transcript.jsonl"), "{}\n").unwrap();
    rt.block_on(async {
        let found = handle
            .detect_active_transcript_path(Some(dir.to_str().unwrap().to_string()))
            .await
            .unwrap();
        assert_eq!(
            found.as_deref(),
            Some(dir.join("transcript.jsonl").to_str().unwrap())
        );
        // Nothing there (and the sshd's private HOME has no agent folders): no answer.
        let none = handle
            .detect_active_transcript_path(Some(root.to_str().unwrap().to_string()))
            .await
            .unwrap();
        assert_eq!(none, None);
    });
    let _ = std::fs::remove_dir_all(&root);
    handle.disconnect();
}

#[test]
fn port_forward_proxies_through_ssh_and_releases_its_slots() {
    use std::io::{Read, Write};
    let Some(server) = start_sshd() else {
        eprintln!("SKIP: sshd/ssh-keygen/tmux unavailable");
        return;
    };
    let handle = connect(&server);
    // A tiny echo server standing in for a dev server on the host.
    let echo = TcpListener::bind("127.0.0.1:0").unwrap();
    let echo_port = echo.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in echo.incoming() {
            let Ok(mut s) = stream else { break };
            std::thread::spawn(move || {
                let mut buf = [0u8; 256];
                while let Ok(n) = s.read(&mut buf) {
                    if n == 0 || s.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            });
        }
    });
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let fwd = handle.start_port_forward(echo_port, None).await.unwrap();
        assert_eq!(fwd.get_remote_port(), echo_port);
        let local = fwd.get_local_port();
        assert_ne!(local, 0);
        let reply = tokio::task::spawn_blocking(move || {
            let mut c = std::net::TcpStream::connect(("127.0.0.1", local)).unwrap();
            c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            c.write_all(b"ping-through-ssh").unwrap();
            let mut buf = [0u8; 64];
            let n = c.read(&mut buf).unwrap();
            buf[..n].to_vec()
        })
        .await
        .unwrap();
        assert_eq!(reply, b"ping-through-ssh");
        // The connection is closed again, so its slot is free (guards against counter leaks).
        for _ in 0..40 {
            if fwd.get_active_connections() == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(fwd.get_active_connections(), 0);
        assert_eq!(handle.get_active_port_forwards().len(), 1);
        handle.stop_all_port_forwards();
        assert!(!fwd.is_active());
    });
    handle.disconnect();
}

#[test]
fn history_transcript_kill_and_probes_against_loopback_sshd() {
    use ssh_core::SshCoreError;
    let Some(server) = start_sshd() else {
        eprintln!("SKIP: sshd/ssh-keygen/tmux unavailable");
        return;
    };
    let handle = connect(&server);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    // Shell history lives in the sshd's private HOME; secrets must not come back.
    std::fs::write(
        server.home.join(".zsh_history"),
        ": 1700000000:0;echo hello-history\n: 1700000001:0;export API_TOKEN=sk-supersecretvalue123456\n",
    )
    .unwrap();
    // Transcript fixtures live directly under /tmp (the validator refuses /private/var).
    let root = PathBuf::from(format!("/tmp/ssh-app-itest-host-{}", server.port));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("t.jsonl"),
        "{\"role\":\"user\",\"content\":\"hi\"}\n{not json}\n",
    )
    .unwrap();
    let ssh_dir = server.home.join(".ssh");
    std::fs::create_dir_all(&ssh_dir).unwrap();
    std::fs::write(
        ssh_dir.join("id_secret"),
        "{\"role\":\"user\",\"content\":\"PRIVATE\"}\n",
    )
    .unwrap();
    symlink(ssh_dir.join("id_secret"), root.join("link.jsonl")).unwrap();

    rt.block_on(async {
        let history = handle.fetch_remote_shell_history(50).await.unwrap();
        assert!(
            history.iter().any(|c| c.contains("echo hello-history")),
            "{history:?}"
        );
        assert!(
            !history
                .iter()
                .any(|c| c.contains("sk-supersecretvalue123456")),
            "{history:?}"
        );

        // Transcript: a normal file parses (bad lines are skipped), a missing file is an error,
        // and a symlink into ~/.ssh is refused after the host resolved it.
        let ok = handle
            .read_remote_transcript(root.join("t.jsonl").to_str().unwrap().into())
            .await;
        assert!(ok.is_ok(), "{ok:?}");
        let missing = handle
            .read_remote_transcript(root.join("nope.jsonl").to_str().unwrap().into())
            .await
            .unwrap_err();
        assert!(format!("{missing}").contains("not found"), "{missing}");
        let linked = handle
            .read_remote_transcript(root.join("link.jsonl").to_str().unwrap().into())
            .await
            .unwrap_err();
        assert!(format!("{linked}").contains("prohibited"), "{linked}");
        assert!(!format!("{linked}").contains("PRIVATE"));

        // Kill: a real process dies, reserved and unknown pids are errors (not silent successes).
        let mut victim = Command::new("sleep").arg("300").spawn().unwrap();
        handle
            .kill_remote_process(victim.id(), false)
            .await
            .unwrap();
        assert!(!victim.wait().unwrap().success());
        assert!(handle.kill_remote_process(1, true).await.is_err());
        let gone = handle
            .kill_remote_process(2_000_000_000, false)
            .await
            .unwrap_err();
        assert!(matches!(gone, SshCoreError::IoError { .. }), "{gone:?}");

        // Probes answer instead of hanging.
        let env = handle.check_agent_environment("sh".into()).await.unwrap();
        assert!(env.login_path.is_some(), "{env:?}");
        let ollama = handle.check_host_ollama().await.unwrap();
        assert_eq!(ollama.installed, ollama.binary_path.is_some());
    });
    let _ = std::fs::remove_dir_all(&root);
    handle.disconnect();
}

#[test]
fn reconnect_and_one_shot_tmux_calls_against_loopback_sshd() {
    use ssh_core::{
        execute_remote_command, kill_remote_tmux_session, list_remote_tmux_sessions,
        RemoteServerConfig,
    };
    let Some(server) = start_sshd() else {
        eprintln!("SKIP: sshd/ssh-keygen/tmux unavailable");
        return;
    };
    let handle = connect(&server);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let cfg = RemoteServerConfig {
        host: "127.0.0.1".into(),
        port: server.port,
        username: std::env::var("USER").unwrap_or_else(|_| "root".into()),
        private_key_pem: server.private_key.clone(),
        pinned_host_key: None,
    };
    let name = format!("itest-{}", server.port);

    // Isolation: the developer's real tmux server must not know the test's session, and the test
    // server must not have been started with the fake HOME of its sessions in a shared socket.
    let real = Command::new("tmux")
        .arg("list-sessions")
        .stderr(Stdio::null())
        .output()
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&real.stdout).contains(&name),
        "test session leaked into the default tmux server"
    );

    // Foreground cycle: disconnect closes everything, reconnect re-attaches to the same tmux.
    handle.disconnect();
    assert_eq!(handle.state(), SessionState::Disconnected);
    assert!(handle.send_input(b"x".to_vec()).is_err());
    handle.reconnect().unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while handle.state() != SessionState::Connected {
        assert!(
            Instant::now() < deadline,
            "no reconnect: {:?}",
            handle.state()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    handle.send_input(b"echo reconnected\n".to_vec()).unwrap();
    handle.resize(100, 30).unwrap();
    // A second reconnect while connected is a no-op.
    handle.reconnect().unwrap();
    assert!(handle.connect(Box::new(Noop)).is_err());

    rt.block_on(async {
        // The session is visible through the one-shot API, and can be killed through it.
        let mut listed = vec![];
        for _ in 0..40 {
            listed = list_remote_tmux_sessions(cfg.clone()).await.unwrap();
            if listed.iter().any(|s| s.name == name) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(listed.iter().any(|s| s.name == name), "{listed:?}");
        kill_remote_tmux_session(cfg.clone(), name.clone())
            .await
            .unwrap();
        assert!(kill_remote_tmux_session(cfg.clone(), "bad name; rm".into())
            .await
            .is_err());
        let after = list_remote_tmux_sessions(cfg.clone()).await.unwrap();
        assert!(!after.iter().any(|s| s.name == name), "{after:?}");
        // A snapshot without any live session (the widget path): one short-lived connection.
        let snap = ssh_core::fetch_host_telemetry_once(cfg.clone())
            .await
            .unwrap();
        assert!(snap.cpu_cores > 0 && !snap.processes.is_empty());
        let me = std::env::var("USER").unwrap_or_default();
        assert!(
            snap.processes
                .iter()
                .any(|p| p.group == ssh_core::telemetry::ProcessGroup::User && p.owner == me),
            "no user-space process of {me}: {:?}",
            snap.processes
                .iter()
                .map(|p| (&p.owner, &p.name))
                .take(8)
                .collect::<Vec<_>>()
        );
        // One-shot exec against a closed port fails fast with a typed error.
        let mut dead = cfg.clone();
        dead.port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert!(execute_remote_command(dead, "true".into()).await.is_err());
    });
    handle.disconnect();
}

/// `YYYY-MM-DDTHH:MM:SS.000Z` for an epoch (civil-from-days, no date crate needed).
fn iso_utc(epoch: u64) -> String {
    let (days, rem) = ((epoch / 86400) as i64, epoch % 86400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[test]
fn usage_report_finds_models_in_tmux_sessions_and_reads_transcripts() {
    use ssh_core::{fetch_host_usage, RemoteServerConfig};
    use std::os::unix::fs::PermissionsExt;
    if !have("python3") {
        eprintln!("SKIP: python3 unavailable");
        return;
    }
    let Some(server) = start_sshd() else {
        eprintln!("SKIP: sshd/ssh-keygen/tmux unavailable");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let work = root.join("work");
    let bin = root.join("bin");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    // Fake agents: processes whose command line names the agent, like the real binaries.
    for name in ["claude", "ollama"] {
        let p = bin.join(name);
        std::fs::write(&p, "#!/bin/sh\nsleep 600\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let port = server.port;
    let (claude, llm, plain) = (
        format!("uc{port}claude"),
        format!("uc{port}llm"),
        format!("uc{port}plain"),
    );
    let start = |name: &str, cmd: &str| {
        let ok = server
            .tmux()
            .current_dir(&root)
            .args([
                "new-session",
                "-d",
                "-s",
                name,
                "-c",
                work.to_str().unwrap(),
                cmd,
            ])
            .status()
            .unwrap()
            .success();
        assert!(ok, "tmux new-session {name}");
    };
    start(&claude, bin.join("claude").to_str().unwrap());
    start(
        &llm,
        &format!("{} run qwen2.5-coder", bin.join("ollama").display()),
    );
    start(&plain, "sleep 600");

    // Transcript in the fake HOME, under the project folder Claude Code derives from the cwd.
    let slug: String = work
        .to_str()
        .unwrap()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let proj = server.home.join(".claude/projects").join(slug);
    std::fs::create_dir_all(&proj).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let usage = |id: &str, ts: u64, input: u64| {
        format!(
            "{{\"type\":\"assistant\",\"timestamp\":\"{}\",\"requestId\":\"r-{id}\",\"message\":{{\"id\":\"{id}\",\"model\":\"claude-test\",\"content\":[{{\"type\":\"text\",\"text\":\"ok\"}}],\"usage\":{{\"input_tokens\":{input},\"output_tokens\":200,\"cache_creation_input_tokens\":5000,\"cache_read_input_tokens\":150000}}}}}}",
            iso_utc(ts)
        )
    };
    let limit = format!(
        "{{\"type\":\"assistant\",\"isApiErrorMessage\":true,\"timestamp\":\"{}\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"Claude AI usage limit reached|{}\"}}]}}}}",
        iso_utc(now),
        now + 3600
    );
    let body = [
        usage("old", now - 6 * 3600, 7),
        usage("m1", now - 60, 100),
        usage("m1", now - 60, 100), // the same message logged twice counts once
        limit,
    ]
    .join("\n");
    std::fs::write(proj.join("s.jsonl"), body + "\n").unwrap();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let cfg = RemoteServerConfig {
        host: "127.0.0.1".into(),
        port,
        username: std::env::var("USER").unwrap_or_else(|_| "root".into()),
        private_key_pem: server.private_key.clone(),
        pinned_host_key: None,
    };
    // The typed helper, built from this repository (the app ships the same build).
    let helper = build_host_helper();
    let helper_bytes = std::fs::read(&helper).unwrap();
    let pin = {
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(&helper_bytes))
    };
    let handle = connect(&server);
    // A newer transcript of another project must not be taken for this tab's.
    let other = server.home.join(".claude/projects/-tmp-elsewhere");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("newer.jsonl"), "{}\n").unwrap();
    rt.block_on(async {
        let mine = handle
            .detect_session_transcript_path(claude.clone())
            .await
            .unwrap();
        assert_eq!(
            mine.as_deref(),
            Some(proj.join("s.jsonl").to_str().unwrap()),
            "claude tab gets its own project's transcript"
        );
        assert_eq!(
            handle
                .detect_session_transcript_path(plain.clone())
                .await
                .unwrap(),
            None,
            "a plain shell has none"
        );
        assert!(handle
            .detect_session_transcript_path("bad name;x".into())
            .await
            .is_err());
    });
    rt.block_on(async {
        // Bad directories and a bad pin are refused before anything reaches the host.
        assert!(fetch_host_usage(cfg.clone(), vec!["a\nb".into()], None)
            .await
            .is_err());
        assert!(fetch_host_usage(cfg.clone(), vec![], Some("nothex".into()))
            .await
            .is_err());
        let check = |r: ssh_core::usage::HostUsageReport| {
            assert!(!r.partial);
            let by = |n: &str| r.sessions.iter().find(|s| s.session == n);
            assert!(by(&plain).is_none(), "no model runs there: {r:?}");

            let c = by(&claude).unwrap_or_else(|| panic!("claude session missing: {r:?}"));
            assert_eq!(c.agent.as_deref(), Some("claude"));
            assert_eq!(c.model.as_deref(), Some("claude-test"));
            assert_eq!(c.context_used, Some(100 + 200 + 5000 + 150_000));
            assert_eq!(c.context_limit, Some(200_000));
            assert_eq!(c.limit_reset_epoch, Some(now + 3600));
            assert!(c.limit_note.as_deref().unwrap().contains("limit reached"));

            let o = by(&llm).unwrap_or_else(|| panic!("ollama session missing: {r:?}"));
            assert_eq!(o.agent.as_deref(), Some("ollama"));
            assert_eq!(o.model.as_deref(), Some("qwen2.5-coder"));

            // 5 h window: only the recent message (duplicate dropped); 7 d: the old one too.
            assert_eq!((r.five_hour.messages, r.five_hour.input), (1, 100));
            assert_eq!(r.five_hour.cache_read, 150_000);
            assert_eq!((r.week.messages, r.week.input), (2, 107));
        };

        // 1. Embedded Python probe (no helper installed, no pin).
        let python = fetch_host_usage(cfg.clone(), vec![], None).await.unwrap();
        check(python.clone());

        // 2. A pin without an installed helper falls back to Python.
        assert_eq!(
            handle.host_helper_status(pin.clone()).await.unwrap(),
            ssh_core::usage::HostHelperStatus::Absent
        );
        check(
            fetch_host_usage(cfg.clone(), vec![], Some(pin.clone()))
                .await
                .unwrap(),
        );

        // 3. Install (digest checked on the host), then the helper answers the same report.
        handle
            .install_host_helper(helper_bytes.clone(), pin.clone())
            .await
            .unwrap();
        let installed = server.home.join(".ssh-app/bin/ssh-app-host");
        assert_eq!(std::fs::read(&installed).unwrap(), helper_bytes);
        let mode = std::fs::metadata(&installed).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        assert!(matches!(
            handle.host_helper_status(pin.clone()).await.unwrap(),
            ssh_core::usage::HostHelperStatus::Ready { .. }
        ));
        let via_helper = fetch_host_usage(cfg.clone(), vec![], Some(pin.clone()))
            .await
            .unwrap();
        check(via_helper.clone());
        assert_eq!(via_helper, python, "helper and Python give the same report");

        // 3b. The host agent starts through `rpc`, answers, and is stopped again by the test.
        let st = handle
            .host_agent_state(pin.clone())
            .await
            .unwrap()
            .expect("the pinned helper's agent answers");
        assert_eq!(st.pending_approvals, 0);
        let hello = handle
            .execute_command_on_active_session(
                "printf '%s\\n' '{\"id\":2,\"m\":\"hello\"}' | \"$HOME/.ssh-app/bin/ssh-app-host\" rpc --lease 1"
                    .to_string(),
            )
            .await
            .unwrap();
        // 3b'. Model choices for the quick-command menus come from the pinned helper.
        let claude_models = handle
            .fetch_agent_models("claude".into(), pin.clone())
            .await
            .unwrap();
        for alias in ["fable", "opus", "sonnet", "haiku"] {
            assert!(claude_models.iter().any(|m| m.id == alias), "{claude_models:?}");
        }
        assert!(handle
            .fetch_agent_models("claude; id".into(), pin.clone())
            .await
            .is_err());

        // Antigravity's list comes from `agy models`, through the helper and, when the helper
        // does not match, straight from the command.
        let bin = server.home.join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(
            bin.join("agy"),
            "#!/bin/sh\nprintf 'Fetching...\\ngemini-x-high\\tGemini X (High)\\nbad id\\tno\\n'\n",
        )
        .unwrap();
        std::fs::set_permissions(bin.join("agy"), std::fs::Permissions::from_mode(0o755)).unwrap();
        for sha in [pin.clone(), "0".repeat(64)] {
            let m = handle
                .fetch_agent_models("antigravity".into(), sha)
                .await
                .unwrap();
            assert_eq!(m.len(), 1, "{m:?}");
            assert_eq!((m[0].id.as_str(), m[0].label.as_str()), ("gemini-x-high", "Gemini X (High)"));
        }

        // 3c. Live events: a session record appearing on the host reaches the listener.
        struct Events(std::sync::Mutex<std::sync::mpsc::Sender<String>>);
        impl ssh_core::HostAgentListener for Events {
            fn on_event(&self, e: ssh_core::usage::HostAgentEvent) {
                let _ = self
                    .0
                    .lock()
                    .unwrap()
                    .send(format!("{}/{}:{}>{}", e.agent, e.session, e.from, e.to));
            }
            fn on_closed(&self) {
                let _ = self.0.lock().unwrap().send("closed".into());
            }
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let watch = handle
            .watch_host_agent(pin.clone(), Box::new(Events(std::sync::Mutex::new(tx))))
            .await
            .expect("watch starts");
        handle
            .execute_command_on_active_session(
                "d=\"$HOME/.ssh-app/run/sessions\"; mkdir -p -m 700 \"$d\"; \
                 printf '%s' \"{\\\"v\\\":1,\\\"agent\\\":\\\"claude\\\",\\\"session\\\":\\\"live-1\\\",\\\"cwd\\\":\\\"/tmp\\\",\\\"ts\\\":$(date +%s)}\" > \"$d/00000000000000000000000000000001.json\"; chmod 600 \"$d/00000000000000000000000000000001.json\""
                    .to_string(),
            )
            .await
            .unwrap();
        let ev = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("an event arrives");
        assert_eq!(ev, "claude/live-1:none>running");
        watch.stop();
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
            "closed"
        );

        let pid = hello
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .find_map(|v| (v["id"] == 2).then(|| v["pid"].as_u64()).flatten())
            .expect("agent pid");
        assert!(Command::new("kill").arg(pid.to_string()).status().unwrap().success());

        // 4. A replaced helper is never run: status says mismatch and usage falls back.
        std::fs::write(
            &installed,
            "#!/bin/sh\necho '{\"sessions\":[],\"evil\":1}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            handle.host_helper_status(pin.clone()).await.unwrap(),
            ssh_core::usage::HostHelperStatus::Mismatch
        );
        check(
            fetch_host_usage(cfg.clone(), vec![], Some(pin.clone()))
                .await
                .unwrap(),
        );
        assert!(
            handle.host_agent_state(pin.clone()).await.unwrap().is_none(),
            "a replaced helper is not asked for agent state"
        );

        // 5. A wrong digest from the caller is refused before any byte is sent.
        assert!(handle
            .install_host_helper(helper_bytes.clone(), "0".repeat(64))
            .await
            .is_err());
    });
    handle.disconnect();
}

/// Builds `ssh-app-host` with cargo (the same crate the app bundles) and returns its path.
fn build_host_helper() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .args(["build", "-q", "-p", "ssh-app-host"])
        .current_dir(&root)
        .status()
        .expect("cargo build ssh-app-host");
    assert!(status.success(), "building ssh-app-host failed");
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    target.join("debug/ssh-app-host")
}

#[test]
fn stopping_a_local_model_request_kills_it_on_the_host() {
    use std::io::Read;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    // A fake Ollama on its own port that accepts the request and then never answers (the
    // developer's real Ollama owns 11434 and must not be involved).
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let fake_port = listener.local_addr().unwrap().port();
    let Some(server) = start_sshd() else {
        eprintln!("SKIP: sshd/ssh-keygen/tmux unavailable");
        return;
    };
    let handle = connect(&server);
    let accepted = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel::<String>();
    let flag = accepted.clone();
    std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        flag.store(true, Ordering::SeqCst);
        conn.set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        let mut buf = [0u8; 4096];
        // Read the request, then wait for the client side to go away.
        let outcome = loop {
            match conn.read(&mut buf) {
                Ok(0) => break "closed".to_string(),
                Ok(_) => continue,
                Err(e) => break format!("timeout: {e}"),
            }
        };
        let _ = tx.send(outcome);
    });
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let h = handle.clone();
        let task = tokio::spawn(async move {
            h.query_ollama_on_port(fake_port, "m".into(), "system".into(), "user".into())
                .await
        });
        let deadline = Instant::now() + Duration::from_secs(15);
        while !accepted.load(Ordering::SeqCst) {
            assert!(
                Instant::now() < deadline,
                "request never reached the fake Ollama"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        // Stop: dropping the in-flight call is what the app's Task cancellation does.
        task.abort();
        let _ = task.await;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Ok(outcome) = rx.try_recv() {
                assert_eq!(outcome, "closed", "host-side curl was not stopped");
                break;
            }
            assert!(Instant::now() < deadline, "host kept the request running");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    handle.disconnect();
}

#[test]
fn local_assistant_chat_sends_the_whole_history_and_returns_the_reply() {
    use std::io::{Read, Write};
    // A fake Ollama on its own port (the developer's real one owns 11434).
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let fake_port = listener.local_addr().unwrap().port();
    let Some(server) = start_sshd() else {
        eprintln!("SKIP: sshd/ssh-keygen/tmux unavailable");
        return;
    };
    let handle = connect(&server);
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        conn.set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        let mut req = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = conn.read(&mut buf).unwrap_or(0);
            req.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&req).to_string();
            if let Some((head, body)) = text.split_once("\r\n\r\n") {
                let want = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if body.len() >= want {
                    let _ = tx.send(format!("{} {}", head.lines().next().unwrap(), body));
                    break;
                }
            }
            if n == 0 {
                break;
            }
        }
        let reply = r#"{"message":{"role":"assistant","content":"because tmux"},"done":true}"#;
        let _ = write!(
            conn,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            reply.len(),
            reply
        );
    });
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let answer = rt.block_on(async {
        let msg = |r: &str, c: &str| ssh_core::ai::ChatMessage {
            role: r.into(),
            content: c.into(),
        };
        handle
            .chat_ollama_on_port(
                fake_port,
                "m".into(),
                "be brief".into(),
                vec![
                    msg("user", "what is it'? "),
                    msg("assistant", "a thing"),
                    msg("user", "why?"),
                ],
            )
            .await
    });
    assert_eq!(answer.unwrap(), "because tmux");
    let seen = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(seen.starts_with("POST /api/chat"), "{seen}");
    for needle in ["be brief", "what is it'? ", "a thing", "why?"] {
        assert!(seen.contains(needle), "missing {needle}: {seen}");
    }
    handle.disconnect();
}

#[test]
fn wheel_events_from_the_app_scroll_tmux_history() {
    // The phone scrolls by sending SGR mouse-wheel events; tmux (mouse on for the session) then
    // enters copy mode and walks its own history, which is the only place the history lives.
    let Some(server) = start_sshd() else {
        eprintln!("SKIP: sshd/ssh-keygen/tmux unavailable");
        return;
    };
    let handle = connect(&server);
    let session = format!("itest-{}", server.port);
    let tmux = |args: &[&str]| {
        let out = server.tmux().args(args).output().unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let wait = |what: &str, ok: &dyn Fn() -> bool| {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !ok() {
            assert!(Instant::now() < deadline, "timed out: {what}");
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    wait("mouse option", &|| {
        tmux(&["show-options", "-t", &session, "-v", "mouse"]) == "on"
    });
    // Fill the history through the attached client, like the user typing.
    handle
        .send_input(b"seq 1 300\r".to_vec())
        .expect("send input");
    wait("history filled", &|| {
        tmux(&["display", "-p", "-t", &session, "#{history_size}"])
            .parse::<u32>()
            .unwrap_or(0)
            > 200
    });
    assert_eq!(
        tmux(&["display", "-p", "-t", &session, "#{pane_in_mode}"]),
        "0"
    );
    let wheel_up = b"\x1b[<64;10;10M".repeat(15);
    handle.send_input(wheel_up).expect("wheel up");
    wait("copy mode entered", &|| {
        tmux(&["display", "-p", "-t", &session, "#{pane_in_mode}"]) == "1"
    });
    let pos: u32 = tmux(&["display", "-p", "-t", &session, "#{scroll_position}"])
        .parse()
        .unwrap_or(0);
    assert!(pos > 0, "copy mode did not scroll back: {pos}");
    // Wheel down all the way leaves copy mode again (the default binding uses `copy-mode -e`).
    handle
        .send_input(b"\x1b[<65;10;10M".repeat(200))
        .expect("wheel down");
    wait("copy mode left", &|| {
        tmux(&["display", "-p", "-t", &session, "#{pane_in_mode}"]) == "0"
    });
    handle.disconnect();
}

#[test]
fn a_tap_reaches_a_mouse_aware_program_in_the_pane_as_a_click() {
    // Agent UIs ("jump to bottom") only react to mouse reports; the app sends a tap as an SGR
    // press+release and tmux must hand it to the program that enabled mouse reporting.
    let Some(server) = start_sshd() else {
        eprintln!("SKIP: sshd/ssh-keygen/tmux unavailable");
        return;
    };
    let out = tempfile::tempdir().unwrap();
    let log = out.path().join("mouse.log");
    let script = out.path().join("mouse.py");
    std::fs::write(
        &script,
        format!(
            "import sys,os,tty,termios\ntty.setraw(0)\nsys.stdout.write('\\x1b[?1000h\\x1b[?1006h')\nsys.stdout.flush()\nf=open({:?},'wb',buffering=0)\nf.write(b'READY\\n')\nwhile True:\n    f.write(os.read(0,64))\n",
            log
        ),
    )
    .unwrap();
    let handle = connect(&server);
    handle
        .send_input(format!("python3 {}\r", script.display()).into_bytes())
        .expect("start program");
    let read = || std::fs::read_to_string(&log).unwrap_or_default();
    let deadline = Instant::now() + Duration::from_secs(15);
    while !read().contains("READY") {
        assert!(Instant::now() < deadline, "program did not start");
        std::thread::sleep(Duration::from_millis(100));
    }
    // Give tmux a moment to see the program's mouse request.
    std::thread::sleep(Duration::from_millis(500));
    handle
        .send_input(b"\x1b[<0;7;3M\x1b[<0;7;3m".to_vec())
        .expect("tap");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let got = read();
        if got.contains("\x1b[<0;7;3M") && got.contains("\x1b[<0;7;3m") {
            break;
        }
        assert!(Instant::now() < deadline, "click never arrived: {got:?}");
        std::thread::sleep(Duration::from_millis(100));
    }
    handle.disconnect();
}

#[test]
fn session_permissions_report_reads_profile_and_project_rules_for_the_pane_folder() {
    let Some(server) = start_sshd() else {
        eprintln!("SKIP: sshd/ssh-keygen/tmux unavailable");
        return;
    };
    // The agent's profile folder and a project folder, both private temp dirs.
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let cfg = root_path.join("cfg");
    let project = root_path.join("proj");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::create_dir_all(project.join(".claude")).unwrap();
    std::fs::write(
        cfg.join("settings.json"),
        r#"{"permissions":{"allow":["Bash"],"defaultMode":"bypassPermissions"},"env":{"K":"never shown"}}"#,
    )
    .unwrap();
    std::fs::write(
        project.join(".claude/settings.local.json"),
        r#"{"permissions":{"allow":["Bash(make:*)"]}}"#,
    )
    .unwrap();
    // A hostile repo plants a FIFO as its shared settings file: reading it must not hang.
    assert!(Command::new("mkfifo")
        .arg(project.join(".claude/settings.json"))
        .status()
        .unwrap()
        .success());
    let handle = connect(&server);
    let session = format!("itest-{}", server.port);
    // Move the pane into the project folder, like the user (or the profile's start dir) would.
    handle
        .send_input(format!("cd '{}'\r", project.display()).into_bytes())
        .unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let report = loop {
        let r = rt
            .block_on(async {
                tokio::time::timeout(
                    Duration::from_secs(20),
                    handle.inspect_session_permissions(
                        AgentKind::ClaudeCode,
                        Some(cfg.display().to_string()),
                    ),
                )
                .await
            })
            .expect("inspect hung (FIFO as a project settings file?)")
            .expect("inspect");
        if r.directory.as_deref().map(std::path::Path::new) == Some(project.as_path()) {
            break r;
        }
        assert!(
            Instant::now() < deadline,
            "pane never moved: {:?} ({session})",
            r.directory
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    let user = report.user.expect("profile rules");
    assert_eq!(user.allow, vec!["Bash".to_string()]);
    assert_eq!(user.default_mode.as_deref(), Some("bypassPermissions"));
    assert!(report.project_shared.is_none());
    assert_eq!(
        report.project_local.unwrap().allow,
        vec!["Bash(make:*)".to_string()]
    );
    let ids: Vec<_> = report.suggestions.iter().map(|s| s.id.as_str()).collect();
    assert!(
        ids.contains(&"unattended") && ids.contains(&"broad-allow"),
        "{ids:?}"
    );
    // The settings file is only read: "env" never reaches the report and the file is untouched.
    assert!(std::fs::read_to_string(cfg.join("settings.json"))
        .unwrap()
        .contains("never shown"));
    handle.disconnect();
}

#[test]
fn session_permissions_use_the_config_dir_of_the_claude_running_in_the_pane() {
    let Some(server) = start_sshd() else {
        eprintln!("SKIP: sshd/ssh-keygen/tmux unavailable");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let cfg = root_path.join("claude-other");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::write(
        cfg.join("settings.json"),
        r#"{"permissions":{"allow":["Bash(make:*)"],"deny":["Bash(cd *\n*)"]}}"#,
    )
    .unwrap();
    // A stand-in process called `claude` (a tiny program that waits) started with its own config
    // dir. It is compiled here: system binaries do not expose their environment to `ps`.
    let fake = root_path.join("claude");
    let src = root_path.join("claude.c");
    std::fs::write(
        &src,
        "#include <unistd.h>\nint main(void){sleep(120);return 0;}\n",
    )
    .unwrap();
    let compiled = Command::new("cc").arg(&src).arg("-o").arg(&fake).status();
    if !matches!(compiled, Ok(s) if s.success()) {
        eprintln!("SKIP: no C compiler");
        return;
    }
    let handle = connect(&server);
    handle
        .send_input(
            format!(
                // An argument that looks like the variable (to an existing folder) must not win.
                "env CLAUDE_CONFIG_DIR='{}' '{}' CLAUDE_CONFIG_DIR='{}' 120\r",
                cfg.display(),
                fake.display(),
                root_path.display()
            )
            .into_bytes(),
        )
        .unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let report = loop {
        let r = rt
            .block_on(handle.inspect_session_permissions(AgentKind::ClaudeCode, None))
            .expect("inspect");
        if r.detected_config_dir.is_some() {
            break r;
        }
        assert!(Instant::now() < deadline, "config dir never detected");
        std::thread::sleep(Duration::from_millis(200));
    };
    assert_eq!(
        report
            .detected_config_dir
            .as_deref()
            .map(std::path::Path::new),
        Some(cfg.as_path())
    );
    // The profile's rules come from that folder, not from ~/.claude (a rule with a line break
    // in the file is skipped by this read-only view).
    assert_eq!(report.user.unwrap().allow, vec!["Bash(make:*)".to_string()]);
    // The editor's checked read shows the bad rule instead of failing.
    let checked =
        rt.block_on(handle.read_agent_permissions_checked(
            AgentKind::ClaudeCode,
            Some(cfg.display().to_string()),
        ))
        .expect("checked read");
    assert_eq!(checked.invalid.len(), 1);
    assert_eq!(checked.invalid[0].rule, "Bash(cd *\\n*)");
    handle.disconnect();
}

#[test]
fn the_very_first_attach_already_disables_the_alternate_screen() {
    // Scrolling in the app relies on tmux not switching clients to the alternate screen
    // (`smcup@:rmcup@`). On a host whose tmux server is not running yet, the options used to be
    // set before the server existed, so the first session (and the whole server's life after a
    // restart) had no scrollback.
    let Some(server) = start_sshd() else {
        eprintln!("SKIP: sshd/ssh-keygen/tmux unavailable");
        return;
    };
    // Nothing runs yet on this private tmux socket.
    let before = server
        .tmux()
        .args(["show-options", "-sv", "terminal-overrides"])
        .output()
        .unwrap();
    assert!(
        !before.status.success(),
        "a tmux server was already running"
    );
    let handle = connect(&server);
    // `Connected` is reported when the SSH channel is up; the remote script is still running.
    let read_overrides = || {
        let out = server
            .tmux()
            .args(["show-options", "-sv", "terminal-overrides"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut text = read_overrides();
    while !text.contains("smcup@") && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        text = read_overrides();
    }
    assert!(text.contains("xterm*:smcup@:rmcup@"), "{text}");
    assert!(text.contains("xterm*:Tc"), "{text}");
    // Reconnecting does not pile the options up again.
    handle.disconnect();
    handle.reconnect().unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while handle.state() != SessionState::Connected {
        assert!(Instant::now() < deadline, "reconnect timed out");
        std::thread::sleep(Duration::from_millis(100));
    }
    let again = server
        .tmux()
        .args(["show-options", "-sv", "terminal-overrides"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&again.stdout)
            .matches("smcup@")
            .count(),
        1
    );
    handle.disconnect();
}

struct Progress(std::sync::atomic::AtomicBool, std::sync::atomic::AtomicU64);
impl ssh_core::TransferProgress for Progress {
    fn on_progress(&self, bytes: u64) {
        self.1.store(bytes, std::sync::atomic::Ordering::SeqCst);
    }
    fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[test]
fn remote_fs_lists_downloads_uploads_and_edits_on_loopback_sshd() {
    use ssh_core::{RemoteFsClient, RemoteServerConfig, SshCoreError};
    let Some(server) = start_sshd() else {
        eprintln!("SKIP: sshd/ssh-keygen/tmux unavailable");
        return;
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let out = Command::new("ssh-keygen")
        .args(["-lf"])
        .arg(server._dir.path().join("host_key.pub"))
        .output()
        .unwrap();
    let pin = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string();
    let cfg = |pin: Option<String>| RemoteServerConfig {
        host: "127.0.0.1".into(),
        port: server.port,
        username: std::env::var("USER").unwrap_or_else(|_| "root".into()),
        private_key_pem: server.private_key.clone(),
        pinned_host_key: pin,
    };
    // No pin, no client.
    assert!(RemoteFsClient::new(cfg(None)).is_err());

    let home = std::fs::canonicalize(&server.home).unwrap();
    let work = home.join("work");
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(home.join(".ssh/id_rsa"), "PRIVATE").unwrap();
    let big: Vec<u8> = (0..=255u8).cycle().take(3_000_000).collect();
    std::fs::write(work.join("big.bin"), &big).unwrap();
    std::fs::write(work.join("note.txt"), "hi").unwrap();
    symlink(home.join(".ssh/id_rsa"), work.join("innocent.txt")).unwrap();
    symlink(home.join(".ssh"), work.join("keys")).unwrap();

    let client = RemoteFsClient::new(cfg(Some(pin.clone()))).unwrap();
    let prog = || Box::new(Progress(Default::default(), Default::default()));
    rt.block_on(async {
        assert_eq!(
            client.home_directory().await.unwrap(),
            server.home.to_str().unwrap()
        );
        let root = client
            .list_remote_entries(home.to_str().unwrap().into(), false)
            .await
            .unwrap();
        assert!(root.iter().any(|e| e.name == "work"));
        assert!(!root.iter().any(|e| e.name == ".ssh"));
        // Listing the keys folder is refused, also through a link.
        assert!(client
            .list_remote_entries(home.join(".ssh").to_str().unwrap().into(), true)
            .await
            .is_err());
        assert!(client
            .list_remote_entries(work.join("keys").to_str().unwrap().into(), true)
            .await
            .is_err());
        let w = client
            .list_remote_entries(work.to_str().unwrap().into(), false)
            .await
            .unwrap();
        let names: Vec<_> = w.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"big.bin") && names.contains(&"note.txt"));
        // Download: exact bytes, no .part left, progress reported.
        let dl = server._dir.path().join("dl.bin");
        let p = prog();
        let r = client
            .download_remote_file(
                work.join("big.bin").to_str().unwrap().into(),
                dl.to_str().unwrap().into(),
                10_000_000,
                p,
            )
            .await
            .unwrap();
        assert_eq!(r.bytes, big.len() as u64);
        assert_eq!(std::fs::read(&dl).unwrap(), big);
        assert!(!server._dir.path().join("dl.bin.part").exists());
        // Too large, a symlink to a key, a folder, a missing file.
        let to = |p: std::path::PathBuf| p.to_str().unwrap().to_string();
        assert!(matches!(
            client
                .download_remote_file(
                    to(work.join("big.bin")),
                    to(server._dir.path().join("x")),
                    1000,
                    prog()
                )
                .await,
            Err(SshCoreError::TooLarge { .. })
        ));
        assert!(client
            .download_remote_file(
                to(work.join("innocent.txt")),
                to(server._dir.path().join("y")),
                1000,
                prog()
            )
            .await
            .is_err());
        assert!(!server._dir.path().join("y").exists());
        assert!(client
            .download_remote_file(
                to(work.clone()),
                to(server._dir.path().join("z")),
                1000,
                prog()
            )
            .await
            .is_err());
        assert!(client
            .download_remote_file(
                to(work.join("nope")),
                to(server._dir.path().join("z")),
                1000,
                prog()
            )
            .await
            .is_err());
        // Cancelled before the first byte: nothing is left behind.
        let cancelled = Box::new(Progress(true.into(), Default::default()));
        assert!(matches!(
            client
                .download_remote_file(
                    to(work.join("big.bin")),
                    to(server._dir.path().join("c.bin")),
                    10_000_000,
                    cancelled
                )
                .await,
            Err(SshCoreError::Cancelled)
        ));
        assert!(!server._dir.path().join("c.bin.part").exists());
        // Upload, refuse overwrite, then overwrite.
        let src = server._dir.path().join("up.bin");
        std::fs::write(&src, &big).unwrap();
        let dest = to(work.join("it's up.bin"));
        client
            .upload_remote_file(to(src.clone()), dest.clone(), false, prog())
            .await
            .unwrap();
        assert_eq!(std::fs::read(work.join("it's up.bin")).unwrap(), big);
        assert!(matches!(
            client
                .upload_remote_file(to(src.clone()), dest.clone(), false, prog())
                .await,
            Err(SshCoreError::AlreadyExists { .. })
        ));
        std::fs::write(&src, "small").unwrap();
        client
            .upload_remote_file(to(src.clone()), dest.clone(), true, prog())
            .await
            .unwrap();
        assert_eq!(std::fs::read(work.join("it's up.bin")).unwrap(), b"small");
        assert!(!work.join("it's up.bin.part").exists());
        // Into the key folder: refused.
        assert!(client
            .upload_remote_file(to(src), to(home.join(".ssh/authorized_keys")), true, prog())
            .await
            .is_err());
        // Folder, rename (no replace), delete (file, empty folder, not a full one).
        client.make_remote_dir(to(work.join("d1"))).await.unwrap();
        assert!(client.make_remote_dir(to(work.join("d1"))).await.is_err());
        client
            .rename_remote(to(work.join("note.txt")), to(work.join("d1/n.txt")))
            .await
            .unwrap();
        assert!(work.join("d1/n.txt").exists() && !work.join("note.txt").exists());
        assert!(matches!(
            client
                .rename_remote(to(work.join("big.bin")), to(work.join("d1/n.txt")))
                .await,
            Err(SshCoreError::AlreadyExists { .. })
        ));
        assert!(
            client.delete_remote(to(work.join("d1"))).await.is_err(),
            "full folder"
        );
        assert!(work.join("d1/n.txt").exists());
        client
            .delete_remote(to(work.join("d1/n.txt")))
            .await
            .unwrap();
        client.delete_remote(to(work.join("d1"))).await.unwrap();
        assert!(!work.join("d1").exists());
        // Writes through a link into the key folder are refused, nothing lands there.
        std::fs::write(home.join(".ssh/known"), "k").unwrap();
        assert!(client
            .make_remote_dir(to(work.join("keys/newdir")))
            .await
            .is_err());
        assert!(!home.join(".ssh/newdir").exists());
        assert!(client
            .delete_remote(to(work.join("keys/known")))
            .await
            .is_err());
        assert!(client
            .rename_remote(to(work.join("keys/known")), to(work.join("stolen")))
            .await
            .is_err());
        assert!(client
            .rename_remote(to(work.join("big.bin")), to(work.join("keys/planted")))
            .await
            .is_err());
        assert!(
            home.join(".ssh/known").exists()
                && !work.join("stolen").exists()
                && !home.join(".ssh/planted").exists()
        );
        assert!(client
            .upload_remote_file(
                to(server._dir.path().join("up.bin")),
                to(work.join("keys/up")),
                true,
                prog()
            )
            .await
            .is_err());
        assert!(!home.join(".ssh/up").exists());
        assert!(client.delete_remote("/".into()).await.is_err());
        assert!(client
            .delete_remote(to(home.join(".ssh/id_rsa")))
            .await
            .is_err());
        assert!(home.join(".ssh/id_rsa").exists());
        assert!(
            client
                .stat_remote(to(work.join("big.bin")))
                .await
                .unwrap()
                .size
                == big.len() as u64
        );

        client.close().await;
        // Another pin: refused.
        let bad = RemoteFsClient::new(cfg(Some("SHA256:AAAA".into()))).unwrap();
        assert!(bad.home_directory().await.is_err());
    });
}

#[test]
fn git_branch_operations_run_in_a_temporary_repo_on_loopback_sshd() {
    use ssh_core::{GitBranchOp, GitDiffStatus};
    if !have("git") {
        eprintln!("SKIP: git unavailable");
        return;
    }
    let Some(server) = start_sshd() else {
        eprintln!("SKIP: sshd/ssh-keygen/tmux unavailable");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().canonicalize().unwrap().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .env("HOME", &server.home)
            .env_remove("TMUX")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "first"]);

    let session = format!("gb{}", server.port);
    let ok = server
        .tmux()
        .args([
            "new-session",
            "-d",
            "-s",
            &session,
            "-c",
            repo.to_str().unwrap(),
            "sleep 600",
        ])
        .status()
        .unwrap()
        .success();
    assert!(ok, "tmux new-session");

    let handle = connect(&server);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let op = |op: GitBranchOp| {
            let h = &handle;
            let s = session.clone();
            async move { h.run_git_branch_op(s, op).await }
        };
        let list = || handle.fetch_git_branches(session.clone());

        let r = list().await.unwrap();
        assert_eq!(r.status, GitDiffStatus::Ok);
        assert_eq!(r.current.as_deref(), Some("main"));
        assert!(!r.dirty && !r.detached);
        assert_eq!(r.branches.len(), 1);

        let created = op(GitBranchOp::Create {
            name: "feat/x".into(),
            from: None,
            switch_to: true,
        })
        .await
        .unwrap();
        assert!(created.ok, "{created:?}");
        assert_eq!(list().await.unwrap().current.as_deref(), Some("feat/x"));

        // A branch that is not checked out, from a named start point.
        let other = op(GitBranchOp::Create {
            name: "other".into(),
            from: Some("main".into()),
            switch_to: false,
        })
        .await
        .unwrap();
        assert!(other.ok, "{other:?}");
        assert_eq!(list().await.unwrap().branches.len(), 3);

        // Dirty tree: the list says so.
        std::fs::write(repo.join("a.txt"), "two\n").unwrap();
        assert!(list().await.unwrap().dirty);
        git(&["checkout", "-q", "--", "a.txt"]);

        let back = op(GitBranchOp::Switch {
            name: "main".into(),
        })
        .await
        .unwrap();
        assert!(back.ok, "{back:?}");

        // Safe delete of a merged branch keeps a backup ref.
        let del = op(GitBranchOp::Delete {
            name: "other".into(),
            force: false,
        })
        .await
        .unwrap();
        assert!(del.ok, "{del:?}");
        assert!(del.output.contains("refs/ssh-app/backup/"), "{del:?}");
        let backups = git(&["for-each-ref", "refs/ssh-app/backup"]);
        assert!(backups.contains("-other"), "{backups}");

        // The current branch cannot be deleted.
        let cur = op(GitBranchOp::Delete {
            name: "main".into(),
            force: true,
        })
        .await
        .unwrap();
        assert!(!cur.ok);

        // An unmerged branch needs force; the refused attempt leaves no backup behind.
        git(&["switch", "-q", "-c", "unmerged"]);
        std::fs::write(repo.join("b.txt"), "x\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "work"]);
        git(&["switch", "-q", "main"]);
        let before = git(&["for-each-ref", "refs/ssh-app/backup"])
            .lines()
            .count();
        let soft = op(GitBranchOp::Delete {
            name: "unmerged".into(),
            force: false,
        })
        .await
        .unwrap();
        assert!(!soft.ok, "{soft:?}");
        assert_eq!(
            git(&["for-each-ref", "refs/ssh-app/backup"])
                .lines()
                .count(),
            before
        );
        let hard = op(GitBranchOp::Delete {
            name: "unmerged".into(),
            force: true,
        })
        .await
        .unwrap();
        assert!(hard.ok, "{hard:?}");
        assert!(git(&["for-each-ref", "refs/ssh-app/backup"]).contains("-unmerged"));

        // Names that could be options or shell syntax never reach the host.
        for bad in ["-f", "a;b", "x y", "a..b"] {
            assert!(
                op(GitBranchOp::Switch { name: bad.into() }).await.is_err(),
                "{bad}"
            );
        }
        // A plain shell session that is not in a repository.
        let elsewhere = format!("gbn{}", server.port);
        let ok = server
            .tmux()
            .args([
                "new-session",
                "-d",
                "-s",
                &elsewhere,
                "-c",
                "/",
                "sleep 600",
            ])
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let none = handle.fetch_git_branches(elsewhere).await.unwrap();
        assert_eq!(none.status, GitDiffStatus::NotARepo);
    });
    handle.disconnect();
}

#[test]
fn git_worktree_operations_run_in_a_temporary_repo_on_loopback_sshd() {
    use ssh_core::{GitDiffStatus, GitWorktreeOp};
    if !have("git") {
        eprintln!("SKIP: git unavailable");
        return;
    }
    let Some(server) = start_sshd() else {
        eprintln!("SKIP: sshd/ssh-keygen/tmux unavailable");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().canonicalize().unwrap().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .env("HOME", &server.home)
            .env_remove("TMUX")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "first"]);
    git(&["branch", "existing"]);

    let session = format!("gw{}", server.port);
    let ok = server
        .tmux()
        .args([
            "new-session",
            "-d",
            "-s",
            &session,
            "-c",
            repo.to_str().unwrap(),
            "sleep 600",
        ])
        .status()
        .unwrap()
        .success();
    assert!(ok, "tmux new-session");
    let handle = connect(&server);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let op = |op: GitWorktreeOp| handle.run_git_worktree_op(session.clone(), op);
        let list = || handle.fetch_git_worktrees(session.clone());

        let r = list().await.unwrap();
        assert_eq!(r.status, GitDiffStatus::Ok);
        assert_eq!(r.worktrees.len(), 1);
        assert!(r.worktrees[0].is_main && r.worktrees[0].is_current);

        // New branch in a sibling folder named after the repository and the branch.
        let added = op(GitWorktreeOp::Add {
            branch: "feat/x".into(),
            new_branch: true,
            from: None,
        })
        .await
        .unwrap();
        assert!(added.ok, "{added:?}");
        let dest = repo.parent().unwrap().join("repo-feat-x");
        assert!(dest.join("a.txt").exists(), "{added:?}");
        assert!(added
            .output
            .contains(&format!("Worktree: {}", dest.display())));

        // The same name again: the folder exists, nothing is touched.
        let again = op(GitWorktreeOp::Add {
            branch: "feat/x".into(),
            new_branch: true,
            from: None,
        })
        .await
        .unwrap();
        assert!(!again.ok);

        // An existing branch.
        let ex = op(GitWorktreeOp::Add {
            branch: "existing".into(),
            new_branch: false,
            from: None,
        })
        .await
        .unwrap();
        assert!(ex.ok, "{ex:?}");
        assert_eq!(list().await.unwrap().worktrees.len(), 3);

        // Removal: only listed worktrees; never the main one.
        let main = op(GitWorktreeOp::Remove {
            path: repo.to_str().unwrap().into(),
            force: true,
        })
        .await
        .unwrap();
        assert!(!main.ok, "{main:?}");
        let stray = tmp.path().join("not-a-worktree");
        std::fs::create_dir_all(&stray).unwrap();
        let notwt = op(GitWorktreeOp::Remove {
            path: stray.to_str().unwrap().into(),
            force: true,
        })
        .await
        .unwrap();
        assert!(!notwt.ok && stray.exists(), "{notwt:?}");

        // A dirty worktree needs force.
        std::fs::write(dest.join("a.txt"), "dirty\n").unwrap();
        let soft = op(GitWorktreeOp::Remove {
            path: dest.to_str().unwrap().into(),
            force: false,
        })
        .await
        .unwrap();
        assert!(!soft.ok && dest.exists(), "{soft:?}");
        let hard = op(GitWorktreeOp::Remove {
            path: dest.to_str().unwrap().into(),
            force: true,
        })
        .await
        .unwrap();
        assert!(hard.ok && !dest.exists(), "{hard:?}");

        for bad in ["relative", "/a/../b"] {
            assert!(op(GitWorktreeOp::Remove {
                path: bad.into(),
                force: false
            })
            .await
            .is_err());
        }
    });
    handle.disconnect();
}

#[test]
fn git_commit_operations_run_in_a_temporary_repo_on_loopback_sshd() {
    use ssh_core::{GitCommitOp, GitResetMode};
    if !have("git") {
        eprintln!("SKIP: git unavailable");
        return;
    }
    let Some(server) = start_sshd() else {
        eprintln!("SKIP: sshd/ssh-keygen/tmux unavailable");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let repo = base.join("repo");
    let remote = base.join("remote.git");
    std::fs::create_dir_all(&repo).unwrap();
    let git_in = |dir: &std::path::Path, args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .env("HOME", &server.home)
            .env_remove("TMUX")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let git = |args: &[&str]| git_in(&repo, args);
    let commit_file = |name: &str, body: &str, msg: &str| {
        std::fs::write(repo.join(name), body).unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", msg]);
        git(&["rev-parse", "HEAD"])
    };
    git(&["init", "-q", "-b", "main"]);
    let c1 = commit_file("a.txt", "one\n", "c1");
    // side branch: two independent commits and one that will conflict with main
    git(&["switch", "-q", "-c", "side"]);
    let s1 = commit_file("s1.txt", "s1\n", "s1");
    let s2 = commit_file("s2.txt", "s2\n", "s2");
    let sconf = commit_file("a.txt", "side edit\n", "side edits a");
    git(&["switch", "-q", "main"]);
    let c2 = commit_file("a.txt", "main edit\n", "c2");

    let session = format!("gc{}", server.port);
    let ok = server
        .tmux()
        .args([
            "new-session",
            "-d",
            "-s",
            &session,
            "-c",
            repo.to_str().unwrap(),
            "sleep 600",
        ])
        .status()
        .unwrap()
        .success();
    assert!(ok, "tmux new-session");
    let handle = connect(&server);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let backups = || {
        git(&["for-each-ref", "refs/ssh-app/backup"])
            .lines()
            .count()
    };
    rt.block_on(async {
        let op = |op: GitCommitOp| handle.run_git_commit_op(session.clone(), op);

        // Cherry-pick two commits in order.
        let r = op(GitCommitOp::CherryPick {
            hashes: vec![s1.clone(), s2.clone()],
        })
        .await
        .unwrap();
        assert!(r.ok, "{r:?}");
        assert!(repo.join("s1.txt").exists() && repo.join("s2.txt").exists());
        assert_eq!(backups(), 1);

        // A conflicting pick is aborted by the app: tree clean, HEAD unchanged.
        let head = git(&["rev-parse", "HEAD"]);
        let r = op(GitCommitOp::CherryPick {
            hashes: vec![sconf.clone()],
        })
        .await
        .unwrap();
        assert!(!r.ok, "{r:?}");
        assert!(r.output.contains("aborted"), "{r:?}");
        assert_eq!(git(&["rev-parse", "HEAD"]), head);
        assert_eq!(git(&["status", "--porcelain"]), "");

        // Not a commit / unknown id.
        assert!(op(GitCommitOp::CherryPick {
            hashes: vec!["HEAD~1".into()]
        })
        .await
        .is_err());
        let r = op(GitCommitOp::CherryPick {
            hashes: vec!["abcdef1234".into()],
        })
        .await
        .unwrap();
        assert!(!r.ok);

        // Dirty tracked file: refused, nothing changed.
        std::fs::write(repo.join("a.txt"), "dirty\n").unwrap();
        let r = op(GitCommitOp::Revert { hash: head.clone() })
            .await
            .unwrap();
        assert!(!r.ok && r.output.contains("uncommitted"), "{r:?}");
        git(&["checkout", "-q", "--", "a.txt"]);

        // Revert the last commit: a new commit that removes s2.txt.
        let r = op(GitCommitOp::Revert { hash: head.clone() })
            .await
            .unwrap();
        assert!(r.ok, "{r:?}");
        assert!(!repo.join("s2.txt").exists());

        // Reset must stay inside this branch's history; the other branch is refused.
        let r = op(GitCommitOp::Reset {
            hash: sconf.clone(),
            mode: GitResetMode::Soft,
        })
        .await
        .unwrap();
        assert!(!r.ok, "{r:?}");
        let tip = git(&["rev-parse", "HEAD"]);
        let r = op(GitCommitOp::Reset {
            hash: c2.clone(),
            mode: GitResetMode::Soft,
        })
        .await
        .unwrap();
        assert!(r.ok, "{r:?}");
        assert_eq!(git(&["rev-parse", "HEAD"]), c2);
        assert!(r.output.contains(&tip), "old tip is reported: {r:?}");
        // Soft: the undone changes are staged, files stay.
        assert!(repo.join("s1.txt").exists());
        assert!(git(&["status", "--porcelain"]).contains("s1.txt"));
        let r = op(GitCommitOp::Reset {
            hash: c1.clone(),
            mode: GitResetMode::Mixed,
        })
        .await
        .unwrap();
        assert!(r.ok, "{r:?}");
        assert!(repo.join("s1.txt").exists(), "mixed keeps files");

        // Reword the last commit; staged files are not swept into it.
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "work"]);
        std::fs::write(repo.join("staged.txt"), "x\n").unwrap();
        git(&["add", "staged.txt"]);
        let r = op(GitCommitOp::AmendMessage {
            message: "better message\n\nbody".into(),
        })
        .await
        .unwrap();
        assert!(r.ok, "{r:?}");
        assert_eq!(git(&["log", "-1", "--format=%s"]), "better message");
        assert!(!git(&["show", "--name-only", "--format=", "HEAD"]).contains("staged.txt"));
        git(&["reset", "-q", "staged.txt"]);

        // Once the commit is on a remote branch, rewording is refused.
        git_in(&base, &["init", "-q", "--bare", remote.to_str().unwrap()]);
        git(&["remote", "add", "origin", remote.to_str().unwrap()]);
        git(&["push", "-q", "origin", "main"]);
        let r = op(GitCommitOp::AmendMessage {
            message: "again".into(),
        })
        .await
        .unwrap();
        assert!(!r.ok && r.output.contains("remote"), "{r:?}");

        assert!(op(GitCommitOp::Reset {
            hash: "--hard".into(),
            mode: GitResetMode::Soft
        })
        .await
        .is_err());
    });
    handle.disconnect();
}

#[test]
fn git_history_and_stash_operations_run_in_a_temporary_repo_on_loopback_sshd() {
    use ssh_core::GitHistoryOp;
    if !have("git") {
        eprintln!("SKIP: git unavailable");
        return;
    }
    let Some(server) = start_sshd() else {
        eprintln!("SKIP: sshd/ssh-keygen/tmux unavailable");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let repo = base.join("repo");
    let remote = base.join("remote.git");
    std::fs::create_dir_all(&repo).unwrap();
    let git_in = |dir: &std::path::Path, args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .env("HOME", &server.home)
            .env_remove("TMUX")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let git = |args: &[&str]| git_in(&repo, args);
    let commit_file = |name: &str, body: &str, msg: &str| {
        std::fs::write(repo.join(name), body).unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", msg]);
        git(&["rev-parse", "HEAD"])
    };
    git(&["init", "-q", "-b", "main"]);
    commit_file("base.txt", "base\n", "base");
    git_in(&base, &["init", "-q", "--bare", remote.to_str().unwrap()]);
    git(&["remote", "add", "origin", remote.to_str().unwrap()]);
    commit_file("pre.txt", "pre\n", "pre");
    git(&["push", "-q", "origin", "main"]);

    let session = format!("gh{}", server.port);
    let ok = server
        .tmux()
        .args([
            "new-session",
            "-d",
            "-s",
            &session,
            "-c",
            repo.to_str().unwrap(),
            "sleep 600",
        ])
        .status()
        .unwrap()
        .success();
    assert!(ok, "tmux new-session");
    let handle = connect(&server);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let op = |op: GitHistoryOp| handle.run_git_history_op(session.clone(), op);

        // Squash: three local commits become one; the pushed base is never touched.
        commit_file("f1.txt", "1\n", "one");
        commit_file("f2.txt", "2\n", "two");
        let third = commit_file("f3.txt", "3\n", "three");
        let r = op(GitHistoryOp::Squash {
            count: 4,
            message: "x".into(),
        })
        .await
        .unwrap();
        assert!(
            !r.ok && r.output.contains("remote"),
            "pushed base is refused: {r:?}"
        );
        let r = op(GitHistoryOp::Squash {
            count: 3,
            message: "feat: combined\n\nbody".into(),
        })
        .await
        .unwrap();
        assert!(r.ok, "{r:?}");
        assert_eq!(git(&["log", "-1", "--format=%s"]), "feat: combined");
        assert_eq!(git(&["rev-list", "--count", "HEAD"]), "3");
        assert!(repo.join("f1.txt").exists() && repo.join("f3.txt").exists());
        assert!(git(&["for-each-ref", "refs/ssh-app/backup"]).contains("-squash"));
        assert!(
            r.output.contains(&third) || r.output.contains("saved as"),
            "{r:?}"
        );

        // Drop: remove a middle commit; a later commit that depends on it conflicts and is aborted.
        let a = commit_file("d.txt", "a\n", "A adds d");
        let b = commit_file("d.txt", "a\nb\n", "B edits d");
        let c = commit_file("e.txt", "e\n", "C adds e");
        let head = git(&["rev-parse", "HEAD"]);
        let r = op(GitHistoryOp::Drop { hash: a.clone() }).await.unwrap();
        assert!(!r.ok && r.output.contains("aborted"), "{r:?}");
        assert_eq!(git(&["rev-parse", "HEAD"]), head);
        assert_eq!(git(&["status", "--porcelain"]), "");
        let r = op(GitHistoryOp::Drop { hash: c.clone() }).await.unwrap();
        assert!(r.ok, "{r:?}");
        assert!(!repo.join("e.txt").exists());
        assert!(git(&["log", "--format=%H"]).contains(&b));
        // Pushed history cannot be dropped.
        let first_pushed = git(&["rev-parse", "origin/main"]);
        let r = op(GitHistoryOp::Drop { hash: first_pushed }).await.unwrap();
        assert!(!r.ok, "{r:?}");

        // Merge: clean merge works, a conflicting one is aborted.
        git(&["switch", "-q", "-c", "topic"]);
        commit_file("topic.txt", "t\n", "topic work");
        std::fs::write(repo.join("d.txt"), "topic side\n").unwrap();
        git(&["commit", "-q", "-am", "topic edits d"]);
        git(&["switch", "-q", "main"]);
        commit_file("d.txt", "main side\n", "main edits d");
        let before = git(&["rev-parse", "HEAD"]);
        let r = op(GitHistoryOp::Merge {
            branch: "topic".into(),
        })
        .await
        .unwrap();
        assert!(!r.ok && r.output.contains("aborted"), "{r:?}");
        assert_eq!(git(&["rev-parse", "HEAD"]), before);
        assert_eq!(git(&["status", "--porcelain"]), "");
        git(&["switch", "-q", "-c", "clean-topic", "HEAD~1"]);
        commit_file("clean.txt", "c\n", "clean work");
        git(&["switch", "-q", "main"]);
        let r = op(GitHistoryOp::Merge {
            branch: "clean-topic".into(),
        })
        .await
        .unwrap();
        assert!(r.ok, "{r:?}");
        assert!(repo.join("clean.txt").exists());

        // Rebase: conflict aborted; clean rebase works; pushed commits refused.
        git(&["switch", "-q", "topic"]);
        let tip = git(&["rev-parse", "HEAD"]);
        let r = op(GitHistoryOp::Rebase {
            onto: "main".into(),
        })
        .await
        .unwrap();
        assert!(!r.ok && r.output.contains("aborted"), "{r:?}");
        assert_eq!(git(&["rev-parse", "HEAD"]), tip);
        git(&["switch", "-q", "main"]);
        git(&["switch", "-q", "-c", "late", "HEAD~2"]);
        commit_file("late.txt", "l\n", "late work");
        let r = op(GitHistoryOp::Rebase {
            onto: "main".into(),
        })
        .await
        .unwrap();
        assert!(r.ok, "{r:?}");
        assert!(repo.join("clean.txt").exists() && repo.join("late.txt").exists());
        git(&["switch", "-q", "main"]);
        let r = op(GitHistoryOp::Rebase {
            onto: "late".into(),
        })
        .await
        .unwrap();
        // main has unpushed commits only after the push base, so this one is refused or ok;
        // what must hold is that nothing is left half-done.
        assert_eq!(git(&["status", "--porcelain"]), "", "{r:?}");

        // Stash: push, list, pop restores, drop removes; dirty tree blocks pop.
        git(&["switch", "-q", "main"]);
        std::fs::write(repo.join("base.txt"), "changed\n").unwrap();
        let r = op(GitHistoryOp::StashPush {
            message: "wip: base".into(),
            include_untracked: false,
        })
        .await
        .unwrap();
        assert!(r.ok, "{r:?}");
        assert_eq!(
            std::fs::read_to_string(repo.join("base.txt")).unwrap(),
            "base\n"
        );
        let list = handle.fetch_git_stashes(session.clone()).await.unwrap();
        assert_eq!(list.stashes.len(), 1);
        assert!(list.stashes[0].message.contains("wip: base"), "{list:?}");
        std::fs::write(repo.join("base.txt"), "other\n").unwrap();
        let r = op(GitHistoryOp::StashPop { index: 0 }).await.unwrap();
        assert!(!r.ok && r.output.contains("uncommitted"), "{r:?}");
        git(&["checkout", "-q", "--", "base.txt"]);
        let r = op(GitHistoryOp::StashPop { index: 0 }).await.unwrap();
        assert!(r.ok, "{r:?}");
        assert_eq!(
            std::fs::read_to_string(repo.join("base.txt")).unwrap(),
            "changed\n"
        );
        assert!(handle
            .fetch_git_stashes(session.clone())
            .await
            .unwrap()
            .stashes
            .is_empty());
        git(&["checkout", "-q", "--", "base.txt"]);
        std::fs::write(repo.join("untracked.txt"), "u\n").unwrap();
        let r = op(GitHistoryOp::StashPush {
            message: "with untracked".into(),
            include_untracked: true,
        })
        .await
        .unwrap();
        assert!(r.ok && !repo.join("untracked.txt").exists(), "{r:?}");
        let r = op(GitHistoryOp::StashDrop { index: 0 }).await.unwrap();
        assert!(r.ok, "{r:?}");
        assert!(git(&["for-each-ref", "refs/ssh-app/backup"]).contains("-stash"));
        let r = op(GitHistoryOp::StashDrop { index: 7 }).await.unwrap();
        assert!(!r.ok);

        for bad in [
            GitHistoryOp::Drop {
                hash: "--root".into(),
            },
            GitHistoryOp::Merge {
                branch: "a b".into(),
            },
            GitHistoryOp::Squash {
                count: 1,
                message: "x".into(),
            },
        ] {
            assert!(op(bad).await.is_err());
        }
    });
    handle.disconnect();
}

#[test]
fn git_index_and_remote_operations_run_in_a_temporary_repo_on_loopback_sshd() {
    use ssh_core::{GitFileState, GitIndexOp};
    if !have("git") {
        eprintln!("SKIP: git unavailable");
        return;
    }
    let Some(server) = start_sshd() else {
        eprintln!("SKIP: sshd/ssh-keygen/tmux unavailable");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let repo = base.join("repo");
    let remote = base.join("remote.git");
    let other = base.join("other");
    std::fs::create_dir_all(&repo).unwrap();
    let git_in = |dir: &std::path::Path, args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .env("HOME", &server.home)
            .env_remove("TMUX")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let git = |args: &[&str]| git_in(&repo, args);
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "a\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "first"]);
    git_in(&base, &["init", "-q", "--bare", remote.to_str().unwrap()]);
    git(&["remote", "add", "origin", remote.to_str().unwrap()]);

    let session = format!("gx{}", server.port);
    let ok = server
        .tmux()
        .args([
            "new-session",
            "-d",
            "-s",
            &session,
            "-c",
            repo.to_str().unwrap(),
            "sleep 600",
        ])
        .status()
        .unwrap()
        .success();
    assert!(ok, "tmux new-session");
    let handle = connect(&server);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let op = |op: GitIndexOp| handle.run_git_index_op(session.clone(), op);
        let changes = || handle.fetch_git_changes(session.clone());
        let paths = |p: &[&str]| p.iter().map(|s| s.to_string()).collect::<Vec<_>>();

        std::fs::write(repo.join("a.txt"), "a2\n").unwrap();
        std::fs::write(repo.join("new.txt"), "n\n").unwrap();
        std::fs::write(repo.join("star*.txt"), "literal\n").unwrap();
        std::fs::write(repo.join("star1.txt"), "other\n").unwrap();
        let c = changes().await.unwrap();
        let e = |c: &ssh_core::GitChangesResult, p: &str| {
            c.entries.iter().find(|e| e.path == p).cloned()
        };
        assert_eq!(
            e(&c, "a.txt").unwrap().worktree_state,
            Some(GitFileState::Modified)
        );
        assert_eq!(
            e(&c, "new.txt").unwrap().worktree_state,
            Some(GitFileState::Untracked)
        );
        assert_eq!(c.branch.as_deref(), Some("main"));
        assert!(c.upstream.is_none());

        // Nothing staged yet: commit refused.
        let r = op(GitIndexOp::Commit {
            message: "x".into(),
        })
        .await
        .unwrap();
        assert!(!r.ok && r.output.contains("Nothing is staged"), "{r:?}");

        // A path is literal: "star*.txt" stages that file, not star1.txt.
        let r = op(GitIndexOp::Stage {
            paths: paths(&["a.txt", "star*.txt"]),
        })
        .await
        .unwrap();
        assert!(r.ok, "{r:?}");
        let c = changes().await.unwrap();
        assert_eq!(
            e(&c, "a.txt").unwrap().index_state,
            Some(GitFileState::Modified)
        );
        assert_eq!(
            e(&c, "star*.txt").unwrap().index_state,
            Some(GitFileState::Added)
        );
        assert_eq!(
            e(&c, "star1.txt").unwrap().index_state,
            None,
            "glob did not expand"
        );

        let r = op(GitIndexOp::Unstage {
            paths: paths(&["star*.txt"]),
        })
        .await
        .unwrap();
        assert!(r.ok, "{r:?}");
        assert_eq!(
            e(&changes().await.unwrap(), "star*.txt")
                .unwrap()
                .index_state,
            None
        );

        let r = op(GitIndexOp::Commit {
            message: "feat: a\n\nbody".into(),
        })
        .await
        .unwrap();
        assert!(r.ok, "{r:?}");
        assert_eq!(git(&["log", "-1", "--format=%s"]), "feat: a");
        assert!(!git(&["show", "--name-only", "--format=", "HEAD"]).contains("new.txt"));

        let r = op(GitIndexOp::StageAll).await.unwrap();
        assert!(r.ok);
        assert!(changes()
            .await
            .unwrap()
            .entries
            .iter()
            .all(|x| x.index_state.is_some()));
        let r = op(GitIndexOp::UnstageAll).await.unwrap();
        assert!(r.ok);
        assert!(changes()
            .await
            .unwrap()
            .entries
            .iter()
            .all(|x| x.index_state.is_none()));

        // Push without an upstream and exactly one remote: sets it.
        let r = op(GitIndexOp::Push).await.unwrap();
        assert!(r.ok, "{r:?}");
        let c = changes().await.unwrap();
        assert_eq!(c.upstream.as_deref(), Some("origin/main"));
        assert_eq!((c.ahead, c.behind), (0, 0));

        // Someone else pushes; fetch shows behind 1; dirty tree blocks pull; clean pull fast-forwards.
        git_in(
            &base,
            &[
                "clone",
                "-q",
                remote.to_str().unwrap(),
                other.to_str().unwrap(),
            ],
        );
        std::fs::write(other.join("remote.txt"), "r\n").unwrap();
        git_in(&other, &["add", "."]);
        git_in(&other, &["commit", "-q", "-m", "remote work"]);
        git_in(&other, &["push", "-q", "origin", "main"]);
        let r = op(GitIndexOp::Fetch).await.unwrap();
        assert!(r.ok, "{r:?}");
        assert_eq!(changes().await.unwrap().behind, 1);
        // Untracked files do not block a fast-forward; a modified tracked file does.
        std::fs::write(repo.join("a.txt"), "dirty tracked\n").unwrap();
        let r = op(GitIndexOp::Pull).await.unwrap();
        assert!(!r.ok && r.output.contains("uncommitted"), "dirty: {r:?}");
        git(&["checkout", "-q", "--", "a.txt"]);
        git(&["add", "star1.txt", "new.txt", "star*.txt"]);
        git(&["commit", "-q", "-m", "local"]);
        // Diverged: pull --ff-only refuses and changes nothing.
        let head = git(&["rev-parse", "HEAD"]);
        let r = op(GitIndexOp::Pull).await.unwrap();
        assert!(!r.ok, "{r:?}");
        assert_eq!(git(&["rev-parse", "HEAD"]), head);
        // A rejected (non-fast-forward) push is reported, never forced.
        let r = op(GitIndexOp::Push).await.unwrap();
        assert!(!r.ok, "{r:?}");
        assert_ne!(git(&["rev-parse", "origin/main"]), head);
        git(&["reset", "-q", "--hard", "origin/main"]);
        let r = op(GitIndexOp::Pull).await.unwrap();
        assert!(r.ok, "{r:?}");
        assert!(repo.join("remote.txt").exists());

        // Bad paths never reach the host.
        for bad in ["-A", "../x", "/etc/passwd", "a\nb"] {
            assert!(
                op(GitIndexOp::Stage {
                    paths: paths(&[bad])
                })
                .await
                .is_err(),
                "{bad}"
            );
        }
        assert!(op(GitIndexOp::Commit {
            message: " ".into()
        })
        .await
        .is_err());

        // Several remotes and no upstream: refuse to guess.
        git(&["switch", "-q", "-c", "solo"]);
        git(&["remote", "add", "second", remote.to_str().unwrap()]);
        let r = op(GitIndexOp::Push).await.unwrap();
        assert!(!r.ok && r.output.contains("several remotes"), "{r:?}");
    });
    handle.disconnect();
}
