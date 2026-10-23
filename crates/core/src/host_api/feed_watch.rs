use super::*;
use crate::feed::{FeedChange, FeedState};
use crate::feed_read::{FeedCursor, FeedRead};
use crate::feed_stream::{FrameParser, StreamEvent as FeedEvent};

/// Receives what the feed stream reports.
#[uniffi::export(callback_interface)]
pub trait FeedListener: Send + Sync {
    /// Turns changed; the `FeedState` given to `watch_feed` already holds them. `cursor` is where
    /// the host has read to (keep it to resume after the app was in the background).
    fn on_change(&self, change: FeedChange, cursor: FeedCursor);
    /// The last lines of the pane changed (status line, input box, questions, dialogs).
    fn on_live(&self, text: String);
    /// The stream ended (the connection dropped, `stop`, or the host said bye); no more events.
    fn on_closed(&self);
}

/// Handle of a running `watch_feed` stream.
#[derive(uniffi::Object)]
pub struct FeedWatch {
    stop: tokio::sync::Notify,
}

#[uniffi::export]
impl FeedWatch {
    /// Ends the stream (idempotent); `on_closed` follows.
    pub fn stop(&self) {
        self.stop.notify_one();
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl SshSessionHandle {
    /// Streams the feed of a tmux session: new lines of its session file (`remote_path`, the one
    /// `detect_session_transcript_path` found) as changes of `state`, and the tail of the pane as
    /// live text. One exec channel stays open and the host writes only when something changed;
    /// closing the connection (the app in the background) stops it. Resume with the last cursor.
    pub async fn watch_feed(
        &self,
        session_name: String,
        remote_path: String,
        state: Arc<FeedState>,
        cursor: Option<FeedCursor>,
        listener: Box<dyn FeedListener>,
    ) -> Result<Arc<FeedWatch>, SshCoreError> {
        let program = crate::zellij_feed::mux_arg(MultiplexerKind::Tmux, &session_name)?;
        debug_assert_eq!(program, "tmux");
        let command =
            crate::feed_stream::build_stream_command(&session_name, &remote_path, cursor)?;
        self.run_feed_stream(command, state, listener).await
    }

    /// Like `watch_feed` for an Antigravity tab: the conversation is read from the agent's own
    /// database by the pinned host helper (`helper_sha256`, as for `watch_host_agent`). The helper
    /// must be installed; otherwise the stream ends at once and the caller keeps its fallback.
    pub async fn watch_agy_feed(
        &self,
        session_name: String,
        helper_sha256: String,
        state: Arc<FeedState>,
        cursor: Option<FeedCursor>,
        listener: Box<dyn FeedListener>,
    ) -> Result<Arc<FeedWatch>, SshCoreError> {
        let program = crate::zellij_feed::mux_arg(MultiplexerKind::Tmux, &session_name)?;
        debug_assert_eq!(program, "tmux");
        let command =
            crate::feed_stream::build_agy_stream_command(&session_name, &helper_sha256, cursor)?;
        self.run_feed_stream(command, state, listener).await
    }
}

impl SshSessionHandle {
    async fn run_feed_stream(
        &self,
        command: String,
        state: Arc<FeedState>,
        listener: Box<dyn FeedListener>,
    ) -> Result<Arc<FeedWatch>, SshCoreError> {
        let channel = self
            .exec_open(&crate::hostprobe::cancellable_sh(&command))
            .await?;
        let mut stream = crate::transport::ExecStream::new(channel);
        let watch = Arc::new(FeedWatch {
            stop: tokio::sync::Notify::new(),
        });
        let task_watch = Arc::clone(&watch);
        crate::session::RUNTIME.spawn(async move {
            let mut parser = FrameParser::default();
            'outer: loop {
                let step = tokio::select! {
                    s = stream.next() => s,
                    () = task_watch.stop.notified() => break,
                };
                match step {
                    crate::transport::StreamEvent::Data(bytes) => {
                        let Ok(events) = parser.push(&bytes) else {
                            break;
                        };
                        for ev in events {
                            match ev {
                                FeedEvent::Hello { proto } if proto != 1 => break 'outer,
                                FeedEvent::Hello { .. } => {}
                                FeedEvent::Delta(out) => {
                                    // A path or answer that does not pass the checks ends the stream.
                                    let Ok(Some(FeedRead {
                                        reset,
                                        cursor,
                                        text,
                                        ..
                                    })) = crate::feed_read::parse_delta_output(&out)
                                    else {
                                        continue;
                                    };
                                    if reset {
                                        state.reset();
                                    }
                                    let change = state.apply_chunk(text);
                                    listener.on_change(change, cursor);
                                }
                                FeedEvent::Live { text, .. } => listener.on_live(text),
                                FeedEvent::Bye { .. } => break 'outer,
                            }
                        }
                    }
                    crate::transport::StreamEvent::Exit(_) => {}
                    crate::transport::StreamEvent::End => break,
                }
            }
            stream.finish().await;
            listener.on_closed();
        });
        Ok(watch)
    }
}
