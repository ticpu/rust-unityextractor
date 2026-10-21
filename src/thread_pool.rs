use log::{debug, trace, warn};
use std::io;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::archive_operations::AssetWriteError;
use crate::memory_tracker::MemoryTracker;

pub type WriteTask = Box<
    dyn FnOnce() -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<(), AssetWriteError>> + Send>,
        > + Send
        + 'static,
>;

pub struct ThreadPool {
    sender: mpsc::UnboundedSender<WriteTask>,
    handles: Vec<JoinHandle<usize>>,
    memory_tracker: Arc<MemoryTracker>,
}

impl ThreadPool {
    pub fn new(num_threads: usize, memory_tracker: Arc<MemoryTracker>) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel::<WriteTask>();
        let receiver = Arc::new(tokio::sync::Mutex::new(receiver));

        let mut handles = Vec::with_capacity(num_threads);

        for thread_id in 0..num_threads {
            let receiver = receiver.clone();
            let handle = tokio::spawn(async move {
                debug!("Writer thread {thread_id} started");
                let mut failures = 0;

                loop {
                    let task = {
                        let mut rx = receiver.lock().await;
                        match rx.recv().await {
                            Some(task) => task,
                            None => {
                                debug!("Writer thread {thread_id} shutting down");
                                break failures;
                            }
                        }
                    };

                    let future = task();
                    if let Err(e) = future.await {
                        warn!("Write task failed: {e}");
                        failures += 1;
                    }
                }
            });
            handles.push(handle);
        }

        Self {
            sender,
            handles,
            memory_tracker,
        }
    }

    /// Whether a task of `task_size` bytes fits in the buffer budget. Room seen
    /// here is still there at queue_task: the archive reader is the only
    /// producer, and writer threads only release memory.
    pub fn has_room(&self, task_size: u64) -> bool {
        self.memory_tracker.can_allocate(task_size)
    }

    /// Queues a task whose room was checked with has_room. Fails only when
    /// every writer thread has stopped.
    pub fn queue_task(&self, task_size: u64, task: WriteTask) -> Result<(), io::Error> {
        self.memory_tracker.reserve(task_size);

        let memory_tracker = self.memory_tracker.clone();
        let wrapped_task = Box::new(move || {
            Box::pin(async move {
                let result = task().await;
                memory_tracker.release(task_size);
                result
            })
                as std::pin::Pin<
                    Box<dyn std::future::Future<Output = Result<(), AssetWriteError>> + Send>,
                >
        });

        if self.sender.send(wrapped_task).is_err() {
            self.memory_tracker.release(task_size);
            return Err(io::Error::other("all writer threads have stopped"));
        }
        trace!("Queued task of size {task_size} bytes");
        Ok(())
    }

    /// Queues `task` if it fits the buffer budget, else runs it here.
    pub async fn queue_or_run(&self, task_size: u64, task: WriteTask) -> Result<(), io::Error> {
        if self.has_room(task_size) {
            return self.queue_task(task_size, task);
        }
        debug!("Queue full, running task of size {task_size} bytes synchronously");
        task().await.map_err(io::Error::other)
    }

    /// Waits for every queued task and returns how many failed, each already
    /// logged; a panicked writer thread counts as one failure.
    pub async fn shutdown(self) -> usize {
        // Close the sender to signal threads to shut down
        drop(self.sender);

        let mut failures = 0;
        for handle in self.handles {
            match handle.await {
                Ok(thread_failures) => failures += thread_failures,
                Err(e) => {
                    warn!("Writer thread panicked: {e}");
                    failures += 1;
                }
            }
        }

        debug!("Thread pool shutdown complete, {failures} failed tasks");
        failures
    }
}
