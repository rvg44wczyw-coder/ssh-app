//! Live check of the sandbox plan against a real Docker engine. Ignored by default and gated by
//! `SSH_APP_LIVE_DOCKER=1` (optionally `SSH_APP_LIVE_AGENTS=claude,codex,grok,agy`); it touches only
//! resources named `ssh-app-sbx-*` / labelled `ssh-app.sandbox`, runs with a throw-away `$HOME` and a
//! throw-away git repository. No agent is logged in or started: the check is the image, the
//! isolation, the proxy allow-list and the review fetch.

use ssh_core::sandbox::*;
use std::path::Path;
use std::process::Command;

fn sh(home: &Path, script: &str) -> (bool, String) {
    let real = std::env::var("HOME").unwrap();
    let out = Command::new("sh")
        .arg("-c")
        .arg(script)
        .env("HOME", home)
        .env("DOCKER_CONFIG", format!("{real}/.docker"))
        .env_remove("TMUX")
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

fn quote(s: &str) -> String {
    s.replace('\'', "'\\''")
}

/// Runs the whole check for one agent; returns a report and the list of failed expectations.
fn check(agent: SandboxAgent, binary: &str, version_cmd: &str, id: &str) -> (String, Vec<String>) {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let repo = tmp.path().join("proj");
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::write(home.join(".ssh/id_ed25519"), "HOST-SECRET-KEY").unwrap();
    std::fs::create_dir_all(home.join(".ssh-app")).unwrap();
    std::fs::write(
        home.join(".ssh-app/allowed_signers"),
        "phone ssh-ed25519 AAAA\n",
    )
    .unwrap();
    std::fs::create_dir_all(repo.join(".git/hooks")).ok();
    let g = |a: &str| sh(&home, &format!("git -C {} {a}", repo.display()));
    assert!(g("init -q -b main").0);
    std::fs::write(repo.join("README.md"), "hello\n").unwrap();
    std::fs::write(repo.join(".gitignore"), ".env\n").unwrap();
    std::fs::write(repo.join(".env"), "UNTRACKED-SECRET").unwrap();
    std::fs::write(
        repo.join(".git/hooks/post-checkout"),
        "#!/bin/sh\ntouch /tmp/HOOK-RAN\n",
    )
    .unwrap();
    assert!(g("add README.md .gitignore").0);
    assert!(g("-c user.name=t -c user.email=t@example.com commit -qm init").0);

    let profile = SandboxProfile {
        engine: SandboxEngine::Docker,
        agent,
        sandbox_id: id.into(),
        project_folder: repo.to_string_lossy().into(),
        network: SandboxNetwork::Online,
        allow_hosts: vec![],
        cpus: 1,
        memory_mb: 1024,
        pids: 256,
        auto_approve: false,
        base_image: DEFAULT_BASE_IMAGE.into(),
        copy_login: false,
        claude_config_dir: String::new(),
    };
    let plan = plan_sandbox(&profile).unwrap();
    let mut log = String::new();
    let mut fails: Vec<String> = Vec::new();
    let mut step = |name: &str, cmd: &str| {
        let (ok, out) = sh(&home, cmd);
        log.push_str(&format!(
            "--- {name}: ok={ok}\n{}\n",
            out.chars()
                .rev()
                .take(1500)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>()
        ));
        (ok, out)
    };
    let mut ready = true;
    for (name, cmd) in [
        ("build", plan.build_command.clone()),
        ("proxy build", plan.proxy_build_command.clone()),
        ("setup0", plan.setup_commands[0].clone()),
        (
            "setup1",
            plan.setup_commands.get(1).cloned().unwrap_or_default(),
        ),
        ("clone", plan.clone_command.clone()),
    ] {
        if !cmd.is_empty() && !step(name, &cmd).0 {
            fails.push(format!("{name} failed"));
            ready = false;
            break;
        }
    }
    let mut extra = String::new();
    if ready {
        let tag = plan.image_tag.clone();
        let run_with = |script: &str| {
            let mut cmd = plan.run_command.replace("run -it --rm", "run --rm");
            let at = cmd.rfind(&format!(" {binary}")).unwrap();
            cmd.truncate(at);
            cmd.push_str(&format!(" sh -c '{}'", quote(script)));
            cmd
        };
        // The agent is installed and starts (no login).
        let (_, v) = step(
            "binary",
            &run_with(&format!(
                "command -v {binary}; {version_cmd} 2>&1 | head -2"
            )),
        );
        if !v.lines().any(|l| l.starts_with('/') && l.ends_with(binary)) {
            fails.push(format!("{binary} not found in the image"));
        }
        extra.push_str(&format!("version: {}\n", v.trim().replace('\n', " | ")));
        // Isolation.
        let probe = r#"
echo "id=$(id -u)"; grep CapEff /proc/self/status
ls /home/agent/.ssh /Users /var/run/docker.sock 2>&1 | grep -c "No such" | sed 's/^/missing=/'
touch /etc/x 2>&1 | head -1
echo "env=$(cat /work/repo/.env 2>&1 | head -1)"
touch /ssh-app/run/decisions/x 2>&1 | head -1
touch /ssh-app/run/pending/x && echo pending_writable
touch /ssh-app/allowed_signers 2>&1 | head -1
echo "direct=$(curl -s -o /dev/null -m 8 --noproxy '*' -w '%{http_code}' https://1.1.1.1/ 2>&1)"
cd /work/repo && touch new && git -c user.name=a -c user.email=a@example.com add new && git -c user.name=a -c user.email=a@example.com commit -qm fromsandbox && echo committed
"#;
        let (_, p) = step("probe", &run_with(probe));
        for (needle, what) in [
            ("CapEff:\t0000000000000000", "capabilities not empty"),
            ("missing=3", "host paths visible"),
            ("Read-only file system", "root or mounts writable"),
            (
                "env=cat: /work/repo/.env: No such file",
                "untracked file crossed",
            ),
            ("pending_writable", "pending not writable"),
            ("direct=000", "direct egress possible"),
            ("committed", "no commit in the sandbox"),
        ] {
            if !p.contains(needle) {
                fails.push(what.to_string());
            }
        }
        if p.contains("id=0") {
            fails.push("runs as root".into());
        }
        // The agent's own hosts through the proxy: any HTTP answer (even 4xx) proves the tunnel;
        // 000 means refused or unreachable.
        let hosts = parse_allowed_hosts(&plan.proxy_filter);
        let mut probes = String::new();
        for h in &hosts {
            let h = h.replace("*.", "www.");
            probes.push_str(&format!(
                "echo \"{h}=$(curl -s -o /dev/null -m 20 -w '%{{http_code}}' https://{h}/ 2>&1)\"\n"
            ));
        }
        probes.push_str("echo \"other=$(curl -s -o /dev/null -m 20 -w '%{http_code}' https://example.org/ 2>&1)\"\n");
        let (_, h) = step("proxy hosts", &run_with(&probes));
        extra.push_str(&format!("hosts: {}\n", h.trim().replace('\n', " | ")));
        if !h.contains("other=000") {
            fails.push("a host outside the list answered".into());
        }
        // Review fetch.
        step("fetch", &plan.fetch_command);
        let refs = sh(
            &home,
            &format!("git -C {} branch --list 'sandbox/*'", repo.display()),
        )
        .1;
        if !refs.contains(&format!("sandbox/{id}/sandbox-work")) {
            fails.push("review fetch brought nothing".into());
        }
        if Path::new("/tmp/HOOK-RAN").exists() {
            fails.push("host hook ran".into());
        }
        let _ = tag;
    }
    for c in &plan.cleanup_commands {
        step("cleanup", c);
    }
    step(
        "rmi",
        &format!("docker rmi {} $(docker images --format '{{{{.Repository}}}}:{{{{.Tag}}}}' | grep '^ssh-app-sbx-proxy:') 2>&1 | tail -1", plan.image_tag),
    );
    (format!("== {binary}\n{log}{extra}fails={fails:?}\n"), fails)
}

#[test]
#[ignore]
fn the_sandbox_holds_on_a_real_engine() {
    if std::env::var("SSH_APP_LIVE_DOCKER").as_deref() != Ok("1") {
        return;
    }
    let want =
        std::env::var("SSH_APP_LIVE_AGENTS").unwrap_or_else(|_| "claude,codex,grok,agy".into());
    let all = [
        (
            "claude",
            SandboxAgent::ClaudeCode,
            "claude --version",
            "live-claude",
        ),
        (
            "codex",
            SandboxAgent::Codex,
            "codex --version",
            "live-codex",
        ),
        ("grok", SandboxAgent::Grok, "grok --version", "live-grok"),
        (
            "agy",
            SandboxAgent::Antigravity,
            "agy --version",
            "live-agy",
        ),
    ];
    let mut report = String::new();
    let mut failed = Vec::new();
    for (name, agent, ver, id) in all {
        if !want.split(',').any(|w| w == name) {
            continue;
        }
        let (r, fails) = check(agent, name, ver, id);
        report.push_str(&r);
        if !fails.is_empty() {
            failed.push(format!("{name}: {fails:?}"));
        }
        std::fs::write("/private/tmp/claude-501/sbx/live.log", &report).unwrap();
    }
    assert!(failed.is_empty(), "{failed:?}; see live.log");
}

/// Prepares (and keeps) a sandbox so the owner can log an agent in by hand inside it:
/// `SSH_APP_LIVE_LOGIN=claude|codex|grok|agy`. Everything lives under `/private/tmp/claude-501/sbx/login`
/// (a throw-away `$HOME`); the run command is written to `login/run.sh`. Nothing is cleaned up.
#[test]
#[ignore]
fn prepare_a_sandbox_for_a_manual_login() {
    let Ok(which) = std::env::var("SSH_APP_LIVE_LOGIN") else {
        return;
    };
    let (agent, binary) = match which.as_str() {
        "claude" => (SandboxAgent::ClaudeCode, "claude"),
        "codex" => (SandboxAgent::Codex, "codex"),
        "grok" => (SandboxAgent::Grok, "grok"),
        _ => (SandboxAgent::Antigravity, "agy"),
    };
    let base = Path::new("/private/tmp/claude-501/sbx/login");
    let home = base.join("home");
    let repo = base.join("proj");
    std::fs::create_dir_all(home.join(".ssh-app")).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(
        home.join(".ssh-app/allowed_signers"),
        "phone ssh-ed25519 AAAA\n",
    )
    .unwrap();
    let g = |a: &str| sh(&home, &format!("git -C {} {a}", repo.display()));
    g("init -q -b main");
    std::fs::write(repo.join("README.md"), "login check\n").unwrap();
    g("add README.md");
    g("-c user.name=t -c user.email=t@example.com commit -qm init");
    let profile = SandboxProfile {
        engine: SandboxEngine::Docker,
        agent,
        sandbox_id: format!("login-{binary}"),
        project_folder: repo.to_string_lossy().into(),
        network: SandboxNetwork::Online,
        allow_hosts: vec![],
        cpus: 1,
        memory_mb: 2048,
        pids: 256,
        auto_approve: false,
        base_image: DEFAULT_BASE_IMAGE.into(),
        copy_login: true,
        claude_config_dir: std::env::var("SSH_APP_LIVE_CLAUDE_DIR").unwrap_or_default(),
    };
    let plan = plan_sandbox(&profile).unwrap();
    for cmd in [
        plan.build_command.clone(),
        plan.proxy_build_command.clone(),
        plan.setup_commands[0].clone(),
        plan.setup_commands[1].clone(),
        plan.login_command.clone(),
        plan.clone_command.clone(),
    ] {
        let (ok, out) = sh(&home, &cmd);
        assert!(ok, "{out}");
    }
    // The tab would run this through a login shell; here a tmux window does.
    let script = format!(
        "#!/bin/sh\nexport HOME='{}'\nexport DOCKER_CONFIG=\"{}/.docker\"\n{}\n",
        home.display(),
        std::env::var("HOME").unwrap(),
        plan.run_command
    );
    std::fs::write(base.join("run.sh"), script).unwrap();
}
