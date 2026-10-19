use crate::error::SshCoreError;
use crate::tmux::shell_quote;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RemoteFileKind {
    Directory,
    File,
    Symlink,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RemoteFileEntry {
    pub name: String,
    pub path: String,
    pub kind: RemoteFileKind,
    pub size_bytes: u64,
    pub is_hidden: bool,
    pub permissions: String,
    /// The host's path policy would refuse to open it (credential folders, key files, links into
    /// them); it is listed so the user sees it is there, but it cannot be entered or read.
    pub is_restricted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RemoteDirectoryListing {
    pub current_path: String,
    pub parent_path: Option<String>,
    pub entries: Vec<RemoteFileEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RemoteFilePreview {
    pub path: String,
    pub filename: String,
    pub content: String,
    pub size_bytes: u64,
    pub is_truncated: bool,
    pub is_binary: bool,
}

/// Validates a remote directory path against NUL/control characters and sensitive credential directories.
pub fn validate_remote_dir_path(path: &str) -> Result<String, SshCoreError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(SshCoreError::InvalidInput {
            reason: "Directory path cannot be empty".to_string(),
        });
    }
    if trimmed.contains('\0') || trimmed.contains('\n') || trimmed.contains('\r') {
        return Err(SshCoreError::InvalidInput {
            reason: "Path contains forbidden control characters".to_string(),
        });
    }

    let lowered = trimmed.to_ascii_lowercase();
    let blocked_dirs = [
        "/.ssh",
        "/.aws",
        "/.gnupg",
        "/.kube",
        "/library/keychains",
        "/etc/shadow",
        "/etc/master.passwd",
    ];
    for blocked in blocked_dirs {
        if lowered.ends_with(blocked) || lowered.contains(&format!("{blocked}/")) {
            return Err(SshCoreError::InvalidInput {
                reason: format!("Access to sensitive directory '{blocked}' is restricted"),
            });
        }
    }

    Ok(trimmed.to_string())
}

/// Validates a remote file path before reading its contents, blocking private keys and credential stores.
pub fn validate_remote_file_read_path(path: &str) -> Result<String, SshCoreError> {
    let validated = validate_remote_dir_path(path)?;
    let lowered = validated.to_ascii_lowercase();

    let filename = lowered.rsplit('/').next().unwrap_or(&lowered);
    let blocked_files = [
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

    if blocked_files.contains(&filename)
        || filename.starts_with(".env")
        || filename.ends_with(".pem")
        || filename.ends_with(".key")
    {
        return Err(SshCoreError::InvalidInput {
            reason: format!("Reading sensitive credential file '{filename}' is restricted"),
        });
    }

    Ok(validated)
}

/// Builds a shell command to resolve the target directory (or active tmux pane path / `$HOME`) and run `LC_ALL=C ls -la`.
pub fn build_list_directory_command(
    path: Option<&str>,
    session_name: &str,
) -> Result<String, SshCoreError> {
    let target_dir_expr = match path.map(str::trim).filter(|s| !s.is_empty()) {
        Some("~") => "\"$HOME\"".to_string(),
        Some(explicit) => {
            let safe = validate_remote_dir_path(explicit)?;
            shell_quote(&safe)
        }
        None => {
            let quoted_sess = shell_quote(session_name);
            format!(
                "$(tmux display-message -p -t {quoted_sess} '#{{pane_current_path}}' 2>/dev/null || printf '%s' \"$HOME\")"
            )
        }
    };

    Ok(format!(
        "target_dir={target_dir_expr}; \
         if [ -z \"$target_dir\" ] || [ ! -d \"$target_dir\" ]; then target_dir=\"$HOME\"; fi; \
         cd \"$target_dir\" 2>/dev/null || cd \"$HOME\"; \
         printf '===CWD===\\n'; \
         pwd -P; \
         printf '===LS===\\n'; \
         LC_ALL=C ls -la"
    ))
}

/// What the pinned helper's `ls` was asked for; the path is the only user text and travels as `$1`.
pub(crate) fn build_helper_ls_command(
    sha256: &str,
    path: Option<&str>,
    session_name: &str,
    include_hidden: bool,
) -> Result<String, SshCoreError> {
    let (flag, arg) = match path.map(str::trim).filter(|s| !s.is_empty()) {
        Some("~") => ("--home", None),
        Some(explicit) => ("--path", Some(validate_remote_dir_path(explicit)?)),
        None => {
            crate::tmux::validate_session_name(session_name)?;
            ("--pane", Some(session_name.to_string()))
        }
    };
    let hidden = if include_hidden { " --hidden" } else { "" };
    let value = if arg.is_some() { " \"$1\"" } else { "" };
    let script = format!(
        "h={path}; if [ ! -f \"$h\" ] || [ -L \"$h\" ] || \
         [ \"$(shasum -a 256 < \"$h\" 2>/dev/null | cut -d' ' -f1)\" != '{sha256}' ]; then echo HELPER_ABSENT; \
         else \"$h\" ls {flag}{value}{hidden} 2>/dev/null; fi",
        path = crate::usage::HELPER_PATH_EXPR
    );
    let args: Vec<String> = arg.iter().map(|a| shell_quote(a)).collect();
    Ok(crate::hostprobe::login_sh_script(&script, &args, 20))
}

/// Result of the helper's `ls`: a listing, a refusal the helper stated (shown, not retried through
/// the shell), or `None` when there is no usable reply (absent / older helper, garbage).
pub(crate) fn parse_helper_listing(
    out: &str,
    include_hidden: bool,
) -> Option<Result<RemoteDirectoryListing, String>> {
    for line in out.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        match v.get("ok") {
            Some(serde_json::Value::Bool(false)) => {
                let why = v.get("error").and_then(|e| e.as_str())?;
                return Some(Err(crate::approval_queue::display_safe(why)));
            }
            Some(serde_json::Value::Bool(true)) => {}
            _ => continue,
        }
        let current_path = v.get("path")?.as_str()?.to_string();
        if current_path.chars().any(char::is_control) {
            return None;
        }
        let mut entries = Vec::new();
        for e in v.get("entries")?.as_array()?.iter().take(2000) {
            let name = e.get("name")?.as_str()?;
            if name.is_empty()
                || name == "."
                || name == ".."
                || name.contains('/')
                || name.chars().any(char::is_control)
            {
                continue;
            }
            let kind = match e.get("kind")?.as_str()? {
                "dir" => RemoteFileKind::Directory,
                "file" => RemoteFileKind::File,
                "symlink" => RemoteFileKind::Symlink,
                _ => continue,
            };
            let is_hidden = name.starts_with('.');
            if !include_hidden && is_hidden {
                continue;
            }
            let path = if current_path == "/" {
                format!("/{name}")
            } else {
                format!("{}/{}", current_path.trim_end_matches('/'), name)
            };
            entries.push(RemoteFileEntry {
                name: crate::approval_queue::display_safe(name),
                path,
                kind,
                size_bytes: e.get("size").and_then(|s| s.as_u64()).unwrap_or(0),
                is_hidden,
                permissions: e
                    .get("perms")
                    .and_then(|p| p.as_str())
                    .unwrap_or("")
                    .chars()
                    .filter(|c| "dlrwx-?".contains(*c))
                    .take(10)
                    .collect(),
                is_restricted: e
                    .get("restricted")
                    .and_then(|r| r.as_bool())
                    .unwrap_or(false),
            });
        }
        let parent_path = compute_parent_path(&current_path);
        return Some(Ok(RemoteDirectoryListing {
            current_path,
            parent_path,
            entries,
        }));
    }
    None
}

/// Builds a command printing the fully resolved (symlink-free) path of `path`, or nothing if it does not exist.
pub fn build_resolve_path_command(path: &str) -> String {
    let q = shell_quote(path);
    format!("realpath -- {q} 2>/dev/null || readlink -f -- {q} 2>/dev/null")
}

/// Builds a command that reads at most `max_bytes + 1` bytes, and only from regular files (never FIFOs/devices).
pub fn build_read_file_command(resolved_path: &str, max_bytes: usize) -> String {
    let q = shell_quote(resolved_path);
    format!(
        "if [ -f {q} ]; then head -c {} -- {q}; else echo 'Not a regular file' >&2; exit 1; fi",
        max_bytes + 1
    )
}

/// Computes the parent directory for a given POSIX absolute path.
pub fn compute_parent_path(current_path: &str) -> Option<String> {
    let trimmed = current_path.trim().trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "/" {
        return None;
    }
    match trimmed.rfind('/') {
        Some(0) => Some("/".to_string()),
        Some(idx) => Some(trimmed[..idx].to_string()),
        None => None,
    }
}

/// Parses the output of `build_list_directory_command` into a structured `RemoteDirectoryListing`.
#[uniffi::export]
pub fn parse_remote_directory_listing(
    raw_output: String,
    include_hidden: bool,
) -> RemoteDirectoryListing {
    let mut current_path = "/".to_string();
    let mut in_cwd = false;
    let mut in_ls = false;
    let mut entries = Vec::new();

    for raw_line in raw_output.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line == "===CWD===" {
            in_cwd = true;
            in_ls = false;
            continue;
        }
        if line == "===LS===" {
            in_cwd = false;
            in_ls = true;
            continue;
        }

        if in_cwd {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                // Control characters would be injected into the terminal on `cd` / Insert Path.
                if !trimmed.chars().any(char::is_control) {
                    current_path = trimmed.to_string();
                }
                in_cwd = false;
            }
            continue;
        }

        if in_ls {
            if let Some(entry) = parse_ls_line(&current_path, line) {
                if !include_hidden && entry.is_hidden {
                    continue;
                }
                entries.push(entry);
            }
        }
    }

    // Sort directories first, then alphabetically (case-insensitive)
    entries.sort_by(|a, b| {
        let a_is_dir = a.kind == RemoteFileKind::Directory;
        let b_is_dir = b.kind == RemoteFileKind::Directory;
        b_is_dir.cmp(&a_is_dir).then_with(|| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
        })
    });

    let parent_path = compute_parent_path(&current_path);

    RemoteDirectoryListing {
        current_path,
        parent_path,
        entries,
    }
}

fn parse_ls_line(current_path: &str, line: &str) -> Option<RemoteFileEntry> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with("total ") {
        return None;
    }

    // Standard `LC_ALL=C ls -la` format:
    // permissions links owner group size month day time_or_year name...
    // Example:
    // drwxr-xr-x  12 ivs  staff   384 Oct 16 13:20 src
    // lrwxr-xr-x   1 ivs  staff    14 Oct 16 13:20 link -> target
    let mut iter = trimmed.split_whitespace();
    let perms_raw = iter.next()?;
    let first_char = perms_raw.chars().next()?;
    let kind = match first_char {
        'd' => RemoteFileKind::Directory,
        'l' => RemoteFileKind::Symlink,
        '-' => RemoteFileKind::File,
        _ => return None,
    };

    let _links = iter.next()?;
    let _owner = iter.next()?;
    let _group = iter.next()?;
    let size_str = iter.next()?;
    let size_bytes = size_str.parse::<u64>().unwrap_or(0);

    // Skip month, day, time/year (3 tokens)
    let _month = iter.next()?;
    let _day = iter.next()?;
    let time_token = iter.next()?;

    // Locate where the filename starts in `trimmed` after the 8th whitespace-separated token.
    // Using token byte offsets preserves filenames with spaces.
    let name_start_ptr =
        time_token.as_ptr() as usize + time_token.len() - trimmed.as_ptr() as usize;
    let remainder = trimmed.get(name_start_ptr..)?.trim_start();
    if remainder.is_empty() {
        return None;
    }

    let raw_name = if kind == RemoteFileKind::Symlink {
        remainder
            .split_once(" -> ")
            .map(|(n, _)| n)
            .unwrap_or(remainder)
    } else {
        remainder
    };

    if raw_name == "." || raw_name == ".." || raw_name.is_empty() {
        return None;
    }
    // Filenames with control characters (ESC, CR, ^C...) are never surfaced: inserting them
    // into the terminal would inject keystrokes.
    if raw_name.chars().any(char::is_control) {
        return None;
    }

    let is_hidden = raw_name.starts_with('.');
    let full_path = if current_path == "/" {
        format!("/{raw_name}")
    } else {
        format!("{}/{}", current_path.trim_end_matches('/'), raw_name)
    };

    let permissions = perms_raw.chars().take(10).collect::<String>();

    Some(RemoteFileEntry {
        name: raw_name.to_string(),
        path: full_path,
        kind,
        size_bytes,
        is_hidden,
        permissions,
        is_restricted: false,
    })
}

/// Builds a safe text preview from raw bytes, detecting binary files and truncation.
pub fn build_file_preview_from_bytes(
    path: &str,
    raw_bytes: &[u8],
    max_bytes: usize,
) -> RemoteFilePreview {
    let filename = path
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string();

    let is_truncated = raw_bytes.len() > max_bytes;
    let slice = if is_truncated {
        &raw_bytes[..max_bytes]
    } else {
        raw_bytes
    };

    // Detect binary data by checking for NUL bytes in the first 8 KB
    let sample_len = slice.len().min(8192);
    let is_binary = slice[..sample_len].contains(&0);

    let content = if is_binary {
        "[Binary file — preview not available]".to_string()
    } else {
        String::from_utf8_lossy(slice).to_string()
    };

    RemoteFilePreview {
        path: path.to_string(),
        filename,
        content,
        size_bytes: slice.len() as u64,
        is_truncated,
        is_binary,
    }
}

/// Quotes a remote path for safe insertion into an interactive shell prompt.
#[uniffi::export]
pub fn quote_remote_path_for_shell(path: String) -> String {
    shell_quote(&path)
}

#[cfg(test)]
mod tests {
    #[test]
    fn policy_matches_the_table_shared_with_the_helper() {
        let table = include_str!("../../host/testdata/fs_policy_cases.tsv");
        let mut n = 0;
        for line in table
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        {
            let f: Vec<&str> = line.split('\t').collect();
            assert_eq!(
                validate_remote_dir_path(f[0]).is_ok(),
                f[1] == "1",
                "dir {line}"
            );
            assert_eq!(
                validate_remote_file_read_path(f[0]).is_ok(),
                f[2] == "1",
                "file {line}"
            );
            n += 1;
        }
        assert!(n > 30);
    }

    #[test]
    fn helper_listing_is_parsed_and_distrusted() {
        let reply = r#"{"ok":true,"path":"/w","parent":"/","truncated":false,"entries":[
          {"name":"sub","kind":"dir","size":0,"perms":"drwxr-xr-x","restricted":false},
          {"name":"a -> b","kind":"file","size":3,"perms":"-rw-r--r--"},
          {"name":".env","kind":"file","size":9,"perms":"-rw-------","restricted":true},
          {"name":"p","kind":"other"},
          {"name":"../x","kind":"file"},
          {"name":"ev\u202eil","kind":"file","size":1},
          {"name":".h","kind":"file","size":1}]}"#
            .replace('\n', "");
        let l = parse_helper_listing(&reply, true).unwrap().unwrap();
        assert_eq!(l.current_path, "/w");
        let names: Vec<&str> = l.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names[..2], ["sub", "a -> b"]);
        assert_eq!(names[2], ".env");
        assert!(
            names[3].contains("U+202E") && names[4] == ".h" && names.len() == 5,
            "{names:?}"
        );
        assert_eq!(l.entries[1].path, "/w/a -> b");
        assert!(l.entries[2].is_restricted && !l.entries[0].is_restricted);
        let shown = parse_helper_listing(&reply, false).unwrap().unwrap();
        assert!(!shown.entries.iter().any(|e| e.is_hidden));
    }

    #[test]
    fn helper_refusals_are_shown_and_other_replies_fall_back() {
        let r = parse_helper_listing(
            r#"{"ok":false,"error":"Access to sensitive directory '/.ssh' is restricted"}"#,
            false,
        );
        assert!(matches!(r, Some(Err(e)) if e.contains(".ssh")));
        for none in [
            "HELPER_ABSENT",
            "",
            "{\"error\":\"usage: x\"}",
            "{\"ok\":true}",
            "{\"ok\":true,\"path\":\"/a\\u001bb\",\"entries\":[]}",
        ] {
            assert!(parse_helper_listing(none, false).is_none(), "{none}");
        }
    }

    #[test]
    fn helper_ls_command_quotes_the_path_and_is_pinned() {
        let sha = "b".repeat(64);
        let c = build_helper_ls_command(&sha, Some("/a b/'; id;'"), "main", true).unwrap();
        assert!(
            c.contains("HELPER_ABSENT")
                && c.contains("ls --path \"$1\" --hidden")
                && c.contains(&sha)
        );
        assert!(build_helper_ls_command(&sha, Some("~"), "main", false)
            .unwrap()
            .contains("ls --home"));
        assert!(build_helper_ls_command(&sha, None, "main", false)
            .unwrap()
            .contains("ls --pane \"$1\""));
        assert!(build_helper_ls_command(&sha, None, "bad name!", false).is_err());
        assert!(build_helper_ls_command(&sha, Some("/home/u/.ssh"), "main", false).is_err());
    }

    use super::*;

    #[test]
    fn test_validate_remote_dir_path_blocks_sensitive_dirs() {
        assert!(validate_remote_dir_path("/Users/ivs/rust/ssh-app").is_ok());
        assert!(validate_remote_dir_path("/Users/ivs/.ssh").is_err());
        assert!(validate_remote_dir_path("/Users/ivs/.ssh/keys").is_err());
        assert!(validate_remote_dir_path("/Users/ivs/.aws").is_err());
        assert!(validate_remote_dir_path("/etc/shadow").is_err());
        assert!(validate_remote_dir_path("/tmp/bad\npath").is_err());
    }

    #[test]
    fn test_validate_remote_file_read_path_blocks_keys_and_env() {
        assert!(validate_remote_file_read_path("/Users/ivs/rust/ssh-app/Cargo.toml").is_ok());
        assert!(validate_remote_file_read_path("/Users/ivs/rust/ssh-app/.env").is_err());
        assert!(validate_remote_file_read_path("/Users/ivs/keys/server.pem").is_err());
        assert!(validate_remote_file_read_path("/Users/ivs/id_ed25519").is_err());
    }

    #[test]
    fn test_parse_remote_directory_listing_sorts_and_filters_hidden() {
        let raw = "\
===CWD===
/Users/ivs/rust/ssh-app
===LS===
total 48
drwxr-xr-x  14 ivs  staff    448 Oct 16 13:50 .
drwxr-xr-x   8 ivs  staff    256 Oct 16 10:00 ..
-rw-r--r--   1 ivs  staff    412 Oct 16 12:00 .gitignore
-rw-r--r--   1 ivs  staff   2340 Oct 16 13:10 Cargo.toml
drwxr-xr-x   5 ivs  staff    160 Oct 16 13:20 crates
lrwxr-xr-x   1 ivs  staff     12 Oct 16 13:25 latest -> crates/core
-rw-r--r--   1 ivs  staff   1024 Oct 16 13:30 README with spaces.md
";

        let listing_no_hidden = parse_remote_directory_listing(raw.to_string(), false);
        assert_eq!(listing_no_hidden.current_path, "/Users/ivs/rust/ssh-app");
        assert_eq!(
            listing_no_hidden.parent_path.as_deref(),
            Some("/Users/ivs/rust")
        );
        assert_eq!(listing_no_hidden.entries.len(), 4);
        assert_eq!(listing_no_hidden.entries[0].name, "crates");
        assert_eq!(listing_no_hidden.entries[0].kind, RemoteFileKind::Directory);
        assert_eq!(listing_no_hidden.entries[1].name, "Cargo.toml");
        assert_eq!(listing_no_hidden.entries[2].name, "latest");
        assert_eq!(listing_no_hidden.entries[2].kind, RemoteFileKind::Symlink);
        assert_eq!(listing_no_hidden.entries[3].name, "README with spaces.md");

        let listing_with_hidden = parse_remote_directory_listing(raw.to_string(), true);
        assert_eq!(listing_with_hidden.entries.len(), 5);
    }

    #[test]
    fn test_validate_blocks_env_variants_and_dotfiles_creds() {
        for f in [
            "/a/.env.staging",
            "/a/.npmrc",
            "/home/u/.git-credentials",
            "/a/.pgpass",
        ] {
            assert!(validate_remote_file_read_path(f).is_err(), "{f}");
        }
        assert!(validate_remote_file_read_path("/a/environment.rs").is_ok());
    }

    #[test]
    fn test_parse_skips_control_character_names() {
        let raw = "===CWD===\n/tmp\n===LS===\n\
-rw-r--r--  1 u g 1 Oct 16 13:30 evil\u{1b}[2Jname\n\
-rw-r--r--  1 u g 1 Oct 16 13:30 ok.txt\n";
        let l = parse_remote_directory_listing(raw.to_string(), true);
        assert_eq!(l.entries.len(), 1);
        assert_eq!(l.entries[0].name, "ok.txt");

        let bad_cwd = "===CWD===\n/tmp/a\u{3}b\n===LS===\n";
        assert_eq!(
            parse_remote_directory_listing(bad_cwd.to_string(), true).current_path,
            "/"
        );
    }

    #[test]
    fn test_commands_quote_hostile_paths() {
        let cmd = build_list_directory_command(Some("/tmp/$(id)'; rm -rf ~ #"), "s").unwrap();
        assert!(cmd.contains("'/tmp/$(id)'\\''; rm -rf ~ #'"));
        let r = build_resolve_path_command("/a b/'c");
        assert!(r.contains("'/a b/'\\''c'"));
        let rd = build_read_file_command("/a/b", 10);
        assert!(rd.contains("head -c 11") && rd.contains("[ -f '/a/b' ]"));
        assert!(build_list_directory_command(Some("/x/.ssh"), "s").is_err());
        assert!(build_list_directory_command(None, "s")
            .unwrap()
            .contains("tmux display-message"));
        assert!(build_list_directory_command(Some("~"), "s")
            .unwrap()
            .contains("$HOME"));
    }

    #[test]
    fn test_parent_path() {
        assert_eq!(compute_parent_path("/"), None);
        assert_eq!(compute_parent_path("/a"), Some("/".into()));
        assert_eq!(compute_parent_path("/a/b/"), Some("/a".into()));
        assert_eq!(compute_parent_path("rel"), None);
    }

    #[test]
    fn test_quote_remote_path() {
        assert_eq!(quote_remote_path_for_shell("it's".into()), "'it'\\''s'");
    }

    #[test]
    fn test_build_file_preview_detects_binary_and_truncation() {
        let text = b"fn main() {\n    println!(\"hello\");\n}\n";
        let preview = build_file_preview_from_bytes("/app/src/main.rs", text, 1024);
        assert_eq!(preview.filename, "main.rs");
        assert!(!preview.is_binary);
        assert!(!preview.is_truncated);
        assert!(preview.content.contains("println!"));

        let binary = &[0x89, b'P', b'N', b'G', 0x00, 0x0D, 0x0A];
        let bin_preview = build_file_preview_from_bytes("/app/icon.png", binary, 1024);
        assert!(bin_preview.is_binary);
    }
}
