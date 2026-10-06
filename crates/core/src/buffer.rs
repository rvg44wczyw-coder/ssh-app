use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub const DEFAULT_FLUSH_INTERVAL: Duration = Duration::from_millis(16);
pub const DEFAULT_MAX_BATCH_SIZE: usize = 32 * 1024; // 32 KB

/// A thread-safe, asynchronous throttling buffer designed to coalesce high-throughput terminal output
/// before dispatching across the UniFFI boundary to SwiftTerm.
pub struct OutputThrottler {
    sender: mpsc::Sender<Vec<u8>>,
    task_handle: Option<JoinHandle<()>>,
}

impl OutputThrottler {
    pub fn new<F>(flush_interval: Duration, max_batch_size: usize, on_flush: F) -> Self
    where
        F: Fn(Vec<u8>) + Send + Sync + 'static,
    {
        let (sender, mut receiver) = mpsc::channel::<Vec<u8>>(1024);
        let callback = Arc::new(on_flush);

        let task_handle = tokio::spawn(async move {
            let mut buffer = Vec::with_capacity(max_batch_size);
            let mut interval = tokio::time::interval(flush_interval);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                tokio::select! {
                    biased;

                    maybe_chunk = receiver.recv() => {
                        match maybe_chunk {
                            Some(chunk) => {
                                buffer.extend_from_slice(&chunk);
                                if buffer.len() >= max_batch_size {
                                    let batch = std::mem::replace(&mut buffer, Vec::with_capacity(max_batch_size));
                                    callback(batch);
                                }
                            }
                            None => {
                                // Channel closed, flush remaining bytes
                                if !buffer.is_empty() {
                                    callback(buffer);
                                }
                                break;
                            }
                        }
                    }

                    _ = interval.tick() => {
                        if !buffer.is_empty() {
                            let batch = std::mem::replace(&mut buffer, Vec::with_capacity(max_batch_size));
                            callback(batch);
                        }
                    }
                }
            }
        });

        Self {
            sender,
            task_handle: Some(task_handle),
        }
    }

    /// Queues a chunk of incoming terminal data.
    pub async fn push(&self, data: Vec<u8>) -> Result<(), mpsc::error::SendError<Vec<u8>>> {
        self.sender.send(data).await
    }

    /// Non-blocking try_push from synchronous or polling contexts.
    pub fn try_push(&self, data: Vec<u8>) -> Result<(), mpsc::error::TrySendError<Vec<u8>>> {
        self.sender.try_send(data)
    }

    /// Stops the throttler and waits for final buffered output to flush.
    pub async fn shutdown(&mut self) {
        if let Some(handle) = self.task_handle.take() {
            // Dropping sender triggers channel close
            handle.abort();
        }
    }
}

impl Drop for OutputThrottler {
    fn drop(&mut self) {
        if let Some(handle) = self.task_handle.take() {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    #[tokio::test]
    async fn test_throttler_coalesces_batches() {
        let collected = Arc::new(Mutex::new(Vec::new()));
        let call_count = Arc::new(AtomicUsize::new(0));

        let collected_clone = Arc::clone(&collected);
        let call_count_clone = Arc::clone(&call_count);

        let throttler = OutputThrottler::new(Duration::from_millis(50), 100, move |batch| {
            call_count_clone.fetch_add(1, Ordering::SeqCst);
            collected_clone.lock().unwrap().extend_from_slice(&batch);
        });

        // Push 10 small chunks rapidly
        for i in 0..10 {
            throttler.push(vec![i as u8]).await.unwrap();
        }

        // Wait for interval tick
        tokio::time::sleep(Duration::from_millis(100)).await;

        let data = collected.lock().unwrap().clone();
        assert_eq!(data, vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        // All 10 small chunks should have been coalesced into 1 callback call
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_throttler_flushes_on_max_batch_size() {
        let collected = Arc::new(Mutex::new(Vec::new()));
        let call_count = Arc::new(AtomicUsize::new(0));

        let collected_clone = Arc::clone(&collected);
        let call_count_clone = Arc::clone(&call_count);

        let throttler = OutputThrottler::new(
            Duration::from_secs(5), // Long interval
            10,                     // Low threshold
            move |batch| {
                call_count_clone.fetch_add(1, Ordering::SeqCst);
                collected_clone.lock().unwrap().extend_from_slice(&batch);
            },
        );

        // Push 12 bytes: should trigger immediate flush of 10+ bytes without waiting 5 seconds
        throttler.push(vec![1; 12]).await.unwrap();

        // Give async task a brief moment to process
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert_eq!(call_count.load(Ordering::SeqCst), 1);
        let len = collected.lock().unwrap().len();
        assert_eq!(len, 12);
    }
}
