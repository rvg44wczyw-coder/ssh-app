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
/// The head of `SESSION_TRANSCRIPT_BODY` (up to the `kind=` line) is run with a fake `ps`; pane pid 100.
/// Codex and Grok are not detected today: those cases change on purpose when their adapters land.
mod detection_golden {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    fn detect(ps_rows: &str) -> String {
        let end = SESSION_TRANSCRIPT_BODY
            .find("kind=${found%% *}")
            .expect("detection head");
        let head = &SESSION_TRANSCRIPT_BODY[..end];
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
