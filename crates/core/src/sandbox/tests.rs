use super::*;

const DIGEST: &str =
    "debian:12-slim@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn profile(agent: SandboxAgent) -> SandboxProfile {
    SandboxProfile {
        engine: SandboxEngine::Docker,
        agent,
        sandbox_id: "demo-1".into(),
        project_folder: "~/work/api".into(),
        network: SandboxNetwork::Online,
        allow_hosts: vec!["github.com".into()],
        cpus: 2,
        memory_mb: 2048,
        pids: 256,
        auto_approve: false,
        base_image: DIGEST.into(),
        copy_login: true,
        claude_config_dir: String::new(),
    }
}

const AGENTS: [SandboxAgent; 4] = [
    SandboxAgent::ClaudeCode,
    SandboxAgent::Antigravity,
    SandboxAgent::Codex,
    SandboxAgent::Grok,
];

fn all_commands(plan: &SandboxPlan) -> Vec<String> {
    let mut v = vec![
        plan.build_command.clone(),
        plan.proxy_build_command.clone(),
        plan.clone_command.clone(),
        plan.run_command.clone(),
        plan.fetch_command.clone(),
    ];
    v.extend(plan.setup_commands.clone());
    v.extend(plan.cleanup_commands.clone());
    v.retain(|c| !c.is_empty());
    v
}

#[test]
fn every_agent_and_network_makes_commands_the_shell_can_parse() {
    for agent in AGENTS {
        for network in [SandboxNetwork::Online, SandboxNetwork::Offline] {
            for auto in [false, true] {
                let mut p = profile(agent);
                p.network = network;
                p.auto_approve = auto;
                let plan = plan_sandbox(&p).unwrap();
                for c in all_commands(&plan) {
                    let ok = std::process::Command::new("sh")
                        .args(["-n", "-c", &c])
                        .status()
                        .unwrap()
                        .success();
                    assert!(ok, "{agent:?} {network:?}: {c}");
                }
                assert_eq!(
                    plan.proxy_build_command.is_empty(),
                    network == SandboxNetwork::Offline
                );
            }
        }
    }
}

#[test]
fn no_command_ever_carries_a_privilege_or_a_host_namespace() {
    for agent in AGENTS {
        let plan = plan_sandbox(&profile(agent)).unwrap();
        for c in all_commands(&plan) {
            for bad in [
                "--privileged",
                "--network host",
                "--network=host",
                "--pid=",
                "--pid ",
                "--ipc",
                "--userns",
                "--cap-add",
                "--device",
                "docker.sock",
                "--security-opt seccomp",
                "unconfined",
                "-v /:",
                "-v \"$HOME\":",
            ] {
                assert!(!c.contains(bad), "{agent:?} has {bad}: {c}");
            }
        }
        let run = &plan.run_command;
        for must in [
            "--cap-drop=ALL",
            "--security-opt=no-new-privileges",
            "--read-only",
            "--pids-limit 256",
            "--memory 2048m",
            "--cpus 2",
            "--rm",
        ] {
            assert!(run.contains(must), "{agent:?} lacks {must}");
        }
        assert!(
            run.contains("--user \"$(id -u):$(id -g)\""),
            "never root: {run}"
        );
    }
}

/// The only host paths a running agent can reach are the sandbox's own folders and the phone's
/// public key; the project folder itself is never mounted.
#[test]
fn the_agent_container_mounts_only_the_sandbox_folders() {
    let plan = plan_sandbox(&profile(SandboxAgent::ClaudeCode)).unwrap();
    let mounts: Vec<&str> = plan
        .run_command
        .split(" -v ")
        .skip(1)
        .map(|m| m.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(
        mounts,
        [
            "ssh-app-sbx-demo-1-work:/work",
            "\"$HOME\"/.ssh-app/sandboxes/demo-1/home:/home/agent",
            "\"$HOME\"/.ssh-app/sandboxes/demo-1/run/pending:/ssh-app/run/pending",
            "\"$HOME\"/.ssh-app/sandboxes/demo-1/run/decisions:/ssh-app/run/decisions:ro",
            "\"$HOME\"/.ssh-app/allowed_signers:/ssh-app/allowed_signers:ro",
        ]
    );
    assert!(
        !plan.run_command.contains("work/api"),
        "the project folder is not mounted"
    );
    // Decisions and the key are read-only: the agent cannot approve itself.
    assert!(plan
        .run_command
        .contains("decisions:/ssh-app/run/decisions:ro"));
}

#[test]
fn auto_mode_drops_the_phone_hook_mounts_and_adds_the_agents_flag() {
    let flags = [
        (
            SandboxAgent::ClaudeCode,
            "claude --dangerously-skip-permissions",
        ),
        (
            SandboxAgent::Antigravity,
            "agy --dangerously-skip-permissions",
        ),
        (
            SandboxAgent::Codex,
            "codex --dangerously-bypass-approvals-and-sandbox",
        ),
        (SandboxAgent::Grok, "grok --always-approve"),
    ];
    for (agent, tail) in flags {
        let mut p = profile(agent);
        p.auto_approve = true;
        let run = plan_sandbox(&p).unwrap().run_command;
        assert!(run.ends_with(tail), "{run}");
        assert!(!run.contains("allowed_signers") && !run.contains("SSH_APP_HOME"));
        let ask = plan_sandbox(&profile(agent)).unwrap().run_command;
        assert!(!ask.contains("--always-approve"));
        // Only Codex gets a hook-trust bypass in Ask mode (our hook, written by the installer).
        assert_eq!(
            ask.contains("--dangerously-bypass-hook-trust"),
            agent == SandboxAgent::Codex,
            "{ask}"
        );
        assert!(!ask.contains("--dangerously-skip-permissions"));
        assert!(!ask.contains("--dangerously-bypass-approvals-and-sandbox"));
    }
}

#[test]
fn online_uses_an_internal_network_and_the_proxy_offline_has_no_network() {
    let online = plan_sandbox(&profile(SandboxAgent::Codex)).unwrap();
    assert!(online
        .run_command
        .contains("--network ssh-app-sbx-demo-1-int"));
    assert!(online
        .run_command
        .contains("HTTPS_PROXY='http://ssh-app-sbx-demo-1-proxy:3128'"));
    let setup = online.setup_commands.join("\n");
    assert!(setup.contains("network create --internal"));
    assert!(setup.contains("network connect ssh-app-sbx-demo-1-int ssh-app-sbx-demo-1-proxy"));
    assert!(online.proxy_filter.contains("^api\\.openai\\.com$"));
    assert!(
        online.proxy_filter.contains("^github\\.com$"),
        "the user's extra host"
    );

    let mut p = profile(SandboxAgent::Codex);
    p.network = SandboxNetwork::Offline;
    let off = plan_sandbox(&p).unwrap();
    assert!(off.run_command.contains("--network none"));
    assert!(!off.run_command.contains("PROXY"));
    assert_eq!(off.setup_commands.len(), 1, "no networks, no proxy");
}

#[test]
fn the_clone_takes_committed_content_and_the_fetch_stays_in_its_namespace() {
    let plan = plan_sandbox(&profile(SandboxAgent::ClaudeCode)).unwrap();
    assert!(plan.clone_command.contains("bundle create - HEAD"));
    assert!(plan.clone_command.contains("--network none"));
    assert!(!plan.clone_command.contains("cp -r"));
    assert!(plan.fetch_command.contains("transfer.fsckObjects=true"));
    assert!(plan.fetch_command.contains("git bundle verify"));
    assert!(plan
        .fetch_command
        .contains("'refs/heads/*:refs/heads/sandbox/demo-1/*'"));
    assert!(plan
        .fetch_command
        .contains("-v ssh-app-sbx-demo-1-work:/work:ro"));
    // Cleanup never removes the sandbox home (logins and transcripts).
    let cleanup = plan.cleanup_commands.join(" ");
    assert!(!cleanup.contains("demo-1/home") && !cleanup.contains("rm -rf"));
}

#[test]
fn hostile_profiles_are_refused() {
    let bad_folders = [
        "",
        "/",
        "~",
        "~/",
        "/Users/me",
        "/home/me",
        "/usr",
        "relative/dir",
        "~/a/../b",
        "/srv/a\nb",
        "~/.ssh",
        "/Users/me/proj/.ssh/keys",
        "~/.aws/x",
        "~/.claude/projects",
        "~/.config/gh",
    ];
    for f in bad_folders {
        let mut p = profile(SandboxAgent::Grok);
        p.project_folder = f.into();
        assert!(plan_sandbox(&p).is_err(), "{f:?}");
    }
    for id in [
        "",
        "ab",
        "UP",
        "a b",
        "a;b",
        "-ab",
        "ab-",
        "../x",
        &"a".repeat(33),
    ] {
        let mut p = profile(SandboxAgent::Grok);
        p.sandbox_id = id.into();
        assert!(plan_sandbox(&p).is_err(), "{id:?}");
    }
    for h in [
        "",
        "localhost",
        "a b.com",
        "evil.com;rm",
        "-x.com",
        "a..com",
        "*.",
        "$(x).com",
    ] {
        let mut p = profile(SandboxAgent::Grok);
        p.allow_hosts = vec![h.into()];
        assert!(plan_sandbox(&p).is_err(), "{h:?}");
    }
    for img in [
        "debian",
        "debian:12",
        "debian@sha256:abc",
        "de bian@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdeG",
        "x;y@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    ] {
        let mut p = profile(SandboxAgent::Grok);
        p.base_image = img.into();
        assert!(plan_sandbox(&p).is_err(), "{img:?}");
    }
    for (cpus, mem, pids) in [
        (0, 2048, 256),
        (99, 2048, 256),
        (2, 1, 256),
        (2, 2048, 1),
        (2, 2048, 99999),
    ] {
        let mut p = profile(SandboxAgent::Grok);
        (p.cpus, p.memory_mb, p.pids) = (cpus, mem, pids);
        assert!(plan_sandbox(&p).is_err());
    }
}

#[test]
fn quotes_and_substitutions_in_a_folder_name_stay_data() {
    let mut p = profile(SandboxAgent::ClaudeCode);
    p.project_folder = "/srv/app's $(touch pwned) `x`".into();
    let plan = plan_sandbox(&p).unwrap();
    assert!(plan
        .clone_command
        .contains("'/srv/app'\\''s $(touch pwned) `x`'"));
    for c in all_commands(&plan) {
        assert!(std::process::Command::new("sh")
            .args(["-n", "-c", &c])
            .status()
            .unwrap()
            .success());
    }
    // And when the command is really run with a stub `git` and `docker`, nothing is created.
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for stub in ["git", "docker"] {
        std::fs::write(
            bin.join(stub),
            "#!/bin/sh\ncat >/dev/null 2>&1 </dev/null\nexit 0\n",
        )
        .unwrap();
        std::fs::set_permissions(
            bin.join(stub),
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
    }
    let marker = dir.path().join("pwned");
    let cmd = plan.clone_command.replace(DOCKER_PATH, "true");
    let status = std::process::Command::new("sh")
        .current_dir(dir.path())
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .args(["-c", &cmd])
        .status()
        .unwrap();
    assert!(status.success());
    assert!(!marker.exists() && !std::path::Path::new("pwned").exists());
}

#[test]
fn the_image_recipe_is_pinned_runs_as_a_user_and_changes_the_tag_when_it_changes() {
    let a = plan_sandbox(&profile(SandboxAgent::ClaudeCode)).unwrap();
    assert!(a.dockerfile.starts_with(&format!("FROM {DIGEST}")));
    assert!(a.dockerfile.contains("USER agent"));
    assert!(a
        .dockerfile
        .contains("npm install -g @anthropic-ai/claude-code"));
    let b = plan_sandbox(&profile(SandboxAgent::Codex)).unwrap();
    assert_ne!(a.image_tag, b.image_tag);
    assert!(b.dockerfile.contains("@openai/codex"));
    let g = plan_sandbox(&profile(SandboxAgent::Grok)).unwrap();
    assert!(g.dockerfile.contains("https://x.ai/cli/install.sh"));
    let y = plan_sandbox(&profile(SandboxAgent::Antigravity)).unwrap();
    assert!(y
        .dockerfile
        .contains("https://antigravity.google/cli/install.sh"));
    assert_eq!(
        a.image_tag,
        plan_sandbox(&profile(SandboxAgent::ClaudeCode))
            .unwrap()
            .image_tag
    );
}

#[test]
fn proxy_filter_regexes_are_anchored() {
    let f = proxy_filter(&["api.x.ai".into(), "*.googleapis.com".into()]);
    assert_eq!(f, "^api\\.x\\.ai$\n(^|\\.)googleapis\\.com$");
}

#[test]
fn review_flags_the_files_the_host_would_run_later() {
    let changed: Vec<String> = [
        "src/lib.rs",
        ".git/hooks/pre-commit",
        ".claude/settings.local.json",
        "Makefile",
        "pkg/package.json",
        ".github/workflows/ci.yml",
        "scripts/deploy.sh",
        "README.md",
        ".envrc",
        "./.vscode/tasks.json",
        "sub/.git/config",
    ]
    .map(String::from)
    .to_vec();
    let flagged = implicit_execution_paths(&changed);
    assert_eq!(flagged.len(), changed.len() - 2, "{flagged:?}");
    assert!(!flagged.contains(&"src/lib.rs".to_string()));
    assert!(!flagged.contains(&"README.md".to_string()));
}

/// Random hostile profiles: whatever the planner accepts yields commands the shell parses, and
/// nothing it refuses panics. (Deterministic generator; no rand dependency.)
#[test]
fn fuzzed_profiles_never_panic_and_accepted_ones_stay_parseable() {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let frags = [
        "/srv",
        "/",
        "~/",
        "..",
        "a",
        "b c",
        "'",
        "\"",
        "$(x)",
        "`x`",
        ";",
        "&&",
        "|",
        "\n",
        "\\",
        "*",
        "-",
        ".ssh",
        "app",
        "é",
        "\0",
        "--privileged",
        "%s",
        "{}",
        "#",
        ">",
        "$HOME",
    ];
    let (mut accepted, mut refused) = (0, 0);
    for i in 0..400 {
        let mut gen = |n: u64| -> String {
            (0..(next() % n + 1))
                .map(|_| frags[(next() % frags.len() as u64) as usize])
                .collect()
        };
        let mut p = profile(AGENTS[i % 4]);
        p.project_folder = format!("/srv/{}", gen(4));
        p.sandbox_id = if i % 3 == 0 {
            gen(2)
        } else {
            format!("fz-{}", i)
        };
        p.allow_hosts = if i % 2 == 0 {
            vec!["ok.example.com".into()]
        } else {
            vec![format!("{}.com", gen(2))]
        };
        match plan_sandbox(&p) {
            Ok(plan) => {
                accepted += 1;
                for c in all_commands(&plan) {
                    let ok = std::process::Command::new("sh")
                        .args(["-n", "-c", &c])
                        .status()
                        .unwrap()
                        .success();
                    assert!(ok, "{p:?}\n{c}");
                }
            }
            Err(_) => refused += 1,
        }
    }
    assert!(accepted > 20 && refused > 20, "{accepted}/{refused}");
}

#[test]
fn the_default_base_image_is_pinned_and_valid() {
    let mut p = profile(SandboxAgent::ClaudeCode);
    p.base_image = DEFAULT_BASE_IMAGE.into();
    assert!(plan_sandbox(&p).is_ok());
    assert_eq!(sandbox_default_base_image(), DEFAULT_BASE_IMAGE);
}

#[test]
fn hook_config_dirs_are_inside_the_sandbox_home() {
    for agent in AGENTS {
        let d = sandbox_hook_config_dir(&profile(agent)).unwrap();
        assert!(d.starts_with("~/.ssh-app/sandboxes/demo-1/home/"), "{d}");
        assert!(crate::agents::agent_hooks_config_path(
            match agent {
                SandboxAgent::ClaudeCode => crate::agents::AgentKind::ClaudeCode,
                SandboxAgent::Antigravity => crate::agents::AgentKind::Antigravity,
                SandboxAgent::Codex => crate::agents::AgentKind::Codex,
                SandboxAgent::Grok => crate::agents::AgentKind::Grok,
            },
            Some(d)
        )
        .is_ok());
    }
}

#[test]
fn sandbox_hooks_point_at_the_script_inside_the_image_not_at_the_host() {
    use crate::agents::{default_hook_preset, plan_agent_hooks, AgentKind};
    for kind in [
        AgentKind::ClaudeCode,
        AgentKind::Antigravity,
        AgentKind::Codex,
        AgentKind::Grok,
    ] {
        let planned =
            plan_agent_hooks(kind, default_hook_preset(), Some(SANDBOX_HOOK_DIR.into())).unwrap();
        assert!(!planned.is_empty());
        for h in planned {
            assert!(
                h.command.contains("\"/usr/local/bin/ssh-app-hook\""),
                "{}",
                h.command
            );
            assert!(!h.command.contains(".ssh-app/bin"), "{}", h.command);
        }
    }
}

#[test]
fn egress_log_lines_become_plain_host_names_only() {
    let log = "INFO Proxying refused on filtered domain \"Example.COM\"\n\
               INFO Proxying refused on filtered domain \"evil.com\"\n\
               INFO Proxying refused on filtered domain \"evil.com\"\n\
               INFO Proxying refused on filtered domain \"1.2.3.4\"\n\
               INFO Proxying refused on filtered domain \"*.wild.com\"\n\
               INFO Proxying refused on filtered domain \"a b.com\"\n\
               INFO Proxying refused on filtered domain \"x.com\\u001b[31m\"\n\
               INFO Proxying refused on filtered domain \"api.anthropic.com\"\n\
               INFO Proxying refused on filtered domain \"open";
    let allowed = vec!["*.anthropic.com".to_string()];
    assert_eq!(
        parse_denied_hosts(log, &allowed),
        vec!["evil.com".to_string(), "example.com".to_string()]
    );
    let many: String = (0..100)
        .map(|i| format!("refused on filtered domain \"h{i}.example.org\"\n"))
        .collect();
    assert_eq!(parse_denied_hosts(&many, &[]).len(), 20);
}

#[test]
fn the_filter_reads_back_and_allow_commands_take_one_plain_host() {
    let filter = proxy_filter(&["api.x.ai".into(), "*.googleapis.com".into()]);
    assert_eq!(
        parse_allowed_hosts(&filter),
        vec!["api.x.ai".to_string(), "*.googleapis.com".to_string()]
    );
    let p = profile(SandboxAgent::Grok);
    for bad in [
        "*.com",
        "a",
        "x.com; rm -rf /",
        "x.com\nevil.com",
        "$(id).com",
        "",
        "a..com",
    ] {
        assert!(egress_allow_command(&p, bad).is_err(), "{bad:?}");
    }
    let cmd = egress_allow_command(&p, "Cdn.Example.com").unwrap();
    assert!(cmd.contains("^cdn\\.example\\.com$") && cmd.contains("kill -s USR1"));
    assert!(sh_parses(&cmd));
    assert!(sh_parses(&egress_log_command(&p).unwrap()));
}

fn sh_parses(c: &str) -> bool {
    std::process::Command::new("sh")
        .args(["-n", "-c", c])
        .status()
        .unwrap()
        .success()
}

#[test]
fn removing_an_entry_rewrites_the_filter_in_place_and_takes_one_plain_pattern() {
    let p = profile(SandboxAgent::Grok);
    for bad in ["*.com", "x.com; id", "$(id).com", "x.com\nevil.com", ""] {
        assert!(egress_remove_command(&p, bad).is_err(), "{bad:?}");
    }
    let cmd = egress_remove_command(&p, "*.googleapis.com").unwrap();
    assert!(sh_parses(&cmd));
    assert!(cmd.contains("cat \"$t\" > \"$f\"") && !cmd.contains("mv "));
    assert!(egress_allow_command(&p, "*.cdn.example.com").is_ok());
}

#[test]
fn the_login_file_is_copied_once_by_cp_and_never_through_a_link() {
    use std::os::unix::fs::symlink;
    let d = tempfile::tempdir().unwrap();
    let home = d.path();
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::write(home.join(".codex/auth.json"), "TOKEN-A").unwrap();
    let mut p = profile(SandboxAgent::Codex);
    p.sandbox_id = "login-1".into();
    let cmd = plan_sandbox(&p).unwrap().login_command;
    assert!(sh_parses(&cmd) && !cmd.contains(" -r") && !cmd.contains("-a "));
    let run = |c: &str| {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(c)
            .env("HOME", home)
            .status()
            .unwrap()
            .success()
    };
    let dest = home.join(".ssh-app/sandboxes/login-1/home/.codex/auth.json");
    assert!(run(&cmd));
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "TOKEN-A");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777,
        0o600
    );
    // A refreshed sandbox copy is kept.
    std::fs::write(&dest, "REFRESHED").unwrap();
    assert!(run(&cmd));
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "REFRESHED");
    // A link at the source is not followed.
    std::fs::remove_file(&dest).unwrap();
    std::fs::remove_file(home.join(".codex/auth.json")).unwrap();
    std::fs::write(home.join("elsewhere"), "SECRET").unwrap();
    symlink(home.join("elsewhere"), home.join(".codex/auth.json")).unwrap();
    assert!(run(&cmd));
    assert!(!dest.exists());
    // Off means no command at all.
    p.copy_login = false;
    assert!(plan_sandbox(&p).unwrap().login_command.is_empty());
}

#[test]
fn claude_login_comes_from_the_configured_folder_and_hostile_folders_are_refused() {
    let mut p = profile(SandboxAgent::ClaudeCode);
    p.claude_config_dir = "~/.claude-personal".into();
    let cmd = plan_sandbox(&p).unwrap().login_command;
    assert!(cmd.contains(".claude-personal") && cmd.contains(".credentials.json"));
    assert!(sh_parses(&cmd));
    for bad in ["~/a/../b", "/x\ny", "/x\u{7}"] {
        p.claude_config_dir = bad.into();
        assert!(plan_sandbox(&p).is_err(), "{bad:?}");
    }
    p.claude_config_dir = "~/a b'$(id)`x`".into();
    let cmd = plan_sandbox(&p).unwrap().login_command;
    assert!(sh_parses(&cmd));
}

#[test]
fn review_lists_changed_files_for_this_sandboxs_branches_only() {
    let mut p = profile(SandboxAgent::ClaudeCode);
    p.sandbox_id = "sbx-abc12345".into();
    let cmd = review_files_command(&p, "sandbox/sbx-abc12345/sandbox-work").unwrap();
    assert!(cmd.contains("merge-base HEAD 'refs/heads/sandbox/sbx-abc12345/sandbox-work'"));
    assert!(cmd.contains("--no-ext-diff --no-textconv --name-only"));
    for bad in [
        "main",
        "sandbox/other-id/x",
        "sandbox/sbx-abc12345/",
        "sandbox/sbx-abc12345/../main",
        "sandbox/sbx-abc12345/a b",
        "sandbox/sbx-abc12345/x;id",
        "sandbox/sbx-abc12345/$(id)",
        "sandbox/sbx-abc12345/-x/",
    ] {
        assert!(review_files_command(&p, bad).is_err(), "{bad}");
    }
}

#[test]
fn review_counts_files_and_flags_what_the_host_would_run() {
    let out = "src/main.rs\n.github/workflows/ci.yml\n\nscripts/run.sh\nREADME.md\n";
    let r = parse_review_files("sandbox/x/y", out);
    assert_eq!(r.changed, 4);
    assert_eq!(r.flagged, [".github/workflows/ci.yml", "scripts/run.sh"]);
    let many: String = (0..80).map(|i| format!(".husky/h{i}\n")).collect();
    assert_eq!(parse_review_files("b", &many).flagged.len(), 50);
}

#[test]
fn the_review_command_lists_a_branchs_files_in_a_real_repo() {
    use std::process::Command;
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("proj");
    std::fs::create_dir(&repo).unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .current_dir(&repo)
            .env_remove("GIT_DIR")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "a").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "one"]);
    git(&["checkout", "-q", "-b", "sandbox/demo-1/sandbox-work"]);
    std::fs::create_dir_all(repo.join(".github/workflows")).unwrap();
    std::fs::write(repo.join(".github/workflows/ci.yml"), "x").unwrap();
    std::fs::write(repo.join("a.txt"), "changed").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "two"]);
    git(&["checkout", "-q", "main"]);

    let mut p = profile(SandboxAgent::ClaudeCode);
    p.project_folder = repo.to_str().unwrap().to_string();
    let cmd = review_files_command(&p, "sandbox/demo-1/sandbox-work").unwrap();
    let out = Command::new("sh").args(["-c", &cmd]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        out.status.success(),
        "{text} {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let r = parse_review_files("sandbox/demo-1/sandbox-work", &text);
    assert_eq!(r.changed, 2, "{text}");
    assert_eq!(r.flagged, [".github/workflows/ci.yml"]);
}
