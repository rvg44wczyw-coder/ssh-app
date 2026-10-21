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

mod listing;
use listing::*;
mod client;
pub use client::*;
#[cfg(test)]
mod tests;
