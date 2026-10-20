//! `pair`: appends one phone key to `~/.ssh/authorized_keys` (feature 44).
//!
//! The request is one JSON object on stdin, never in the command line:
//! `{"key": "ssh-ed25519 AAAA...", "name": "My iPhone"}`. The key must be a plain public key of
//! the two allowed types (no options), the name becomes the comment `ssh-app: <name>` and may only
//! hold letters, digits, space and `._-`, so nothing can start a second line or add key options.
//!
//! The file is never rewritten or shortened: a missing final newline is completed, the new line is
//! appended with a single write, an existing line with the same key blob means "nothing to do".
//! A symlink at `~/.ssh` or at the file is refused (never followed), a directory that others can
//! write to is refused, a file that is too open is tightened to `0600` and reported.

use serde_json::{json, Value};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::time::{Duration, Instant};

const MAX_INPUT: u64 = 2048;
const MAX_FILE: u64 = 1 << 20;
const MAX_NAME: usize = 40;
const BACKUP: &str = "authorized_keys.ssh-app-bak";
const LOCK: &str = ".ssh-app-pair.lock";

/// Base64 of the key-type string that every blob of that type starts with.
const ED25519_PREFIX: &str = "AAAAC3NzaC1lZDI1NTE5AAAA";
const SK_ED25519_PREFIX: &str = "AAAAGnNrLXNzaC1lZDI1NTE5QG9wZW5zc2guY29t";

fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// `(type, blob)` of a plain public key line, or why it is refused. A trailing comment is ignored.
pub fn parse_key(line: &str) -> Result<(&str, &str), &'static str> {
    if line.len() > 600 || line.chars().any(|c| c.is_control()) {
        return Err("bad key");
    }
    let mut parts = line.split(' ');
    let (Some(kind), Some(blob)) = (parts.next(), parts.next()) else {
        return Err("bad key");
    };
    let base64 = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
    };
    let shape = match kind {
        "ssh-ed25519" => blob.len() == 68 && blob.starts_with(ED25519_PREFIX),
        "sk-ssh-ed25519@openssh.com" => blob.len() <= 400 && blob.starts_with(SK_ED25519_PREFIX),
        _ => return Err("unsupported key type"),
    };
    if shape && base64(blob) {
        Ok((kind, blob))
    } else {
        Err("bad key")
    }
}

pub fn valid_name(name: &str) -> bool {
    let n = name.chars().count();
    (1..=MAX_NAME).contains(&n)
        && name.trim() == name
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'))
}

/// Whether some line of `file` carries `blob` as one of its whitespace-separated fields. A line
/// that is commented out does not authorize anything and does not count.
pub fn has_blob(file: &[u8], blob: &str) -> bool {
    file.split(|b| *b == b'\n').any(|line| {
        let first = line.iter().find(|b| !b.is_ascii_whitespace());
        first != Some(&b'#')
            && line
                .split(|b| b.is_ascii_whitespace())
                .any(|field| field == blob.as_bytes())
    })
}

fn lock(dir: &Path) -> Result<File, &'static str> {
    let f = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(dir.join(LOCK))
        .map_err(|_| "could not lock")?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        use std::os::fd::AsRawFd;
        // SAFETY: the descriptor is open for the whole call.
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(f);
        }
        if Instant::now() >= deadline {
            return Err("busy");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn ssh_dir(home: &Path) -> Result<std::path::PathBuf, &'static str> {
    let dir = home.join(".ssh");
    match std::fs::symlink_metadata(&dir) {
        Ok(m) if m.file_type().is_symlink() => return Err("~/.ssh is a symlink"),
        Ok(m) if !m.is_dir() => return Err("~/.ssh is not a directory"),
        Ok(m) if m.uid() != euid() => return Err("~/.ssh is not yours"),
        Ok(m) if m.mode() & 0o022 != 0 => return Err("~/.ssh is writable by others"),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(&dir).map_err(|_| "could not create ~/.ssh")?;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| "could not create ~/.ssh")?;
        }
        Err(_) => return Err("~/.ssh is not readable"),
    }
    Ok(dir)
}

struct Outcome {
    added: bool,
    tightened: bool,
}

fn append_key(home: &Path, kind: &str, blob: &str, name: &str) -> Result<Outcome, &'static str> {
    let dir = ssh_dir(home)?;
    let _lock = lock(&dir)?;
    let path = dir.join("authorized_keys");
    let mut file = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .map_err(|_| "authorized_keys is a symlink or not writable")?;
    let meta = file
        .metadata()
        .map_err(|_| "authorized_keys is unreadable")?;
    if !meta.is_file() || meta.uid() != euid() {
        return Err("authorized_keys is not a regular file of yours");
    }
    if meta.len() > MAX_FILE {
        return Err("authorized_keys is too large");
    }
    let mut tightened = false;
    if meta.mode() & 0o077 != 0 {
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|_| "could not tighten the file mode")?;
        tightened = true;
    }
    let mut content = Vec::new();
    (&mut file)
        .take(MAX_FILE)
        .read_to_end(&mut content)
        .map_err(|_| "authorized_keys is unreadable")?;
    if has_blob(&content, blob) {
        return Ok(Outcome {
            added: false,
            tightened,
        });
    }
    if !content.is_empty() {
        // One backup, made the first time the file changes; an existing one is kept.
        if let Ok(mut b) = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(dir.join(BACKUP))
        {
            let _ = b.write_all(&content);
        }
    }
    let mut line = Vec::new();
    if content.last().is_some_and(|b| *b != b'\n') {
        line.push(b'\n');
    }
    line.extend_from_slice(format!("{kind} {blob} ssh-app: {name}\n").as_bytes());
    file.write_all(&line).map_err(|_| "could not write")?;
    Ok(Outcome {
        added: true,
        tightened,
    })
}

/// Handles one request read from `input`; the answer is the JSON printed on stdout.
pub fn pair(home: &Path, input: &mut dyn Read) -> Value {
    let mut text = String::new();
    if input.take(MAX_INPUT + 1).read_to_string(&mut text).is_err() || text.len() as u64 > MAX_INPUT
    {
        return json!({"ok": false, "error": "bad input"});
    }
    let Ok(req) = serde_json::from_str::<Value>(&text) else {
        return json!({"ok": false, "error": "bad input"});
    };
    let (Some(key), Some(name)) = (req["key"].as_str(), req["name"].as_str()) else {
        return json!({"ok": false, "error": "bad input"});
    };
    let (kind, blob) = match parse_key(key) {
        Ok(k) => k,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    if !valid_name(name) {
        return json!({"ok": false, "error": "bad name"});
    }
    match append_key(home, kind, blob, name) {
        Ok(o) => json!({"ok": true, "added": o.added, "tightened": o.tightened}),
        Err(e) => json!({"ok": false, "error": e}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    const BLOB: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";
    fn key() -> String {
        format!("ssh-ed25519 {BLOB}")
    }
    fn req(key: &str, name: &str) -> Vec<u8> {
        json!({"key": key, "name": name}).to_string().into_bytes()
    }
    fn run(home: &Path, body: &[u8]) -> Value {
        pair(home, &mut &body[..])
    }
    fn auth(home: &Path) -> String {
        std::fs::read_to_string(home.join(".ssh/authorized_keys")).unwrap()
    }

    #[test]
    fn key_validation_table() {
        assert!(parse_key(&key()).is_ok());
        assert!(parse_key(&format!("{} laptop comment", key())).is_ok());
        let sk = format!("sk-ssh-ed25519@openssh.com {SK_ED25519_PREFIX}AAAA");
        assert!(parse_key(&sk).is_ok());
        for bad in [
            "".to_string(),
            "ssh-rsa AAAAB3NzaC1yc2E".into(),
            format!("command=\"x\" {}", key()),
            format!("{}\nssh-ed25519 {BLOB}", key()),
            format!("{}\0", key()),
            format!("ssh-ed25519 {}", &BLOB[..60]),
            format!("ssh-ed25519 {}", "A".repeat(68)),
            format!("ssh-ed25519 {}", BLOB.replace('I', "!")),
            format!("ssh-ed25519 {}", "A".repeat(700)),
            format!("ssh-ed25519\t{BLOB}"),
        ] {
            assert!(parse_key(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn name_validation_table() {
        for ok in ["My iPhone", "iPad-2", "phone_1.0", "a"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in [
            "",
            " lead",
            "trail ",
            "a\nb",
            "a\"b",
            "a;b",
            "an iphone with an é",
            &"x".repeat(41),
            "x\ty",
        ] {
            assert!(!valid_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn creates_the_folder_and_the_file_with_private_modes() {
        let h = tempfile::tempdir().unwrap();
        let r = run(h.path(), &req(&key(), "My iPhone"));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["added"], true);
        assert_eq!(auth(h.path()), format!("{} ssh-app: My iPhone\n", key()));
        let mode = |p: &str| std::fs::metadata(h.path().join(p)).unwrap().mode() & 0o777;
        assert_eq!(mode(".ssh"), 0o700);
        assert_eq!(mode(".ssh/authorized_keys"), 0o600);
    }

    #[test]
    fn a_second_pairing_adds_nothing() {
        let h = tempfile::tempdir().unwrap();
        assert_eq!(run(h.path(), &req(&key(), "A"))["added"], true);
        let again = run(h.path(), &req(&key(), "B"));
        assert_eq!(
            (again["ok"].clone(), again["added"].clone()),
            (true.into(), false.into())
        );
        assert_eq!(auth(h.path()).lines().count(), 1);
    }

    #[test]
    fn a_file_without_a_final_newline_is_completed_not_merged() {
        let h = tempfile::tempdir().unwrap();
        std::fs::create_dir(h.path().join(".ssh")).unwrap();
        std::fs::set_permissions(
            h.path().join(".ssh"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        std::fs::write(h.path().join(".ssh/authorized_keys"), "ssh-rsa OLD me@x").unwrap();
        assert_eq!(run(h.path(), &req(&key(), "P"))["added"], true);
        let text = auth(h.path());
        assert_eq!(text, format!("ssh-rsa OLD me@x\n{} ssh-app: P\n", key()));
    }

    #[test]
    fn crlf_files_keep_their_lines_and_get_one_appended() {
        let h = tempfile::tempdir().unwrap();
        std::fs::create_dir(h.path().join(".ssh")).unwrap();
        std::fs::set_permissions(
            h.path().join(".ssh"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        std::fs::write(h.path().join(".ssh/authorized_keys"), "ssh-rsa OLD\r\n").unwrap();
        assert_eq!(run(h.path(), &req(&key(), "P"))["added"], true);
        assert!(auth(h.path()).starts_with("ssh-rsa OLD\r\n"));
    }

    #[test]
    fn a_key_already_present_with_options_or_another_comment_is_not_added() {
        for existing in [
            format!("command=\"echo hi\",no-pty {} old\n", key()),
            format!("{} someone@else\n", key()),
        ] {
            let h = tempfile::tempdir().unwrap();
            std::fs::create_dir(h.path().join(".ssh")).unwrap();
            std::fs::set_permissions(
                h.path().join(".ssh"),
                std::fs::Permissions::from_mode(0o700),
            )
            .unwrap();
            std::fs::write(h.path().join(".ssh/authorized_keys"), &existing).unwrap();
            let r = run(h.path(), &req(&key(), "P"));
            assert_eq!(
                (r["ok"].clone(), r["added"].clone()),
                (true.into(), false.into())
            );
            assert_eq!(auth(h.path()), existing);
        }
    }

    #[test]
    fn non_utf8_content_is_kept_byte_for_byte() {
        let h = tempfile::tempdir().unwrap();
        std::fs::create_dir(h.path().join(".ssh")).unwrap();
        std::fs::set_permissions(
            h.path().join(".ssh"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let old = b"# \xff\xfe comment\n".to_vec();
        std::fs::write(h.path().join(".ssh/authorized_keys"), &old).unwrap();
        assert_eq!(run(h.path(), &req(&key(), "P"))["added"], true);
        let now = std::fs::read(h.path().join(".ssh/authorized_keys")).unwrap();
        assert!(now.starts_with(&old));
    }

    #[test]
    fn a_symlinked_file_or_folder_is_refused_and_not_followed() {
        let h = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        std::fs::create_dir(h.path().join(".ssh")).unwrap();
        std::fs::set_permissions(
            h.path().join(".ssh"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let target = other.path().join("real");
        std::fs::write(&target, "x\n").unwrap();
        symlink(&target, h.path().join(".ssh/authorized_keys")).unwrap();
        let r = run(h.path(), &req(&key(), "P"));
        assert_eq!(r["ok"], false, "{r}");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "x\n");

        let h2 = tempfile::tempdir().unwrap();
        symlink(other.path(), h2.path().join(".ssh")).unwrap();
        assert_eq!(run(h2.path(), &req(&key(), "P"))["ok"], false);
        assert!(!other.path().join("authorized_keys").exists());
    }

    #[test]
    fn a_folder_that_others_can_write_is_refused() {
        let h = tempfile::tempdir().unwrap();
        std::fs::create_dir(h.path().join(".ssh")).unwrap();
        std::fs::set_permissions(
            h.path().join(".ssh"),
            std::fs::Permissions::from_mode(0o770),
        )
        .unwrap();
        assert_eq!(run(h.path(), &req(&key(), "P"))["ok"], false);
    }

    #[test]
    fn a_loose_file_mode_is_tightened_and_reported() {
        let h = tempfile::tempdir().unwrap();
        std::fs::create_dir(h.path().join(".ssh")).unwrap();
        std::fs::set_permissions(
            h.path().join(".ssh"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let f = h.path().join(".ssh/authorized_keys");
        std::fs::write(&f, "ssh-rsa OLD\n").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o666)).unwrap();
        let r = run(h.path(), &req(&key(), "P"));
        assert_eq!(
            (r["added"].clone(), r["tightened"].clone()),
            (true.into(), true.into())
        );
        assert_eq!(std::fs::metadata(&f).unwrap().mode() & 0o777, 0o600);
    }

    #[test]
    fn the_backup_is_made_once() {
        let h = tempfile::tempdir().unwrap();
        std::fs::create_dir(h.path().join(".ssh")).unwrap();
        std::fs::set_permissions(
            h.path().join(".ssh"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let f = h.path().join(".ssh/authorized_keys");
        std::fs::write(&f, "ssh-rsa OLD\n").unwrap();
        run(h.path(), &req(&key(), "P"));
        let bak = h.path().join(".ssh").join(BACKUP);
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), "ssh-rsa OLD\n");
        let other = format!("ssh-ed25519 {}", BLOB.replace("IOMq", "IOMr"));
        run(h.path(), &req(&other, "Q"));
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), "ssh-rsa OLD\n");
    }

    #[test]
    fn a_huge_file_is_refused_untouched() {
        let h = tempfile::tempdir().unwrap();
        std::fs::create_dir(h.path().join(".ssh")).unwrap();
        std::fs::set_permissions(
            h.path().join(".ssh"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let f = h.path().join(".ssh/authorized_keys");
        std::fs::write(&f, vec![b'#'; (MAX_FILE + 1) as usize]).unwrap();
        assert_eq!(run(h.path(), &req(&key(), "P"))["ok"], false);
        assert_eq!(std::fs::metadata(&f).unwrap().len(), MAX_FILE + 1);
    }

    #[test]
    fn hostile_input_is_refused_before_any_file_is_touched() {
        let h = tempfile::tempdir().unwrap();
        let injected = format!("{}\nssh-ed25519 {}", key(), BLOB);
        for body in [
            req(&injected, "P"),
            req(&key(), "P\nssh-ed25519 AAAA"),
            req(&key(), "x\" ; rm -rf ~ #"),
            b"not json".to_vec(),
            vec![b'{'; 5000],
            json!({"key": 1, "name": 2}).to_string().into_bytes(),
            json!({"key": key()}).to_string().into_bytes(),
        ] {
            assert_eq!(run(h.path(), &body)["ok"], false);
        }
        assert!(!h.path().join(".ssh").exists());
    }

    #[test]
    fn concurrent_pairings_of_one_key_give_one_line() {
        let h = tempfile::tempdir().unwrap();
        let home = h.path().to_path_buf();
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let home = home.clone();
                std::thread::spawn(move || run(&home, &req(&key(), "P")))
            })
            .collect();
        let added = threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .filter(|r| r["added"] == true)
            .count();
        assert_eq!(added, 1);
        assert_eq!(auth(h.path()).lines().count(), 1);
    }

    #[test]
    fn blob_search_matches_whole_fields_only() {
        assert!(has_blob(b"ssh-ed25519 ABC c\n", "ABC"));
        assert!(has_blob(b"opt=\"a b\" ssh-ed25519 ABC", "ABC"));
        assert!(!has_blob(b"ssh-ed25519 ABCD c\n", "ABC"));
        assert!(
            !has_blob(b"# ABCD", "ABCD"),
            "a commented-out key is not authorized"
        );
        assert!(!has_blob(b"  \t# ssh-ed25519 ABCD x", "ABCD"));
        assert!(has_blob(b"# c\nssh-ed25519 ABCD x", "ABCD"));
    }
}
