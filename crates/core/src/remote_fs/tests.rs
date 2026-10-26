use super::*;

#[cfg(test)]
mod client_tests {
    use super::*;

    #[test]
    fn paths_are_cleaned_or_refused() {
        assert_eq!(clean_path("/a//b/").unwrap(), "/a/b");
        assert_eq!(clean_path("/").unwrap(), "/");
        for bad in [
            "a/b",
            "",
            "/a/../b",
            "/a/./b",
            "/a\nb",
            "/a\0b",
            &format!("/{}", "x".repeat(5000)),
        ] {
            assert!(clean_path(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn credentials_are_blocked() {
        for p in [
            "/Users/u/.ssh/id_ed25519",
            "/Users/u/.ssh",
            "/home/u/.aws/credentials",
            "/home/u/.env",
            "/home/u/.docker/config.json",
            "/home/u/.config/gh/hosts.yml",
            "/home/u/.zsh_history",
            "/Users/u/Library/Keychains/login.keychain-db",
            "/home/u/server.pem",
        ] {
            assert!(check_allowed(p).is_err(), "{p}");
        }
        assert!(check_allowed("/Users/u/Terminals/notes.md").is_ok());
    }

    fn entry(ty: &str, size: u64, name: &str) -> String {
        format!("E\t{ty}\t{size}\t1760650000\t644\t{name}\n")
    }

    #[test]
    fn listing_is_parsed_sorted_and_filtered() {
        let out = format!(
            "FS_DIR=/home/u\n{}{}{}{}{}FS_END\n",
            entry("regular file", 10, "b.txt"),
            entry("directory", 4096, "Docs"),
            entry("regular file", 5, ".hidden"),
            entry("regular file", 1, ".env"),
            entry("socket", 0, "sock"),
        );
        let (dir, e) = parse_listing(&out, false).unwrap();
        assert_eq!(dir, "/home/u");
        let names: Vec<_> = e.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["Docs", "b.txt"]);
        assert_eq!(e[1].path, "/home/u/b.txt");
        assert_eq!(e[1].mode, 0o644);
        let (_, all) = parse_listing(&out, true).unwrap();
        // The blocked .env never appears, the hidden file does.
        assert!(all.iter().any(|x| x.name == ".hidden"));
        assert!(!all.iter().any(|x| x.name == ".env"));
    }

    #[test]
    fn bsd_type_names_work() {
        let out = format!(
            "FS_DIR=/Users/u\n{}{}FS_END\n",
            entry("Regular File", 3, "a"),
            entry("Directory", 64, "d")
        );
        assert_eq!(parse_listing(&out, true).unwrap().1.len(), 2);
    }

    #[test]
    fn hostile_names_cannot_forge_headers_or_paths() {
        // A name with a newline: the continuation line is not a header (an entry came before it) and not an entry.
        let out =
            "FS_DIR=/home/u\nE\tregular file\t1\t1\t644\tx\nFS_ERR=notfound\nFS_DIR=/etc\nFS_END\n";
        let (dir, e) = parse_listing(out, true).unwrap();
        assert_eq!(dir, "/home/u");
        assert_eq!(e.len(), 1);
        // Slash, control characters and empty names are dropped.
        let out = format!(
            "FS_DIR=/home/u\n{}{}{}FS_END\n",
            entry("regular file", 1, "../etc"),
            entry("regular file", 1, "a\u{7}b"),
            entry("regular file", 1, ""),
        );
        assert!(parse_listing(&out, true).unwrap().1.is_empty());
    }

    #[test]
    fn listing_errors_are_typed() {
        assert!(matches!(
            parse_listing("FS_ERR=notfound\n", true),
            Err(SshCoreError::FileNotFound { .. })
        ));
        assert!(parse_listing("FS_ERR=toomany\n", true).is_err());
        assert!(
            parse_listing("FS_DIR=/home/u\n", true).is_err(),
            "cut-off listing"
        );
        assert!(parse_listing("", true).is_err());
        // The resolved folder is checked too: a link into .ssh shows nothing.
        assert!(parse_listing("FS_DIR=/home/u/.ssh\nFS_END\n", true).is_err());
    }

    #[test]
    fn stat_output_is_parsed() {
        let out = "FS_PATH=/home/u/a.txt\nE\tregular file\t42\t1760650000\t600\tx\nFS_END\n";
        let e = parse_stat(out).unwrap();
        assert_eq!((e.name.as_str(), e.size, e.mode), ("a.txt", 42, 0o600));
        assert!(parse_stat("FS_ERR=notfound\n").is_err());
        assert!(
            parse_stat("FS_PATH=/home/u/.ssh/id_rsa\nE\tregular file\t1\t1\t600\tx\nFS_END\n")
                .is_err()
        );
    }

    #[test]
    fn commands_quote_paths() {
        let nasty = "/tmp/it's $(rm -rf ~) `x`; -rf";
        let d = download_command(nasty);
        assert!(d.contains("'\\''") || d.contains("'\"'\"'") || !d.contains("it's"));
        let u = upload_command(nasty, false);
        assert!(u.starts_with("sh -c '"));
        assert!(!u.contains("rm -rf ~) `x`; -rf\"") || u.contains("\\'"));
    }

    #[test]
    fn upload_without_overwrite_checks_twice() {
        let u = upload_command("/tmp/a", false);
        assert_eq!(u.matches("-e").count(), 2);
        let o = upload_command("/tmp/a", true);
        assert!(o.contains("! -d"));
    }

    #[test]
    fn client_requires_a_pinned_host_key() {
        let cfg = |pin: Option<&str>| RemoteServerConfig {
            host: "h".into(),
            port: 22,
            username: "u".into(),
            key: crate::key_store::KeyRef::default(),
            pinned_host_key: pin.map(str::to_string),
        };
        assert!(RemoteFsClient::new(cfg(None)).is_err());
        assert!(RemoteFsClient::new(cfg(Some(""))).is_err());
        assert!(RemoteFsClient::new(cfg(Some("SHA256:abc"))).is_ok());
    }

    #[test]
    fn exit_codes_map_to_errors() {
        assert!(matches!(
            exit_error(3, "/a"),
            SshCoreError::NotARegularFile { .. }
        ));
        assert!(matches!(
            exit_error(4, "/a"),
            SshCoreError::AlreadyExists { .. }
        ));
        assert!(matches!(exit_error(1, "/a"), SshCoreError::IoError { .. }));
        assert_eq!(parent_of("/a/b"), "/a");
        assert_eq!(parent_of("/a"), "/");
    }
}

#[cfg(test)]
mod script_tests {
    //! The host scripts run for real in a temp folder (no ssh, no tmux): this machine's `sh` and `stat`.
    use super::*;
    use std::process::Command;

    fn sh(script: &str, arg: &str) -> String {
        let out = Command::new("sh")
            .arg("-c")
            .arg(script)
            .arg("sh")
            .arg(arg)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    #[test]
    fn list_and_stat_scripts_on_a_real_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        std::fs::write(root.join(".hid"), "x").unwrap();
        std::fs::write(root.join("sp ace's $x.md"), "yy").unwrap();
        std::fs::create_dir(root.join("sub")).unwrap();
        std::os::unix::fs::symlink(root.join("sub"), root.join("link")).unwrap();
        std::os::unix::fs::symlink(root.join("nope"), root.join("broken")).unwrap();
        let r = root.to_str().unwrap();

        let (d, e) = parse_listing(&sh(&list_script(), r), true).unwrap();
        assert_eq!(d, r);
        let names: Vec<_> = e.iter().map(|x| (x.name.as_str(), x.kind)).collect();
        assert!(names.contains(&("a.txt", RemoteFileKind::File)));
        assert!(names.contains(&("sub", RemoteFileKind::Directory)));
        assert!(
            names.contains(&("link", RemoteFileKind::Directory)),
            "{names:?}"
        );
        assert!(names.contains(&(".hid", RemoteFileKind::File)));
        assert!(names.contains(&("sp ace's $x.md", RemoteFileKind::File)));
        assert!(!names.iter().any(|(n, _)| *n == "broken"));
        let a = e.iter().find(|x| x.name == "a.txt").unwrap();
        assert_eq!(a.size, 5);
        assert!(a.modified_unix > 1_600_000_000);
        assert_eq!(
            parse_listing(&sh(&list_script(), r), false)
                .unwrap()
                .1
                .len(),
            4
        );

        let s = parse_stat(&sh(&stat_script(), root.join("a.txt").to_str().unwrap())).unwrap();
        assert_eq!((s.name.as_str(), s.size), ("a.txt", 5));
        assert!(matches!(
            parse_stat(&sh(&stat_script(), root.join("missing").to_str().unwrap())),
            Err(SshCoreError::FileNotFound { .. })
        ));
        assert!(matches!(
            parse_listing(
                &sh(&list_script(), root.join("missing").to_str().unwrap()),
                true
            ),
            Err(SshCoreError::FileNotFound { .. })
        ));
    }

    #[test]
    fn write_commands_do_what_they_say() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let run = |cmd: &str, stdin: &[u8]| {
            use std::io::Write;
            let mut c = Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .env_remove("TMUX")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            c.stdin.take().unwrap().write_all(stdin).unwrap();
            c.wait_with_output().unwrap()
        };
        let f = root.join("it's.bin");
        let fp = f.to_str().unwrap();
        let bytes: Vec<u8> = (0..=255u8).cycle().take(200_000).collect();
        assert_eq!(
            run(&upload_command(fp, false), &bytes).status.code(),
            Some(0)
        );
        assert_eq!(std::fs::read(&f).unwrap(), bytes);
        // Existing file: refused without overwrite, replaced with it, no temp file left behind.
        assert_eq!(
            run(&upload_command(fp, false), b"new").status.code(),
            Some(4)
        );
        assert_eq!(std::fs::read(&f).unwrap(), bytes);
        assert_eq!(
            run(&upload_command(fp, true), b"new").status.code(),
            Some(0)
        );
        assert_eq!(std::fs::read(&f).unwrap(), b"new");
        assert!(!root.join("it's.bin.part").exists());
        // The mode of the replaced file is kept (a script stays executable).
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(
                run(&upload_command(fp, true), b"#!/bin/sh\n").status.code(),
                Some(0)
            );
            assert_eq!(
                std::fs::metadata(&f).unwrap().permissions().mode() & 0o777,
                0o755
            );
        }
        // Download streams the exact bytes, refuses folders.
        std::fs::write(&f, &bytes).unwrap();
        assert_eq!(run(&download_command(fp), b"").stdout, bytes);
        assert_eq!(
            run(&download_command(root.to_str().unwrap()), b"")
                .status
                .code(),
            Some(3)
        );
    }
}
