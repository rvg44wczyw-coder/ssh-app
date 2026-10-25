use super::*;

#[test]
fn attachment_command_stores_the_stdin_bytes() {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    let home = std::env::temp_dir().join(format!("att-home-{}", rand::random::<u64>()));
    std::fs::create_dir_all(&home).unwrap();
    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg(attachment_command("0a1b2c3d-photo.png", true, 8))
        .env("HOME", &home)
        .env_remove("TMUX")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"\x89PNGdata")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let path = parse_attachment_path(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"\x89PNGdata");
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert!(!std::path::Path::new(&format!("{path}.part")).exists());
    // temporary mode: private folder under TMPDIR, outside HOME, mode 0700
    let tmp = home.join("tmp");
    std::fs::create_dir_all(&tmp).unwrap();
    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg(attachment_command("0a1b2c3e-new.png", false, 7))
        .env("HOME", &home)
        .env("TMPDIR", format!("{}/", tmp.display()))
        .env_remove("TMUX")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"tmpdata").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let tpath = parse_attachment_path(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert!(tpath.starts_with(tmp.to_str().unwrap()), "{tpath}");
    assert_eq!(std::fs::read(&tpath).unwrap(), b"tmpdata");
    let dir_mode = std::fs::metadata(std::path::Path::new(&tpath).parent().unwrap())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(dir_mode, 0o700);
    // a link in place of the folder is refused
    let evil = home.join("evil");
    std::fs::create_dir_all(&evil).unwrap();
    let uid = String::from_utf8(
        std::process::Command::new("id")
            .arg("-u")
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let link = tmp.join(format!("ssh-app-{}", uid.trim()));
    std::fs::remove_dir_all(&link).unwrap();
    std::os::unix::fs::symlink(&evil, &link).unwrap();
    let refused = std::process::Command::new("sh")
        .arg("-c")
        .arg(attachment_command("x-y.png", false, 0))
        .env("HOME", &home)
        .env("TMPDIR", &tmp)
        .env_remove("TMUX")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert_eq!(std::fs::read_dir(&evil).unwrap().count(), 0);
    std::fs::remove_dir_all(&home).unwrap();
}

#[test]
fn a_cut_upload_leaves_no_file() {
    use std::io::Write;
    let home = std::env::temp_dir().join(format!("att-cut-{}", rand::random::<u64>()));
    std::fs::create_dir_all(&home).unwrap();
    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg(attachment_command("0a1b2c3f-cut.png", true, 100))
        .env("HOME", &home)
        .env_remove("TMUX")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // 10 of 100 bytes, then the channel closes (cancelled, or the network went away)
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"0123456789")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(!out.status.success());
    assert_eq!(
        parse_attachment_path(&String::from_utf8_lossy(&out.stdout)),
        None
    );
    assert_eq!(
        std::fs::read_dir(home.join(".ssh-app/inbox"))
            .unwrap()
            .count(),
        0,
        "neither the file nor its .part may stay"
    );
    std::fs::remove_dir_all(&home).unwrap();
}

#[test]
fn attachment_names_are_safe() {
    assert_eq!(attachment_name("IMG_0001.HEIC"), "IMG_0001.HEIC");
    assert_eq!(attachment_name("../../etc/passwd"), "passwd");
    assert_eq!(attachment_name("a b'c\"d;$(x).png"), "a_b_c_d___x_.png");
    assert_eq!(attachment_name(".hidden"), "hidden");
    assert_eq!(attachment_name("   "), "___");
    assert_eq!(attachment_name(""), "file");
    let long = format!("{}.mov", "x".repeat(200));
    let n = attachment_name(&long);
    assert_eq!(n.chars().count(), 80);
    assert!(n.ends_with(".mov"));
}

#[test]
fn attachment_command_and_path_parsing() {
    assert!(attachment_command("a-b.png", true, 1).contains(".ssh-app/inbox"));
    assert!(attachment_command("a-b.png", false, 1).contains("ssh-app-$(id -u)"));
    let c = attachment_command("0a1b2c3d-photo.png", true, 1);
    assert!(c.starts_with("sh -c '"));
    assert!(c.contains("0a1b2c3d-photo.png"));
    assert!(!c.contains("'0a1b"));
    assert_eq!(
        parse_attachment_path("noise\nATTACH_PATH=/Users/me/.ssh-app/inbox/x.png\n"),
        Some("/Users/me/.ssh-app/inbox/x.png".to_string())
    );
    assert_eq!(parse_attachment_path("ATTACH_PATH=relative"), None);
    assert_eq!(parse_attachment_path(""), None);
}

#[test]
fn config_write_refuses_when_the_file_changed_since_it_was_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    let p = path.to_str().unwrap();
    let run = |prev: &str, new: &str| {
        let out = std::process::Command::new("sh")
            .args(["-c", &build_write_config_command(p, prev, new)])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    // New file: the expected previous content is empty.
    assert_eq!(run("", "{\"a\":1}"), "CONFIG_WRITTEN");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\":1}");
    // Someone edits the file after we read it: nothing is written, no temp file is left.
    std::fs::write(&path, "{\"a\":2}").unwrap();
    assert_eq!(run("{\"a\":1}", "{\"b\":1}"), "CONFIG_CHANGED");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\":2}");
    assert!(!dir.path().join("settings.json.ssh-app.tmp").exists());
    // Unchanged since the read: written.
    assert_eq!(run("{\"a\":2}", "{\"b\":1}"), "CONFIG_WRITTEN");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"b\":1}");
}

/// Phase 0 of the universal agent feed: what the host's agent detection says for recorded `ps` outputs.
/// The head of the locate script (up to the `kind=` line) is run with a fake `ps`; pane pid 100.
/// Codex and Grok are not detected today: those cases change on purpose when their adapters land.
mod detection_golden {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    fn detect(ps_rows: &str) -> String {
        let body = session_transcript_script();
        let end = body.find("kind=${found%% *}").expect("detection head");
        let head = &body[..end];
        let dir = tempfile::tempdir().unwrap();
        let ps = dir.path().join("ps");
        std::fs::write(&ps, format!("#!/bin/sh\ncat <<'ROWS'\n{ps_rows}\nROWS\n")).unwrap();
        std::fs::set_permissions(&ps, std::fs::Permissions::from_mode(0o755)).unwrap();
        let script = format!("roots=100\n{head}\nprintf '%s' \"$found\"\n");
        let out = Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .env_clear()
            .env("PATH", format!("{}:/usr/bin:/bin", dir.path().display()))
            .output()
            .expect("sh");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    #[test]
    fn claude_agy_and_wrappers_are_found_below_the_pane_process() {
        assert_eq!(detect("100 1 -zsh\n101 100 claude --resume"), "claude 101");
        assert_eq!(
            detect("100 1 -zsh\n101 100 node /opt/x/node_modules/@anthropic-ai/claude-code/cli.js"),
            "claude 101"
        );
        assert_eq!(detect("100 1 -zsh\n101 100 /usr/local/bin/agy"), "agy 101");
        assert_eq!(
            detect("100 1 -zsh\n101 100 /home/u/.local/antigravity/bin/agent --x"),
            "agy 101"
        );
        assert_eq!(
            detect("100 1 -zsh\n101 100 zsh\n102 101 claude"),
            "claude 102"
        );
    }

    #[test]
    fn other_programs_and_agents_not_connected_yet_are_not_found() {
        assert_eq!(detect("100 1 -zsh\n101 100 vim notes.txt"), "");
        assert_eq!(detect("100 1 -zsh\n101 100 codex"), "");
        assert_eq!(detect("100 1 -zsh\n101 100 grok"), "");
        // Not below the pane process: another terminal's claude is ignored.
        assert_eq!(detect("100 1 -zsh\n200 1 -zsh\n201 200 claude"), "");
    }
}

/// Phase 2 of the universal agent feed: the locate script is built from the agent registry and the
/// adapters' arms, and was byte-identical to the hand-written script it replaced until the hacker review (2026-10-25: literal tab-name match, agy folder confined to its own brain/).
#[test]
fn the_session_locate_script_is_frozen() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/snap/session_locate_script.snap");
    let now = session_transcript_script();
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, &now).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).expect("snapshot (UPDATE_GOLDEN=1 writes it)");
    assert!(
        want == now,
        "the locate script changed:\n--- want\n{want}\n--- got\n{now}"
    );
}

/// Phase 2: the whole locate script run for real with a fake `ps` and `lsof` and a temporary `HOME` holding
/// the files of Claude and Antigravity. What it prints is the session the feed will read.
mod locate_end_to_end {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    struct Host {
        home: tempfile::TempDir,
        bin: tempfile::TempDir,
    }

    impl Host {
        fn new(ps_rows: &str, cwd: &str) -> Host {
            let home = tempfile::tempdir().unwrap();
            let bin = tempfile::tempdir().unwrap();
            let stub = |name: &str, body: String| {
                let p = bin.path().join(name);
                std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            };
            stub(
                "ps",
                format!("case \"$1\" in -Ao) cat <<'ROWS'\n{ps_rows}\nROWS\n;; esac"),
            );
            stub("lsof", format!("echo n{cwd}"));
            Host { home, bin }
        }

        fn file(&self, rel: &str, age_secs: u64) -> String {
            let p = self.home.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "{}\n").unwrap();
            let t = std::time::SystemTime::now() - std::time::Duration::from_secs(age_secs);
            std::fs::File::options()
                .write(true)
                .open(&p)
                .unwrap()
                .set_modified(t)
                .unwrap();
            p.to_string_lossy().into_owned()
        }

        fn note_session(&self, tmux: &str, agent: &str, session: &str) {
            let rel = format!(".ssh-app/run/sessions/{session}.json");
            let p = self.home.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(
                p,
                format!("{{\"tmux\":\"{tmux}\",\"agent\":\"{agent}\",\"session\":\"{session}\",\"ts\":5}}"),
            )
            .unwrap();
        }

        fn locate(&self) -> String {
            self.locate_tab("sess")
        }

        fn locate_tab(&self, tab: &str) -> String {
            let script = format!(
                "roots=100\nset -- {tab} tmux\n{}",
                session_transcript_script()
            );
            let out = Command::new("/bin/sh")
                .arg("-c")
                .arg(script)
                .env_clear()
                .env("HOME", self.home.path())
                .env(
                    "PATH",
                    format!("{}:/usr/bin:/bin", self.bin.path().display()),
                )
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        }
    }

    #[test]
    fn claude_by_the_session_id_the_hooks_noted() {
        let h = Host::new("100 1 -zsh\n101 100 claude", "/tmp/project");
        let want = h.file(".claude/projects/-tmp-project/abc123.jsonl", 100);
        h.file(".claude/projects/-tmp-project/newer.jsonl", 1);
        h.note_session("sess", "claude", "abc123");
        assert_eq!(h.locate(), want);
    }

    #[test]
    fn claude_without_a_noted_id_takes_the_newest_file_of_the_folder() {
        let h = Host::new("100 1 -zsh\n101 100 claude", "/tmp/project");
        h.file(".claude/projects/-tmp-project/old.jsonl", 500);
        let want = h.file(".claude/projects/-tmp-project/new.jsonl", 5);
        h.file(".claude/projects/-other/other.jsonl", 1);
        assert_eq!(h.locate(), want);
    }

    #[test]
    fn claude_in_a_second_account_directory_is_found() {
        let h = Host::new("100 1 -zsh\n101 100 claude", "/tmp/project");
        let want = h.file(".claude-personal/projects/-tmp-project/s1.jsonl", 3);
        assert_eq!(h.locate(), want);
    }

    #[test]
    fn agy_by_the_noted_id_else_the_newest_conversation() {
        let h = Host::new("100 1 -zsh\n101 100 agy", "/tmp/project");
        let old = h.file(
            ".gemini/antigravity-cli/brain/aaa/.system_generated/logs/transcript.jsonl",
            900,
        );
        let new = h.file(
            ".gemini/antigravity-cli/brain/bbb/.system_generated/logs/transcript.jsonl",
            9,
        );
        assert_eq!(h.locate(), new);
        h.note_session("sess", "antigravity", "aaa");
        assert_eq!(h.locate(), old);
    }

    /// Hacker finding 1: the tab name is text, not a pattern (`.` is allowed in tab names).
    #[test]
    fn a_tab_name_is_not_a_pattern() {
        let h = Host::new("100 1 -zsh\n101 100 claude", "/x/y");
        h.file(".claude/projects/-tmp-project/victim.jsonl", 5);
        h.note_session("abc", "claude", "victim");
        assert_eq!(h.locate_tab("a.c"), "");
        assert!(h.locate_tab("abc").ends_with("victim.jsonl"));
    }

    /// Hacker finding 2: a project the agent has open can hold a `brain/<x>/.system_generated/...` of its own;
    /// only the agent's own folder counts.
    #[test]
    fn agy_ignores_a_brain_folder_outside_its_own() {
        let repo = tempfile::tempdir().unwrap();
        let evil = repo.path().join("brain/evil/.system_generated/logs");
        std::fs::create_dir_all(&evil).unwrap();
        std::fs::write(evil.join("transcript.jsonl"), "{}\n").unwrap();
        let cwd = repo.path().join("brain/evil");
        let h = Host::new("100 1 -zsh\n101 100 agy", &cwd.to_string_lossy());
        let real = h.file(
            ".gemini/antigravity-cli/brain/real/.system_generated/logs/transcript.jsonl",
            9,
        );
        assert_eq!(h.locate(), real);
        // Without a conversation of its own nothing is printed, the repository's file is never offered.
        let bare = Host::new("100 1 -zsh\n101 100 agy", &cwd.to_string_lossy());
        assert_eq!(bare.locate(), "");
    }

    #[test]
    fn nothing_is_printed_when_no_agent_runs_or_no_file_exists() {
        let none = Host::new("100 1 -zsh\n101 100 vim", "/tmp/project");
        none.file(".claude/projects/-tmp-project/s.jsonl", 3);
        assert_eq!(none.locate(), "");
        let empty = Host::new("100 1 -zsh\n101 100 claude", "/tmp/project");
        assert_eq!(empty.locate(), "");
    }
}
