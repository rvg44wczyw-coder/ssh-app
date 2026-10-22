//! SSH transport shared by the interactive session and the one-shot exec calls: connecting,
//! host-key pinning, public-key authentication and bounded command output.

use crate::error::SshCoreError;
use crate::keys::parse_private_key;
use russh::client::{self, Handler};
use russh::keys::{PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::ChannelMsg;
use ssh_key::HashAlg;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(crate) type ClientHandle = client::Handle<SshHandler>;

/// Upper bound for the output of one exec call. A runaway remote command (or a hostile shell rc)
/// must not be able to grow the client's memory until the OS kills the app.
pub(crate) const MAX_EXEC_OUTPUT: usize = 4 * 1024 * 1024;

/// A command that never answers must not block the caller (and its UI) forever.
pub(crate) const DEFAULT_EXEC_TIMEOUT: Duration = Duration::from_secs(120);

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Verifies the server's host key against a pinned SHA-256 fingerprint (`SHA256:...`, the format
/// `ssh-keygen -lf` prints). Without a pin the first key is accepted and recorded so the caller can
/// pin it (trust on first use); with a pin any other key is refused before authentication.
pub(crate) struct SshHandler {
    pinned: Option<String>,
    observed: Arc<Mutex<Option<String>>>,
}

impl SshHandler {
    fn new(pinned: Option<String>) -> (Self, Arc<Mutex<Option<String>>>) {
        let observed = Arc::new(Mutex::new(None));
        (
            Self {
                pinned,
                observed: Arc::clone(&observed),
            },
            observed,
        )
    }
}

fn fingerprint_of(key: &PublicKeyOrCertificate) -> String {
    match key {
        PublicKeyOrCertificate::PublicKey { key, .. } => key.fingerprint(HashAlg::Sha256),
        PublicKeyOrCertificate::Certificate(cert) => cert.public_key().fingerprint(HashAlg::Sha256),
    }
    .to_string()
}

impl Handler for SshHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let seen = fingerprint_of(server_public_key);
        *self
            .observed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(seen.clone());
        Ok(self.pinned.as_ref().is_none_or(|p| *p == seen))
    }
}

pub(crate) struct Connection {
    pub handle: ClientHandle,
    /// Fingerprint of the key the server presented (what to pin on first use).
    pub host_key: String,
}

pub(crate) struct Endpoint<'a> {
    pub host: &'a str,
    pub port: u16,
    pub username: &'a str,
    pub private_key_pem: &'a str,
    pub pinned_host_key: Option<&'a str>,
}

/// Connects (10 s limit) and checks the host key against `pinned`; nothing is authenticated yet.
async fn open_connection(
    host: &str,
    port: u16,
    pinned: Option<&str>,
    keepalive: Duration,
    keepalive_max: usize,
) -> Result<Connection, SshCoreError> {
    let config = Arc::new(client::Config {
        keepalive_interval: Some(keepalive),
        keepalive_max,
        ..Default::default()
    });
    let (handler, observed) = SshHandler::new(pinned.map(str::to_string));
    let seen = || {
        observed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    };

    let handle = tokio::time::timeout(
        CONNECT_TIMEOUT,
        client::connect(config, (host, port), handler),
    )
    .await
    .map_err(|_| SshCoreError::Timeout)?
    .map_err(|e| match (pinned, seen()) {
        (Some(expected), Some(actual)) if expected != actual => SshCoreError::HostKeyChanged {
            host: host.to_string(),
            expected: expected.to_string(),
            actual,
        },
        _ => SshCoreError::ConnectionError {
            reason: format!("Failed to connect to {host}:{port}: {e}"),
        },
    })?;
    let host_key = seen().ok_or_else(|| SshCoreError::ConnectionError {
        reason: "The server presented no host key".to_string(),
    })?;
    Ok(Connection { handle, host_key })
}

/// Connects (10 s limit), checks the host key, and authenticates with the Ed25519 key.
pub(crate) async fn connect_authenticated(
    ep: &Endpoint<'_>,
    keepalive: Duration,
    keepalive_max: usize,
) -> Result<Connection, SshCoreError> {
    let key = PrivateKeyWithHashAlg::new(Arc::new(parse_private_key(ep.private_key_pem)?), None);
    let mut conn = open_connection(
        ep.host,
        ep.port,
        ep.pinned_host_key,
        keepalive,
        keepalive_max,
    )
    .await?;
    let auth = conn
        .handle
        .authenticate_publickey(ep.username, key)
        .await
        .map_err(|e| SshCoreError::AuthenticationError {
            username: ep.username.to_string(),
            reason: format!("Authentication request failed: {e}"),
        })?;
    if !auth.success() {
        return Err(SshCoreError::AuthenticationError {
            username: ep.username.to_string(),
            reason: "Host rejected public key authentication".to_string(),
        });
    }
    Ok(conn)
}

/// The host key a server presents, read without authenticating (so it can be shown to the user
/// before anything secret is sent). The connection is closed at once.
pub(crate) async fn probe_host_key(host: &str, port: u16) -> Result<String, SshCoreError> {
    let conn = open_connection(host, port, None, Duration::from_secs(15), 2).await?;
    let _ = conn
        .handle
        .disconnect(russh::Disconnect::ByApplication, "", "en")
        .await;
    Ok(conn.host_key)
}

/// One-time login with a password, for pairing only (feature 44): the host key must already be
/// pinned, and the password is never stored. The caller holds it in a zeroizing buffer.
pub(crate) async fn connect_with_password(
    host: &str,
    port: u16,
    username: &str,
    password: &str,
    pinned_host_key: &str,
) -> Result<Connection, SshCoreError> {
    let mut conn = open_connection(
        host,
        port,
        Some(pinned_host_key),
        Duration::from_secs(15),
        2,
    )
    .await?;
    let auth = conn
        .handle
        .authenticate_password(username, password)
        .await
        .map_err(|e| SshCoreError::AuthenticationError {
            username: username.to_string(),
            reason: format!("Authentication request failed: {e}"),
        })?;
    if !auth.success() {
        return Err(SshCoreError::AuthenticationError {
            username: username.to_string(),
            reason: "Host rejected the password".to_string(),
        });
    }
    Ok(conn)
}

/// What an exec call produced: stdout and stderr interleaved, capped, plus the exit status.
pub(crate) struct ExecOutput {
    pub bytes: Vec<u8>,
    pub exit_status: Option<u32>,
}

impl ExecOutput {
    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).to_string()
    }
}

/// Appends `data` up to `MAX_EXEC_OUTPUT`; returns true when the cap is reached (stop reading).
pub(crate) fn push_capped(output: &mut Vec<u8>, data: &[u8]) -> bool {
    let room = MAX_EXEC_OUTPUT.saturating_sub(output.len());
    output.extend_from_slice(&data[..data.len().min(room)]);
    output.len() >= MAX_EXEC_OUTPUT
}

/// Opens a fresh channel of `handle` and starts `command` on it (no PTY).
pub(crate) async fn exec_start(
    handle: &ClientHandle,
    command: &str,
) -> Result<russh::Channel<client::Msg>, SshCoreError> {
    let channel = handle
        .channel_open_session()
        .await
        .map_err(|e| SshCoreError::ChannelError {
            reason: format!("Failed to open session channel: {e}"),
        })?;
    channel
        .exec(true, command)
        .await
        .map_err(|e| SshCoreError::ChannelError {
            reason: format!("Exec command failed: {e}"),
        })?;
    Ok(channel)
}

/// Closes the channel if the read is abandoned (the caller's future was dropped, e.g. the user
/// pressed Stop): without it the remote command keeps running with nobody listening.
pub(crate) struct CloseOnDrop(pub(crate) Option<russh::Channel<client::Msg>>);

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        if let (Some(ch), Ok(rt)) = (self.0.take(), tokio::runtime::Handle::try_current()) {
            rt.spawn(async move {
                let _ = ch.eof().await;
                let _ = ch.close().await;
            });
        }
    }
}

/// Reads the channel to its end (or the output cap), giving up with `Timeout` after `limit`.
/// The channel is closed either way, including when this future is dropped before it finishes;
/// a dropped connection ends the read.
pub(crate) async fn collect_output(
    channel: russh::Channel<client::Msg>,
    limit: Duration,
) -> Result<ExecOutput, SshCoreError> {
    let mut guard = CloseOnDrop(Some(channel));
    let read = async {
        let mut bytes = Vec::new();
        let mut exit_status = None;
        while let Some(msg) = match guard.0.as_mut() {
            Some(ch) => ch.wait().await,
            None => None,
        } {
            match msg {
                ChannelMsg::Data { ref data } | ChannelMsg::ExtendedData { ref data, .. } => {
                    if push_capped(&mut bytes, data) {
                        break;
                    }
                }
                ChannelMsg::ExitStatus { exit_status: s } => exit_status = Some(s),
                // Eof can arrive before the exit status; only Close ends the exchange.
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        ExecOutput { bytes, exit_status }
    };
    let out = tokio::time::timeout(limit, read).await;
    if let Some(ch) = guard.0.take() {
        let _ = ch.eof().await;
        let _ = ch.close().await;
    }
    out.map_err(|_| SshCoreError::Timeout)
}

/// One step of a streamed exec call.
pub(crate) enum StreamEvent {
    /// Raw stdout bytes (stderr is not part of the stream).
    Data(Vec<u8>),
    Exit(u32),
    End,
}

/// A running exec channel read piece by piece, for payloads too large for `collect_output`
/// (file transfers). Dropping it closes the channel, so a cancelled transfer stops the remote command.
pub(crate) struct ExecStream {
    guard: CloseOnDrop,
}

impl ExecStream {
    pub(crate) fn new(channel: russh::Channel<client::Msg>) -> Self {
        ExecStream {
            guard: CloseOnDrop(Some(channel)),
        }
    }

    pub(crate) async fn next(&mut self) -> StreamEvent {
        loop {
            let msg = match self.guard.0.as_mut() {
                Some(ch) => ch.wait().await,
                None => None,
            };
            match msg {
                Some(ChannelMsg::Data { ref data }) => return StreamEvent::Data(data.to_vec()),
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    return StreamEvent::Exit(exit_status)
                }
                Some(ChannelMsg::Close) | None => return StreamEvent::End,
                Some(_) => {}
            }
        }
    }

    /// Waits for the exit status (None when the channel ends without one).
    pub(crate) async fn exit_status(&mut self) -> Option<u32> {
        loop {
            match self.next().await {
                StreamEvent::Exit(c) => return Some(c),
                StreamEvent::Data(_) => {}
                StreamEvent::End => return None,
            }
        }
    }

    pub(crate) async fn write(&self, chunk: Vec<u8>) -> Result<(), SshCoreError> {
        match self.guard.0.as_ref() {
            Some(ch) => ch
                .data_bytes(chunk)
                .await
                .map_err(|e| SshCoreError::ChannelError {
                    reason: format!("Sending data failed: {e}"),
                }),
            None => Err(SshCoreError::NotConnected),
        }
    }

    pub(crate) async fn eof(&self) {
        if let Some(ch) = self.guard.0.as_ref() {
            let _ = ch.eof().await;
        }
    }

    pub(crate) async fn finish(mut self) {
        if let Some(ch) = self.guard.0.take() {
            let _ = ch.eof().await;
            let _ = ch.close().await;
        }
    }
}

/// `exec_start` + `collect_output` on a connection the caller owns.
pub(crate) async fn exec_collect(
    handle: &ClientHandle,
    command: &str,
    limit: Duration,
) -> Result<ExecOutput, SshCoreError> {
    collect_output(exec_start(handle, command).await?, limit).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_output_is_capped() {
        let mut out = Vec::new();
        assert!(!push_capped(&mut out, &[1u8; 10]));
        let big = vec![2u8; MAX_EXEC_OUTPUT * 2];
        assert!(push_capped(&mut out, &big));
        assert_eq!(out.len(), MAX_EXEC_OUTPUT);
        assert!(push_capped(&mut out, &[3u8; 5]));
        assert_eq!(out.len(), MAX_EXEC_OUTPUT);
    }
}
