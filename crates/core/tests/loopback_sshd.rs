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
    // Sessions get a private HOME so installs never touch the developer's real ~/.ssh-app.
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).ok()?;
    let cfg = dir.path().join("sshd_config");
    std::fs::write(
        &cfg,
        format!(
            "Port {port}\nListenAddress 127.0.0.1\nHostKey {}\nAuthorizedKeysFile {}\n\
             PidFile {}\nUsePAM no\nStrictModes no\nPubkeyAuthentication yes\n\
             PasswordAuthentication no\nKbdInteractiveAuthentication no\nSetEnv HOME={}\n",
            host_key.display(),
            authorized.display(),
            dir.path().join("sshd.pid").display(),
            home.display()
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

        // Antigravity uses hooks.json in its dir; Codex/Grok are refused.
        let agy_dir = Some(root.join("gemini").to_str().unwrap().to_string());
        handle
            .install_agent_hooks(AgentKind::Antigravity, agy_dir.clone(), hooks.clone())
            .await
            .unwrap();
        let hj = std::fs::read_to_string(root.join("gemini/hooks.json")).unwrap();
        assert!(hj.contains("\"ssh-app\"") && hj.contains("run_command"));
        assert!(handle
            .install_agent_hooks(AgentKind::Codex, None, hooks.clone())
            .await
            .is_err());
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
