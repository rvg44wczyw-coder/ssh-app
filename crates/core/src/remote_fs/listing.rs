use super::*;

/// Shell function that prints `type<TAB>size<TAB>mtime<TAB>mode` of `$1` (symlinks followed), for GNU and BSD `stat`.
pub(super) const STAT_FN: &str = r#"TAB=$(printf '\t')
if stat -c %s . >/dev/null 2>&1; then
  st() { stat -L -c "%F${TAB}%s${TAB}%Y${TAB}%a" -- "$1" 2>/dev/null; }
else
  st() { stat -L -f "%HT${TAB}%z${TAB}%m${TAB}%Lp" -- "$1" 2>/dev/null; }
fi
"#;

pub(super) fn list_script() -> String {
    format!(
        r#"{STAT_FN}cd -- "$1" 2>/dev/null || {{ printf 'FS_ERR=notfound\n'; exit 0; }}
printf 'FS_DIR=%s\n' "$(pwd -P)"
n=0
for f in .[!.]* ..?* *; do
  [ -e "$f" ] || continue
  l=$(st "$f") || continue
  n=$((n+1)); [ "$n" -gt {max} ] && {{ printf 'FS_ERR=toomany\n'; exit 0; }}
  printf 'E\t%s\t%s\n' "$l" "$f"
done
printf 'FS_END\n'"#,
        max = MAX_ENTRIES
    )
}

pub(super) fn stat_script() -> String {
    format!(
        r#"{STAT_FN}p=$(realpath -- "$1" 2>/dev/null || readlink -f -- "$1" 2>/dev/null)
[ -n "$p" ] || {{ printf 'FS_ERR=notfound\n'; exit 0; }}
l=$(st "$p") || {{ printf 'FS_ERR=notfound\n'; exit 0; }}
printf 'FS_PATH=%s\nE\t%s\tx\n' "$p" "$l"
printf 'FS_END\n'"#
    )
}

pub(super) struct Parsed {
    dir: Option<String>,
    entries: Vec<(String, RemoteFileKind, u64, i64, u32)>,
    error: Option<String>,
    complete: bool,
}

/// Header lines (`FS_*`) are only trusted before the first entry line, so a file name that contains
/// a newline followed by `FS_ERR=...` cannot change the result.
pub(super) fn parse_fs_output(output: &str) -> Parsed {
    let mut parsed = Parsed {
        dir: None,
        entries: vec![],
        error: None,
        complete: false,
    };
    let mut seen_entry = false;
    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("E\t") {
            seen_entry = true;
            let f: Vec<&str> = rest.splitn(5, '\t').collect();
            if f.len() != 5 {
                continue;
            }
            let ty = f[0].to_ascii_lowercase();
            let kind = if ty.contains("directory") {
                RemoteFileKind::Directory
            } else if ty.contains("regular") {
                RemoteFileKind::File
            } else {
                continue; // sockets, devices, FIFOs
            };
            let (Ok(size), Ok(mtime), Ok(mode)) = (
                f[1].parse::<u64>(),
                f[2].parse::<i64>(),
                u32::from_str_radix(f[3], 8),
            ) else {
                continue;
            };
            let name = f[4];
            if name.is_empty() || name.contains('/') || name.chars().any(|c| c.is_control()) {
                continue;
            }
            parsed
                .entries
                .push((name.to_string(), kind, size, mtime, mode));
        } else if seen_entry {
            if line == "FS_END" {
                parsed.complete = true;
            }
        } else if let Some(d) = line
            .strip_prefix("FS_DIR=")
            .or_else(|| line.strip_prefix("FS_PATH="))
        {
            if parsed.dir.is_none() {
                parsed.dir = Some(d.to_string());
            }
        } else if let Some(e) = line.strip_prefix("FS_ERR=") {
            parsed.error = Some(e.to_string());
        } else if line == "FS_END" {
            parsed.complete = true;
        }
    }
    parsed
}

pub(super) fn parse_listing(
    output: &str,
    include_hidden: bool,
) -> Result<(String, Vec<RemoteEntry>), SshCoreError> {
    let p = parse_fs_output(output);
    match p.error.as_deref() {
        Some("notfound") => {
            return Err(SshCoreError::FileNotFound {
                path: p.dir.unwrap_or_default(),
            })
        }
        Some("toomany") => return Err(invalid("Folder has too many entries")),
        Some(_) => return Err(invalid("Unexpected answer from the host")),
        None => {}
    }
    let dir = p.dir.ok_or_else(|| SshCoreError::IoError {
        reason: "The host did not answer the listing".to_string(),
    })?;
    let dir = clean_path(&dir)?;
    check_allowed(&dir)?;
    if !p.complete {
        return Err(SshCoreError::IoError {
            reason: "The listing was cut off".to_string(),
        });
    }
    let base = if dir == "/" {
        String::new()
    } else {
        dir.clone()
    };
    let mut out = vec![];
    for (name, kind, size, modified_unix, mode) in p.entries {
        if !include_hidden && name.starts_with('.') {
            continue;
        }
        let path = format!("{base}/{name}");
        // Blocked entries (keys, credentials) simply do not exist as far as the Files app is concerned.
        if check_allowed(&path).is_err() {
            continue;
        }
        out.push(RemoteEntry {
            path,
            name,
            kind,
            size,
            modified_unix,
            mode,
        });
    }
    out.sort_by(|a, b| {
        (b.kind == RemoteFileKind::Directory)
            .cmp(&(a.kind == RemoteFileKind::Directory))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok((dir, out))
}

pub(super) fn parse_stat(output: &str) -> Result<RemoteEntry, SshCoreError> {
    let p = parse_fs_output(output);
    if p.error.is_some() {
        return Err(SshCoreError::FileNotFound {
            path: String::new(),
        });
    }
    let path = clean_path(&p.dir.ok_or_else(|| SshCoreError::IoError {
        reason: "The host did not answer".to_string(),
    })?)?;
    check_allowed(&path)?;
    let Some((_, kind, size, modified_unix, mode)) = p.entries.into_iter().next() else {
        return Err(SshCoreError::NotARegularFile { path });
    };
    Ok(RemoteEntry {
        name: name_of(&path),
        path,
        kind,
        size,
        modified_unix,
        mode,
    })
}

pub(super) fn download_command(path: &str) -> String {
    format!(
        "sh -c {}",
        shell_quote(&format!(
            "[ -f {q} ] && [ -r {q} ] || exit 3; exec cat -- {q}",
            q = shell_quote(path)
        ))
    )
}

pub(super) fn upload_command(path: &str, overwrite: bool) -> String {
    let q = shell_quote(path);
    let check = if overwrite {
        "[ ! -d \"$d\" ]".to_string()
    } else {
        "[ ! -e \"$d\" ]".to_string()
    };
    let script = format!(
        "set -C; d={q}; t=\"$d.part.$$\"; umask 077; {check} || exit 4; cat > \"$t\" || {{ rm -f \"$t\"; exit 1; }}; \
         {check} || {{ rm -f \"$t\"; exit 4; }}; if [ -f \"$d\" ]; then m=$(stat -c %a -- \"$d\" 2>/dev/null || stat -f %Lp -- \"$d\" 2>/dev/null); [ -n \"$m\" ] && chmod \"$m\" \"$t\"; fi; \
         mv -f \"$t\" \"$d\" || {{ rm -f \"$t\"; exit 1; }}"
    );
    format!("sh -c {}", shell_quote(&script))
}

pub(super) fn exit_error(code: u32, path: &str) -> SshCoreError {
    match code {
        3 => SshCoreError::NotARegularFile {
            path: path.to_string(),
        },
        4 => SshCoreError::AlreadyExists {
            path: path.to_string(),
        },
        c => SshCoreError::IoError {
            reason: format!("The host command failed (exit {c})"),
        },
    }
}
