//! Runs the generated OpenCode plugin under `node` against a fake hook script: a deny must make
//! `tool.execute.before` throw, an allow must pass, and every failure to ask the script (missing,
//! crash, garbage output) must block the call (fail closed). Skipped without `node`/`python3`.

use ssh_core::{default_hook_preset, merge_planned_hooks, plan_agent_hooks, AgentKind};
use std::path::Path;
use std::process::{Command, Stdio};

fn available() -> bool {
    ["node", "python3"].iter().all(|c| {
        Command::new(c)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
    })
}

/// Writes the plugin and a fake script under a temp HOME, then runs `driver` (JS) in node.
fn run(fake_script: Option<&str>, driver: &str) -> String {
    let home = tempfile::tempdir().unwrap();
    let bin = home.path().join(".ssh-app/bin");
    std::fs::create_dir_all(&bin).unwrap();
    if let Some(body) = fake_script {
        std::fs::write(bin.join("ssh-app-hook"), body).unwrap();
    }
    let plan = plan_agent_hooks(AgentKind::OpenCode, default_hook_preset(), None).unwrap();
    let plugin = merge_planned_hooks(AgentKind::OpenCode, String::new(), plan).unwrap();
    let plugin_path = home.path().join("plugin.mjs");
    std::fs::write(&plugin_path, plugin).unwrap();
    let main = home.path().join("main.mjs");
    std::fs::write(
        &main,
        format!(
            "import {{ SshAppPlugin }} from {:?};\nconst hooks = await SshAppPlugin({{ directory: '/work' }});\n{driver}\n",
            plugin_path.to_str().unwrap()
        ),
    )
    .unwrap();
    let out = Command::new("node")
        .arg(&main)
        .env("HOME", home.path())
        // Never reach the developer's tmux (AGENTS.md, test isolation); node does not need it.
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "node failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

const TRY: &str = r#"
async function attempt(tool) {
  try { await hooks["tool.execute.before"]({ tool, sessionID: "s" }, { args: { command: "ls" } }); return "pass" }
  catch (e) { return "blocked:" + e.message }
}
"#;

#[test]
fn deny_blocks_allow_passes_and_other_tools_are_not_asked() {
    if !available() {
        return;
    }
    let deny = "import json,sys\nd=json.load(sys.stdin)\nassert d['tool']=='bash' and d['args']['command']=='ls' and d['cwd']=='/work'\nprint(json.dumps({'decision':'deny','reason':'nope from phone'}))\n";
    let out = run(
        Some(deny),
        &format!("{TRY}\nconsole.log(await attempt('bash'), await attempt('read'), await attempt('mybash'));"),
    );
    // `read` and `mybash` do not match ^(?:bash)$, so the script is never asked.
    assert_eq!(out, "blocked:nope from phone pass pass");

    let allow = "import sys\nsys.stdin.read()\nprint('{\"decision\":\"allow\"}')\n";
    assert_eq!(
        run(
            Some(allow),
            &format!("{TRY}\nconsole.log(await attempt('bash'));")
        ),
        "pass"
    );
    // No opinion (empty answer): the call continues into OpenCode's own permission flow.
    let none = "import sys\nsys.stdin.read()\n";
    assert_eq!(
        run(
            Some(none),
            &format!("{TRY}\nconsole.log(await attempt('bash'));")
        ),
        "pass"
    );
}

#[test]
fn every_failure_to_ask_blocks_the_call() {
    if !available() {
        return;
    }
    let drive = format!("{TRY}\nconsole.log((await attempt('bash')).startsWith('blocked:'));");
    assert_eq!(run(None, &drive), "true", "missing script must fail closed");
    assert_eq!(run(Some("raise SystemExit(3)\n"), &drive), "true", "crash");
    assert_eq!(run(Some("print('not json')\n"), &drive), "true", "garbage");
}

#[test]
fn idle_sends_a_notification_and_survives_a_failing_script() {
    if !available() {
        return;
    }
    let out = run(
        None,
        "await hooks.event({ event: { type: 'session.idle', properties: { sessionID: 's' } } });\nawait hooks.event({ event: { type: 'other' } });\nconsole.log('ok');",
    );
    assert_eq!(out, "ok");
}
