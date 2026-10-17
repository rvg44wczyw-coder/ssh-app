//! Remote file system access for the Files app (File Provider extension): list, stat, download,
//! upload, create folder, rename, delete. It is its own client (`RemoteFsClient`): no tmux, no PTY,
//! a pinned host key is mandatory, file bytes are streamed to and from local files and never held
//! in memory. Every path is cleaned, checked against the credential blocklist of the file browser
//! (on the resolved location, so a symlink cannot expose a blocked file) and passed to the host as
//! a quoted argument.

use crate::config::RemoteServerConfig;
use crate::error::SshCoreError;
use crate::file_browser::{
    build_resolve_path_command, validate_remote_dir_path, validate_remote_file_read_path,
    RemoteFileKind,
};
use crate::tmux::shell_quote;
use crate::transport::{
    collect_output, connect_authenticated, exec_start, ClientHandle, Endpoint, ExecStream,
    StreamEvent, DEFAULT_EXEC_TIMEOUT,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Entries returned for one folder; more is an error rather than a silent cut.
const MAX_ENTRIES: usize = 20_000;
const MAX_PATH_LEN: usize = 4096;
const CHUNK: usize = 64 * 1024;
const IDLE_LIMIT: Duration = Duration::from_secs(60);
const PROGRESS_STEP: u64 = 256 * 1024;
const WRITE_STALL: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RemoteEntry {
    /// Absolute path on the host.
    pub path: String,
    pub name: String,
    /// `Directory` or `File` (symlinks are reported as what they point to).
    pub kind: RemoteFileKind,
    pub size: u64,
    pub modified_unix: i64,
    pub mode: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct TransferResult {
    pub bytes: u64,
}

#[uniffi::export(callback_interface)]
pub trait TransferProgress: Send + Sync {
    fn on_progress(&self, bytes: u64);
    fn is_cancelled(&self) -> bool;
}

fn invalid(reason: &str) -> SshCoreError {
    SshCoreError::InvalidInput {
        reason: reason.to_string(),
    }
}

/// Absolute path without `.`/`..`/empty components, control characters or excess length.
pub fn clean_path(path: &str) -> Result<String, SshCoreError> {
    let p = path.trim();
    if !p.starts_with('/') {
        return Err(invalid("Path must be absolute"));
    }
    if p.len() > MAX_PATH_LEN || p.chars().any(|c| c.is_control()) {
        return Err(invalid("Path is too long or contains control characters"));
    }
    let mut parts = vec![];
    for c in p.split('/') {
        match c {
            "" => {}
            "." | ".." => return Err(invalid("Path must not contain '.' or '..' components")),
            c => parts.push(c),
        }
    }
    Ok(format!("/{}", parts.join("/")))
}

/// The blocklist of the file browser plus a few more credential stores.
pub fn check_allowed(path: &str) -> Result<(), SshCoreError> {
    validate_remote_file_read_path(path)?;
    let lower = path.to_ascii_lowercase();
    for tail in ["/.docker/config.json", "/.config/gh/hosts.yml"] {
        if lower.ends_with(tail) {
            return Err(invalid("Access to this credential file is restricted"));
        }
    }
    Ok(())
}

fn clean_allowed(path: &str) -> Result<String, SshCoreError> {
    let p = clean_path(path)?;
    check_allowed(&p)?;
    Ok(p)
}

fn name_of(path: &str) -> String {
    path.rsplit('/').next().unwrap_or("").to_string()
}

fn parent_of(path: &str) -> String {
    match path.rsplit_once('/') {
        Some(("", _)) | None => "/".to_string(),
        Some((p, _)) => p.to_string(),
    }
}

/// Shell function that prints `type<TAB>size<TAB>mtime<TAB>mode` of `$1` (symlinks followed), for GNU and BSD `stat`.
const STAT_FN: &str = r#"TAB=$(printf '\t')
if stat -c %s . >/dev/null 2>&1; then
  st() { stat -L -c "%F${TAB}%s${TAB}%Y${TAB}%a" -- "$1" 2>/dev/null; }
else
  st() { stat -L -f "%HT${TAB}%z${TAB}%m${TAB}%Lp" -- "$1" 2>/dev/null; }
fi
"#;

fn list_script() -> String {
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

fn stat_script() -> String {
    format!(
        r#"{STAT_FN}p=$(realpath -- "$1" 2>/dev/null || readlink -f -- "$1" 2>/dev/null)
[ -n "$p" ] || {{ printf 'FS_ERR=notfound\n'; exit 0; }}
l=$(st "$p") || {{ printf 'FS_ERR=notfound\n'; exit 0; }}
printf 'FS_PATH=%s\nE\t%s\tx\n' "$p" "$l"
printf 'FS_END\n'"#
    )
}

struct Parsed {
    dir: Option<String>,
    entries: Vec<(String, RemoteFileKind, u64, i64, u32)>,
    error: Option<String>,
    complete: bool,
}

/// Header lines (`FS_*`) are only trusted before the first entry line, so a file name that contains
/// a newline followed by `FS_ERR=...` cannot change the result.
fn parse_fs_output(output: &str) -> Parsed {
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

fn parse_listing(
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

fn parse_stat(output: &str) -> Result<RemoteEntry, SshCoreError> {
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

fn download_command(path: &str) -> String {
    format!(
        "sh -c {}",
        shell_quote(&format!(
            "[ -f {q} ] && [ -r {q} ] || exit 3; exec cat -- {q}",
            q = shell_quote(path)
        ))
    )
}

fn upload_command(path: &str, overwrite: bool) -> String {
    let q = shell_quote(path);
    let tmp = shell_quote(&format!("{path}.part"));
    let check = if overwrite {
        "[ ! -d \"$d\" ]".to_string()
    } else {
        "[ ! -e \"$d\" ]".to_string()
    };
    let script = format!(
        "d={q}; t={tmp}; umask 077; {check} || exit 4; cat > \"$t\" || {{ rm -f \"$t\"; exit 1; }}; \
         {check} || {{ rm -f \"$t\"; exit 4; }}; mv -f \"$t\" \"$d\" || {{ rm -f \"$t\"; exit 1; }}"
    );
    format!("sh -c {}", shell_quote(&script))
}

fn exit_error(code: u32, path: &str) -> SshCoreError {
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

#[derive(uniffi::Object)]
pub struct RemoteFsClient {
    config: RemoteServerConfig,
    conn: tokio::sync::Mutex<Option<ClientHandle>>,
}

impl RemoteFsClient {
    async fn open(
        &self,
        command: &str,
    ) -> Result<russh::Channel<russh::client::Msg>, SshCoreError> {
        for attempt in 0..2 {
            let mut slot = self.conn.lock().await;
            if slot.is_none() {
                let c = &self.config;
                let conn = connect_authenticated(
                    &Endpoint {
                        host: &c.host,
                        port: c.port,
                        username: &c.username,
                        private_key_pem: &c.private_key_pem,
                        pinned_host_key: c.pinned_host_key.as_deref(),
                    },
                    Duration::from_secs(10),
                    2,
                )
                .await?;
                *slot = Some(conn.handle);
            }
            if let Some(h) = slot.as_ref() {
                match exec_start(h, command).await {
                    Ok(ch) => return Ok(ch),
                    Err(e) if attempt == 1 => return Err(e),
                    Err(_) => *slot = None,
                }
            }
        }
        Err(SshCoreError::NotConnected)
    }

    async fn run(&self, command: &str) -> Result<crate::transport::ExecOutput, SshCoreError> {
        let ch = self.open(command).await?;
        collect_output(ch, DEFAULT_EXEC_TIMEOUT).await
    }

    async fn resolve(&self, path: &str) -> Result<String, SshCoreError> {
        let out = self.run(&build_resolve_path_command(path)).await?;
        let resolved = out.text();
        let resolved = resolved.trim();
        if resolved.is_empty() {
            return Err(SshCoreError::FileNotFound {
                path: path.to_string(),
            });
        }
        clean_allowed(resolved)
    }

    async fn run_checked(&self, script: String, path: &str) -> Result<(), SshCoreError> {
        let out = self.run(&format!("sh -c {}", shell_quote(&script))).await?;
        match out.exit_status {
            Some(0) => Ok(()),
            Some(c) => Err(exit_error(c, path)),
            None => Err(SshCoreError::IoError {
                reason: "The host did not report a result".to_string(),
            }),
        }
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl RemoteFsClient {
    /// A pinned host key is mandatory: this client never trusts a key on first use.
    #[uniffi::constructor]
    pub fn new(config: RemoteServerConfig) -> Result<Arc<Self>, SshCoreError> {
        if config.pinned_host_key.as_deref().is_none_or(str::is_empty) {
            return Err(invalid("The host key must be trusted in the app first"));
        }
        Ok(Arc::new(RemoteFsClient {
            config,
            conn: tokio::sync::Mutex::new(None),
        }))
    }

    pub async fn close(&self) {
        if let Some(h) = self.conn.lock().await.take() {
            let _ = h
                .disconnect(russh::Disconnect::ByApplication, "", "English")
                .await;
        }
    }

    /// `$HOME` of the login user: the root of the server in the Files app.
    pub async fn home_directory(&self) -> Result<String, SshCoreError> {
        let out = self.run("printf 'FS_HOME=%s\\n' \"$HOME\"").await?;
        let text = out.text();
        let home = text
            .lines()
            .find_map(|l| l.strip_prefix("FS_HOME="))
            .ok_or_else(|| SshCoreError::IoError {
                reason: "The host did not report the home folder".to_string(),
            })?;
        let home = clean_path(home)?;
        validate_remote_dir_path(&home)?;
        Ok(home)
    }

    pub async fn stat_remote(&self, path: String) -> Result<RemoteEntry, SshCoreError> {
        let path = clean_path(&path)?;
        let script = stat_script();
        let out = self
            .run(&format!(
                "sh -c {} sh {}",
                shell_quote(&script),
                shell_quote(&path)
            ))
            .await?;
        let mut e = parse_stat(&out.text()).map_err(|e| match e {
            SshCoreError::FileNotFound { .. } => SshCoreError::FileNotFound { path: path.clone() },
            e => e,
        })?;
        // The root of a share may be asked for by a link; the user-facing name stays the requested one.
        if path == "/" {
            e.name = "/".to_string();
        }
        Ok(e)
    }

    pub async fn list_remote_entries(
        &self,
        path: String,
        include_hidden: bool,
    ) -> Result<Vec<RemoteEntry>, SshCoreError> {
        let path = clean_allowed(&path)?;
        let script = list_script();
        let out = self
            .run(&format!(
                "sh -c {} sh {}",
                shell_quote(&script),
                shell_quote(&path)
            ))
            .await?;
        parse_listing(&out.text(), include_hidden).map(|(_, e)| e)
    }

    /// Streams a regular file to `dest_local_path` (written as `<dest>.part`, renamed when complete).
    pub async fn download_remote_file(
        &self,
        path: String,
        dest_local_path: String,
        max_bytes: u64,
        progress: Box<dyn TransferProgress>,
    ) -> Result<TransferResult, SshCoreError> {
        let requested = clean_allowed(&path)?;
        let resolved = self.resolve(&requested).await?;
        let meta = self.stat_remote(resolved.clone()).await?;
        if meta.kind != RemoteFileKind::File {
            return Err(SshCoreError::NotARegularFile { path: resolved });
        }
        if meta.size > max_bytes {
            return Err(SshCoreError::TooLarge { limit: max_bytes });
        }
        let part = format!("{dest_local_path}.part");
        let result = self
            .stream_to_file(&resolved, &part, max_bytes, progress.as_ref())
            .await;
        match result {
            Ok(n) => {
                tokio::fs::rename(&part, &dest_local_path)
                    .await
                    .map_err(|e| SshCoreError::IoError {
                        reason: e.to_string(),
                    })?;
                Ok(TransferResult { bytes: n })
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&part).await;
                Err(e)
            }
        }
    }

    /// Sends a local file to `dest_path`; without `overwrite` an existing file is never touched.
    pub async fn upload_remote_file(
        &self,
        src_local_path: String,
        dest_path: String,
        overwrite: bool,
        progress: Box<dyn TransferProgress>,
    ) -> Result<TransferResult, SshCoreError> {
        let dest = clean_allowed(&dest_path)?;
        let parent = self.resolve(&parent_of(&dest)).await?;
        let dest = if parent == "/" {
            format!("/{}", name_of(&dest))
        } else {
            format!("{parent}/{}", name_of(&dest))
        };
        check_allowed(&dest)?;
        let mut file =
            tokio::fs::File::open(&src_local_path)
                .await
                .map_err(|e| SshCoreError::IoError {
                    reason: e.to_string(),
                })?;
        let mut stream = ExecStream::new(self.open(&upload_command(&dest, overwrite)).await?);
        let mut sent = 0u64;
        let mut next_report = PROGRESS_STEP;
        let mut buf = vec![0u8; CHUNK];
        loop {
            if progress.is_cancelled() {
                return Err(SshCoreError::Cancelled);
            }
            let n = file
                .read(&mut buf)
                .await
                .map_err(|e| SshCoreError::IoError {
                    reason: e.to_string(),
                })?;
            if n == 0 {
                break;
            }
            // A host command that exited early (target exists, no permission) stops reading, and
            // the send window never reopens: ask for its exit status instead of waiting forever.
            let sent_ok = matches!(
                tokio::time::timeout(WRITE_STALL, stream.write(buf[..n].to_vec())).await,
                Ok(Ok(()))
            );
            if !sent_ok {
                return Err(
                    match tokio::time::timeout(Duration::from_secs(5), stream.exit_status()).await {
                        Ok(Some(c)) if c != 0 => exit_error(c, &dest),
                        _ => SshCoreError::Timeout,
                    },
                );
            }
            sent += n as u64;
            if sent >= next_report {
                progress.on_progress(sent);
                next_report = sent + PROGRESS_STEP;
            }
        }
        stream.eof().await;
        let mut exit = None;
        loop {
            match tokio::time::timeout(IDLE_LIMIT, stream.next()).await {
                Ok(StreamEvent::Exit(c)) => exit = Some(c),
                Ok(StreamEvent::Data(_)) => {}
                Ok(StreamEvent::End) => break,
                Err(_) => return Err(SshCoreError::Timeout),
            }
        }
        stream.finish().await;
        match exit {
            Some(0) => {
                progress.on_progress(sent);
                Ok(TransferResult { bytes: sent })
            }
            Some(c) => Err(exit_error(c, &dest)),
            None => Err(SshCoreError::IoError {
                reason: "The host did not confirm the upload".to_string(),
            }),
        }
    }

    pub async fn make_remote_dir(&self, path: String) -> Result<(), SshCoreError> {
        let p = clean_allowed(&path)?;
        self.run_checked(format!("mkdir -- {}", shell_quote(&p)), &p)
            .await
    }

    /// Renames or moves `from` to `to`; an existing target is never replaced.
    pub async fn rename_remote(&self, from: String, to: String) -> Result<(), SshCoreError> {
        let (a, b) = (clean_allowed(&from)?, clean_allowed(&to)?);
        if a == "/" || b == "/" {
            return Err(invalid("Cannot move the root"));
        }
        let script = format!(
            "[ -e {a} ] || exit 3; [ ! -e {b} ] && [ ! -L {b} ] || exit 4; mv -- {a} {b}",
            a = shell_quote(&a),
            b = shell_quote(&b)
        );
        self.run_checked(script, &b).await
    }

    /// Deletes a file or an empty folder. Never recursive.
    pub async fn delete_remote(&self, path: String) -> Result<(), SshCoreError> {
        let p = clean_allowed(&path)?;
        if p == "/" {
            return Err(invalid("Cannot delete the root"));
        }
        let q = shell_quote(&p);
        self.run_checked(
            format!("if [ -d {q} ] && [ ! -L {q} ]; then rmdir -- {q}; else rm -- {q}; fi"),
            &p,
        )
        .await
    }
}

impl RemoteFsClient {
    async fn stream_to_file(
        &self,
        path: &str,
        part: &str,
        max_bytes: u64,
        progress: &dyn TransferProgress,
    ) -> Result<u64, SshCoreError> {
        let mut file = tokio::fs::File::create(part)
            .await
            .map_err(|e| SshCoreError::IoError {
                reason: e.to_string(),
            })?;
        let mut stream = ExecStream::new(self.open(&download_command(path)).await?);
        let mut written = 0u64;
        let mut next_report = PROGRESS_STEP;
        let mut exit = None;
        loop {
            if progress.is_cancelled() {
                return Err(SshCoreError::Cancelled);
            }
            match tokio::time::timeout(IDLE_LIMIT, stream.next()).await {
                Ok(StreamEvent::Data(d)) => {
                    written += d.len() as u64;
                    if written > max_bytes {
                        return Err(SshCoreError::TooLarge { limit: max_bytes });
                    }
                    file.write_all(&d)
                        .await
                        .map_err(|e| SshCoreError::IoError {
                            reason: e.to_string(),
                        })?;
                    if written >= next_report {
                        progress.on_progress(written);
                        next_report = written + PROGRESS_STEP;
                    }
                }
                Ok(StreamEvent::Exit(c)) => exit = Some(c),
                Ok(StreamEvent::End) => break,
                Err(_) => return Err(SshCoreError::Timeout),
            }
        }
        stream.finish().await;
        file.flush().await.map_err(|e| SshCoreError::IoError {
            reason: e.to_string(),
        })?;
        match exit {
            Some(0) => {
                progress.on_progress(written);
                Ok(written)
            }
            Some(c) => Err(exit_error(c, path)),
            None => Err(SshCoreError::IoError {
                reason: "The host did not confirm the end of the file".to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
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
            private_key_pem: String::new(),
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
