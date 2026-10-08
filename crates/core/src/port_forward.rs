use crate::error::SshCoreError;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

/// Information about a running or requested port forward.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PortForwardInfo {
    pub remote_port: u16,
    pub local_port: u16,
    pub remote_host: String,
    pub active_connections: u32,
    pub is_running: bool,
}

/// Handle returned to Swift/Kotlin to control the lifecycle of a forwarded port.
#[derive(uniffi::Object)]
pub struct PortForwardHandle {
    pub remote_port: u16,
    pub local_port: u16,
    pub remote_host: String,
    shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
    active_connections: Arc<AtomicUsize>,
    is_running: Arc<AtomicBool>,
}

#[uniffi::export]
impl PortForwardHandle {
    /// Returns the local port bound on 127.0.0.1.
    pub fn get_local_port(&self) -> u16 {
        self.local_port
    }

    /// Returns the remote target port on the host.
    pub fn get_remote_port(&self) -> u16 {
        self.remote_port
    }

    /// Returns the full local HTTP URL for the mobile WebView to load.
    pub fn get_local_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.local_port)
    }

    /// Returns current number of open proxied connections.
    pub fn get_active_connections(&self) -> u32 {
        self.active_connections.load(Ordering::SeqCst) as u32
    }

    /// Checks if the tunnel is currently running.
    pub fn is_active(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }

    /// Stops the local listener and terminates all active TCP forward streams.
    pub fn stop(&self) {
        if self.is_running.swap(false, Ordering::SeqCst) {
            if let Ok(mut lock) = self.shutdown_tx.lock() {
                if let Some(tx) = lock.take() {
                    let _ = tx.send(());
                }
            }
        }
    }
}

impl PortForwardHandle {
    pub fn new(
        remote_port: u16,
        local_port: u16,
        remote_host: String,
        shutdown_tx: oneshot::Sender<()>,
        active_connections: Arc<AtomicUsize>,
        is_running: Arc<AtomicBool>,
    ) -> Self {
        Self {
            remote_port,
            local_port,
            remote_host,
            shutdown_tx: Mutex::new(Some(shutdown_tx)),
            active_connections,
            is_running,
        }
    }

    pub fn to_info(&self) -> PortForwardInfo {
        PortForwardInfo {
            remote_port: self.remote_port,
            local_port: self.local_port,
            remote_host: self.remote_host.clone(),
            active_connections: self.get_active_connections(),
            is_running: self.is_active(),
        }
    }
}

/// Pumps bidirectional data between a local TCP stream and a remote SSH direct-tcpip channel.
pub async fn pump_tcp_and_ssh(
    mut tcp_stream: tokio::net::TcpStream,
    mut channel: russh::Channel<russh::client::Msg>,
) -> Result<(), SshCoreError> {
    use russh::ChannelMsg;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let (mut tcp_read, mut tcp_write) = tcp_stream.split();
    let mut tcp_buf = vec![0u8; 8192];

    loop {
        tokio::select! {
            // Read from local TCP and send to remote SSH
            read_res = tcp_read.read(&mut tcp_buf) => {
                match read_res {
                    Ok(0) => {
                        let _ = channel.eof().await;
                        break;
                    }
                    Ok(n) => {
                        if let Err(e) = channel.data(&tcp_buf[..n]).await {
                            return Err(SshCoreError::PortForwardFailed { reason: e.to_string() });
                        }
                    }
                    Err(e) => {
                        return Err(SshCoreError::IoError { reason: e.to_string() });
                    }
                }
            }
            // Read from remote SSH and write to local TCP
            maybe_msg = channel.wait() => {
                match maybe_msg {
                    Some(ChannelMsg::Data { ref data }) => {
                        if let Err(e) = tcp_write.write_all(data).await {
                            return Err(SshCoreError::IoError { reason: e.to_string() });
                        }
                    }
                    Some(ChannelMsg::ExtendedData { ref data, .. }) => {
                        if let Err(e) = tcp_write.write_all(data).await {
                            return Err(SshCoreError::IoError { reason: e.to_string() });
                        }
                    }
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    let _ = tcp_write.shutdown().await;
    let _ = channel.close().await;
    Ok(())
}

/// Spawns a local loopback listener on 127.0.0.1 and proxies incoming connections to the remote host over SSH.
pub(crate) async fn start_port_forward_listener(
    ssh_handle: Arc<tokio::sync::Mutex<Option<russh::client::Handle<crate::session::SshHandler>>>>,
    remote_host: String,
    remote_port: u16,
    requested_local_port: Option<u16>,
) -> Result<Arc<PortForwardHandle>, SshCoreError> {
    let local_port = requested_local_port.unwrap_or(0);
    let bind_addr = format!("127.0.0.1:{}", local_port);

    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AddrInUse {
                SshCoreError::LocalPortInUse { port: local_port }
            } else {
                SshCoreError::PortForwardFailed {
                    reason: format!("Failed to bind local loopback port {}: {}", local_port, e),
                }
            }
        })?;

    let actual_local_port = listener
        .local_addr()
        .map_err(|e| SshCoreError::IoError {
            reason: e.to_string(),
        })?
        .port();

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let active_connections = Arc::new(AtomicUsize::new(0));
    let is_running = Arc::new(AtomicBool::new(true));

    let handle = Arc::new(PortForwardHandle::new(
        remote_port,
        actual_local_port,
        remote_host.clone(),
        shutdown_tx,
        Arc::clone(&active_connections),
        Arc::clone(&is_running),
    ));

    let active_conn_clone = Arc::clone(&active_connections);
    let is_running_clone = Arc::clone(&is_running);
    let target_host = remote_host.clone();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => {
                    tracing::info!(
                        "Port forward {} -> {}:{} received shutdown signal",
                        actual_local_port, target_host, remote_port
                    );
                    break;
                }
                accept_res = listener.accept() => {
                    match accept_res {
                        Ok((tcp_stream, peer_addr)) => {
                            if active_conn_clone.load(Ordering::SeqCst) >= 32 {
                                tracing::warn!(
                                    "Max concurrent port forward connections reached (32), rejecting {}",
                                    peer_addr
                                );
                                drop(tcp_stream);
                                continue;
                            }

                            let ssh = Arc::clone(&ssh_handle);
                            let rem_host = target_host.clone();
                            let active_counter = Arc::clone(&active_conn_clone);

                            tokio::spawn(async move {
                                active_counter.fetch_add(1, Ordering::SeqCst);
                                let peer_port = peer_addr.port() as u32;

                                let channel_res: Result<russh::Channel<russh::client::Msg>, SshCoreError> = {
                                    let mut lock = ssh.lock().await;
                                    match lock.as_mut() {
                                        Some(client) => client
                                            .channel_open_direct_tcpip(
                                                &rem_host,
                                                remote_port as u32,
                                                "127.0.0.1",
                                                peer_port,
                                            )
                                            .await
                                            .map_err(|e| SshCoreError::PortForwardFailed {
                                                reason: e.to_string(),
                                            }),
                                        None => Err(SshCoreError::NotConnected),
                                    }
                                };

                                match channel_res {
                                    Ok(channel) => {
                                        let _ = pump_tcp_and_ssh(tcp_stream, channel).await;
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "Failed to open direct-tcpip channel for {}:{}: {}",
                                            rem_host, remote_port, e
                                        );
                                    }
                                }
                                active_counter.fetch_sub(1, Ordering::SeqCst);
                            });
                        }
                        Err(e) => {
                            tracing::warn!("Port forward accept error on port {}: {}", actual_local_port, e);
                            break;
                        }
                    }
                }
            }
        }

        is_running_clone.store(false, Ordering::SeqCst);
    });

    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_port_forward_handle_lifecycle() {
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let active = Arc::new(AtomicUsize::new(2));
        let running = Arc::new(AtomicBool::new(true));

        let handle = PortForwardHandle::new(
            3000,
            13000,
            "127.0.0.1".to_string(),
            shutdown_tx,
            active,
            running,
        );

        assert_eq!(handle.get_remote_port(), 3000);
        assert_eq!(handle.get_local_port(), 13000);
        assert_eq!(handle.get_local_url(), "http://127.0.0.1:13000");
        assert_eq!(handle.get_active_connections(), 2);
        assert!(handle.is_active());

        let info = handle.to_info();
        assert_eq!(info.remote_port, 3000);
        assert_eq!(info.local_port, 13000);
        assert_eq!(info.active_connections, 2);
        assert!(info.is_running);

        handle.stop();
        assert!(!handle.is_active());
        assert!(shutdown_rx.try_recv().is_ok());

        let info_stopped = handle.to_info();
        assert!(!info_stopped.is_running);
    }
}
