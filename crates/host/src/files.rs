//! `ssh-app-host ls`: a directory listing as typed JSON, with the path policy applied host-side
//! (feature 32, phase T2).
//!
//! The names are the file system's own bytes (no `ls -la` column parsing), so a name with spaces,
//! ` -> ` or leading blanks is unambiguous. The listed folder is canonicalised first and the policy
//! (credential folders, key and token files) is applied to that real path and, for symlinks, to
//! the target as well. Entries the policy would refuse to open are returned with
//! `restricted: true`.

use crate::proc::run_with_input;
use serde_json::{json, Value};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Entries returned at most; up to ten times as many are collected before sorting (and ten times
/// that many directory entries inspected) before the rest is ignored.
pub const MAX_ENTRIES: usize = 2000;
const READ_LIMIT: usize = MAX_ENTRIES * 10;

const BLOCKED_DIRS: [&str; 7] = [
    "/.ssh",
    "/.aws",
    "/.gnupg",
    "/.kube",
    "/library/keychains",
    "/etc/shadow",
    "/etc/master.passwd",
];

const BLOCKED_FILES: [&str; 14] = [
    "id_ed25519",
    "id_rsa",
    "id_ecdsa",
    "id_dsa",
    ".netrc",
    ".npmrc",
    ".pgpass",
    ".pypirc",
    ".git-credentials",
    "credentials",
    ".zsh_history",
    ".bash_history",
    "shadow",
    "master.passwd",
];

/// Same rules as `validate_remote_dir_path` in the core (a shared table tests both).
pub fn dir_policy(path: &str) -> Result<(), String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("Directory path cannot be empty".into());
    }
    if trimmed.contains(['\0', '\n', '\r']) {
        return Err("Path contains forbidden control characters".into());
    }
    let lowered = trimmed.to_ascii_lowercase();
    for blocked in BLOCKED_DIRS {
        if lowered.ends_with(blocked) || lowered.contains(&format!("{blocked}/")) {
            return Err(format!(
                "Access to sensitive directory '{blocked}' is restricted"
            ));
        }
    }
    Ok(())
}

/// Same rules as `validate_remote_file_read_path` in the core.
pub fn file_policy(path: &str) -> Result<(), String> {
    dir_policy(path)?;
    let lowered = path.trim().to_ascii_lowercase();
    let filename = lowered.rsplit('/').next().unwrap_or(&lowered);
    if BLOCKED_FILES.contains(&filename)
        || filename.starts_with(".env")
        || filename.ends_with(".pem")
        || filename.ends_with(".key")
    {
        return Err(format!(
            "Reading sensitive credential file '{filename}' is restricted"
        ));
    }
    Ok(())
}

pub enum Target {
    Path(String),
    Home,
    Pane(String),
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Letters, digits, `_ . -`, not starting with `-` (tmux would read it as an option / current pane).
fn valid_session_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && !s.starts_with('-')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}

fn pane_path(session: &str) -> Option<PathBuf> {
    let mut cmd = Command::new("tmux");
    cmd.args([
        "display-message",
        "-p",
        "-t",
        session,
        "#{pane_current_path}",
    ]);
    let (code, out) = run_with_input(cmd, b"", Duration::from_secs(3))?;
    let text = String::from_utf8_lossy(&out);
    (code == Some(0))
        .then(|| PathBuf::from(text.trim()))
        .filter(|p| !p.as_os_str().is_empty())
}

fn mode_string(kind: char, mode: u32) -> String {
    let mut s = String::from(kind);
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 7;
        s.push(if bits & 4 != 0 { 'r' } else { '-' });
        s.push(if bits & 2 != 0 { 'w' } else { '-' });
        s.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    s
}

fn entry(dir: &Path, name: &str, ft_meta: &std::fs::Metadata) -> Value {
    let path = dir.join(name);
    let ps = path.to_string_lossy().into_owned();
    let ft = ft_meta.file_type();
    let mode = ft_meta.permissions().mode() & 0o777;
    let (kind, letter) = if ft.is_symlink() {
        ("symlink", 'l')
    } else if ft.is_dir() {
        ("dir", 'd')
    } else if ft.is_file() {
        ("file", '-')
    } else {
        ("other", '?')
    };
    let mut restricted = if kind == "dir" {
        dir_policy(&ps).is_err()
    } else {
        file_policy(&ps).is_err()
    };
    let mut v = json!({
        "name": name, "kind": kind, "size": ft_meta.len(), "mode": mode,
        "perms": mode_string(letter, mode), "hidden": name.starts_with('.'),
        "mtime": ft_meta.modified().ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs()),
    });
    if kind == "symlink" {
        let raw = std::fs::read_link(&path)
            .ok()
            .map(|t| t.to_string_lossy().into_owned());
        v["link"] = json!(raw.map(|t| t.chars().filter(|c| !c.is_control()).collect::<String>()));
        match std::fs::canonicalize(&path) {
            Ok(real) => {
                let rs = real.to_string_lossy().into_owned();
                let target_is_dir = real.is_dir();
                v["target_kind"] = json!(if target_is_dir { "dir" } else { "file" });
                restricted |= if target_is_dir {
                    dir_policy(&rs).is_err()
                } else {
                    file_policy(&rs).is_err()
                };
            }
            Err(_) => v["target_kind"] = json!("broken"),
        }
    }
    v["restricted"] = json!(restricted);
    v
}

pub fn list(target: &Target, hidden: bool, max: usize) -> Value {
    let max = max.clamp(1, MAX_ENTRIES);
    let home = home();
    let candidate = match target {
        Target::Home => Some(home.clone()),
        Target::Pane(s) if valid_session_name(s) => pane_path(s),
        Target::Pane(_) => return json!({"ok": false, "error": "invalid session name"}),
        Target::Path(p) => {
            if let Err(e) = dir_policy(p) {
                return json!({"ok": false, "error": e});
            }
            let p = Path::new(p.trim());
            Some(if p.is_absolute() {
                p.to_path_buf()
            } else {
                home.join(p)
            })
        }
    };
    // A folder that does not exist (or is not one) falls back to $HOME, as the shell version did.
    let real = candidate
        .filter(|c| c.is_dir())
        .and_then(|c| std::fs::canonicalize(c).ok())
        .or_else(|| std::fs::canonicalize(&home).ok());
    let Some(real) = real else {
        return json!({"ok": false, "error": "cannot read the folder"});
    };
    let real_s = real.to_string_lossy().into_owned();
    if real_s.chars().any(char::is_control) {
        return json!({"ok": false, "error": "path contains control characters"});
    }
    if let Err(e) = dir_policy(&real_s) {
        return json!({"ok": false, "error": e});
    }
    let Ok(rd) = std::fs::read_dir(&real) else {
        return json!({"ok": false, "error": "cannot read the folder"});
    };
    let mut entries: Vec<Value> = Vec::new();
    let mut truncated = false;
    let mut seen = 0usize;
    for item in rd.flatten() {
        // Entries that are skipped (hidden, odd names) do not count, so a flood of dotfiles cannot
        // push ordinary files out of the listing; `seen` still bounds the work.
        seen += 1;
        if entries.len() >= READ_LIMIT || seen > READ_LIMIT * 10 {
            truncated = true;
            break;
        }
        // Names that are not UTF-8 or carry control characters are never surfaced: inserted into the
        // terminal they would inject keystrokes (and a lossy name would point to nothing).
        let Some(name) = item.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if name.chars().any(char::is_control) || (!hidden && name.starts_with('.')) {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(item.path()) else {
            continue;
        };
        entries.push(entry(&real, &name, &meta));
    }
    entries.sort_by(|a, b| {
        let dir = |v: &Value| v["kind"] == "dir";
        dir(b).cmp(&dir(a)).then_with(|| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .to_ascii_lowercase()
                .cmp(&b["name"].as_str().unwrap_or("").to_ascii_lowercase())
        })
    });
    if entries.len() > max {
        entries.truncate(max);
        truncated = true;
    }
    let parent = real.parent().map(|p| p.to_string_lossy().into_owned());
    json!({"ok": true, "path": real_s, "parent": parent, "truncated": truncated, "entries": entries})
}

pub const DEFAULT_READ: usize = 65_536;
pub const MAX_READ: usize = 131_072;

/// The path the kernel says an open descriptor refers to.
#[cfg(target_os = "macos")]
fn fd_path(f: &std::fs::File) -> Option<String> {
    let mut buf = vec![0u8; libc::PATH_MAX as usize];
    // SAFETY: F_GETPATH writes a NUL-terminated path of at most PATH_MAX bytes into `buf`.
    if unsafe { libc::fcntl(f.as_raw_fd(), libc::F_GETPATH, buf.as_mut_ptr()) } == -1 {
        return None;
    }
    let end = buf.iter().position(|b| *b == 0)?;
    String::from_utf8(buf[..end].to_vec()).ok()
}

#[cfg(not(target_os = "macos"))]
fn fd_path(f: &std::fs::File) -> Option<String> {
    std::fs::read_link(format!("/proc/self/fd/{}", f.as_raw_fd()))
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

/// Reads at most `max` bytes of a regular file. The path is canonicalised, checked, opened with
/// `O_NOFOLLOW | O_NONBLOCK` (a FIFO or device never blocks), the descriptor must be a regular file,
/// and the policy is applied once more to where the descriptor really points, so a component swapped
/// for a link after the first check cannot reach a blocked file.
pub fn read(path: &str, max: usize) -> Value {
    let max = if max == 0 {
        DEFAULT_READ
    } else {
        max.min(MAX_READ)
    };
    if let Err(e) = file_policy(path) {
        return json!({"ok": false, "error": e});
    }
    let Ok(real) = std::fs::canonicalize(path.trim()) else {
        return json!({"ok": false, "error": "File not found"});
    };
    let real_s = real.to_string_lossy().into_owned();
    if let Err(e) = file_policy(&real_s) {
        return json!({"ok": false, "error": e});
    }
    let opened = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&real);
    let Ok(file) = opened else {
        return json!({"ok": false, "error": "Cannot open the file"});
    };
    let Ok(meta) = file.metadata() else {
        return json!({"ok": false, "error": "Cannot open the file"});
    };
    if !meta.is_file() {
        return json!({"ok": false, "error": "Not a regular file"});
    }
    match fd_path(&file) {
        Some(at) => {
            if let Err(e) = file_policy(&at) {
                return json!({"ok": false, "error": e});
            }
        }
        None => return json!({"ok": false, "error": "Cannot verify the file location"}),
    }
    let mut buf = Vec::new();
    if file.take(max as u64 + 1).read_to_end(&mut buf).is_err() {
        return json!({"ok": false, "error": "Cannot read the file"});
    }
    let truncated = buf.len() > max;
    buf.truncate(max);
    let binary = buf[..buf.len().min(8192)].contains(&0);
    let mut v = json!({"ok": true, "path": real_s, "size": meta.len(), "truncated": truncated,
                       "binary": binary, "read": buf.len()});
    if !binary {
        v["text"] = json!(String::from_utf8_lossy(&buf));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_matches_the_shared_table() {
        let table = include_str!("../testdata/fs_policy_cases.tsv");
        let mut n = 0;
        for line in table
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        {
            let f: Vec<&str> = line.split('\t').collect();
            assert_eq!(dir_policy(f[0]).is_ok(), f[1] == "1", "dir {line}");
            assert_eq!(file_policy(f[0]).is_ok(), f[2] == "1", "file {line}");
            n += 1;
        }
        assert!(n > 30);
        assert!(
            dir_policy("").is_err() && dir_policy("/a\nb").is_err() && dir_policy("/a\0b").is_err()
        );
    }

    #[test]
    fn mode_strings() {
        assert_eq!(mode_string('d', 0o755), "drwxr-xr-x");
        assert_eq!(mode_string('-', 0o600), "-rw-------");
    }

    #[test]
    fn lists_odd_names_symlinks_and_restricted_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let w = root.join("w");
        std::fs::create_dir_all(w.join("sub")).unwrap();
        std::fs::create_dir_all(root.join(".ssh")).unwrap();
        std::fs::write(w.join("a -> b"), "x").unwrap();
        std::fs::write(w.join("  lead"), "x").unwrap();
        std::fs::write(w.join(".env"), "SECRET=1").unwrap();
        std::fs::write(w.join(".hidden"), "x").unwrap();
        std::fs::write(w.join("bad\nname"), "x").unwrap();
        std::fs::write(w.join("zz.txt"), "hello").unwrap();
        std::os::unix::fs::symlink(root.join(".ssh"), w.join("innocent")).unwrap();
        std::os::unix::fs::symlink(w.join("nowhere"), w.join("broken")).unwrap();
        std::os::unix::fs::symlink(w.join("zz.txt"), w.join("fine")).unwrap();
        let r = list(&Target::Path(w.to_string_lossy().into()), true, 100);
        assert_eq!(r["ok"], true, "{r}");
        let names: Vec<&str> = r["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap())
            .collect();
        assert_eq!(names[0], "sub", "directories first: {names:?}");
        assert!(names.contains(&"a -> b") && names.contains(&"  lead"));
        assert!(names.contains(&".hidden") && !names.iter().any(|n| n.contains('\n')));
        let by = |n: &str| {
            r["entries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["name"] == n)
                .unwrap()
                .clone()
        };
        assert_eq!(by(".env")["restricted"], true);
        assert_eq!(by("innocent")["restricted"], true, "link into .ssh");
        assert_eq!(by("innocent")["target_kind"], "dir");
        assert_eq!(by("broken")["target_kind"], "broken");
        assert_eq!(by("fine")["restricted"], false);
        assert_eq!(by("zz.txt")["size"], 5);
        let shown = list(&Target::Path(w.to_string_lossy().into()), false, 100);
        assert!(!shown["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["name"] == ".hidden"));
        // The listed folder itself is refused when it is (or is reached through a link to) .ssh.
        let via = list(
            &Target::Path(w.join("innocent").to_string_lossy().into()),
            true,
            100,
        );
        assert_eq!(via["ok"], false, "{via}");
        assert_eq!(
            list(
                &Target::Path(root.join(".ssh").to_string_lossy().into()),
                true,
                10
            )["ok"],
            false
        );
    }

    #[test]
    fn truncates_and_falls_back_to_home() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        for i in 0..30 {
            std::fs::write(root.join(format!("f{i:02}")), "").unwrap();
        }
        let r = list(&Target::Path(root.to_string_lossy().into()), false, 10);
        assert_eq!(
            (
                r["entries"].as_array().unwrap().len(),
                r["truncated"].clone()
            ),
            (10, json!(true))
        );
        let gone = list(
            &Target::Path(root.join("missing").to_string_lossy().into()),
            false,
            10,
        );
        assert_eq!(gone["ok"], true);
        assert_eq!(
            gone["path"].as_str().map(PathBuf::from),
            std::fs::canonicalize(home()).ok()
        );
        assert_eq!(list(&Target::Pane("a b".into()), false, 10)["ok"], false);
    }

    #[test]
    fn dotfile_flood_does_not_hide_ordinary_files_and_pane_names_are_strict() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        for i in 0..2500 {
            std::fs::write(root.join(format!(".d{i:05}")), "").unwrap();
        }
        for i in 0..20 {
            std::fs::write(root.join(format!("real{i:02}")), "").unwrap();
        }
        let r = list(&Target::Path(root.to_string_lossy().into()), false, 100);
        assert_eq!(
            r["entries"].as_array().unwrap().len(),
            20,
            "{}",
            r["truncated"]
        );
        assert!(
            !valid_session_name("-x") && !valid_session_name("") && valid_session_name("a-b_1.2")
        );
    }

    #[test]
    fn reads_text_binary_truncated_and_refuses_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let f = |n: &str| root.join(n).to_string_lossy().into_owned();
        std::fs::write(root.join("a.txt"), "hello\nworld").unwrap();
        std::fs::write(root.join("bin.dat"), [1u8, 0, 2]).unwrap();
        std::fs::write(root.join("big.txt"), "x".repeat(5000)).unwrap();
        std::fs::write(root.join(".env"), "S=1").unwrap();
        std::fs::create_dir(root.join(".ssh")).unwrap();
        std::fs::write(root.join(".ssh/k"), "key").unwrap();
        std::os::unix::fs::symlink(root.join(".ssh/k"), root.join("innocent")).unwrap();
        std::os::unix::fs::symlink(root.join("a.txt"), root.join("fine")).unwrap();
        let t = read(&f("a.txt"), 0);
        assert_eq!(
            (t["ok"].clone(), t["text"].as_str(), t["size"].clone()),
            (json!(true), Some("hello\nworld"), json!(11))
        );
        assert_eq!(read(&f("bin.dat"), 0)["binary"], true);
        assert!(read(&f("bin.dat"), 0).get("text").is_none());
        let b = read(&f("big.txt"), 1000);
        assert_eq!(
            (b["truncated"].clone(), b["read"].clone(), b["size"].clone()),
            (json!(true), json!(1000), json!(5000))
        );
        assert_eq!(read(&f(".env"), 0)["ok"], false);
        assert_eq!(read(&f("innocent"), 0)["ok"], false, "link into .ssh");
        assert_eq!(read(&f("fine"), 0)["ok"], true, "harmless link");
        assert_eq!(read(&f("missing"), 0)["ok"], false);
        assert_eq!(
            read(&root.to_string_lossy(), 0)["error"],
            "Not a regular file"
        );
    }

    #[test]
    fn a_fifo_is_refused_without_blocking() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("pipe");
        let c = std::ffi::CString::new(p.to_string_lossy().as_bytes()).unwrap();
        // SAFETY: valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let t = std::time::Instant::now();
        assert_eq!(read(&p.to_string_lossy(), 0)["error"], "Not a regular file");
        assert!(t.elapsed() < std::time::Duration::from_secs(2));
    }
}
