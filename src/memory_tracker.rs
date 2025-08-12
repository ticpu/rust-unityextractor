use bytesize::ByteSize;
use log::{debug, trace};
use std::sync::atomic::{AtomicU64, Ordering};

/// Memory tracker for queue buffer management
pub struct MemoryTracker {
    used_memory: AtomicU64,
    max_buffer_memory: u64,
}

impl MemoryTracker {
    pub fn new(max_buffer_memory: u64) -> Self {
        Self {
            used_memory: AtomicU64::new(0),
            max_buffer_memory,
        }
    }

    /// Check if we can allocate more memory for buffering
    /// Returns true if: used_memory + size <= max_buffer_memory
    pub fn can_allocate(&self, size: u64) -> bool {
        let current_used = self.used_memory.load(Ordering::Relaxed);

        // Check against our buffer limit
        if current_used + size > self.max_buffer_memory {
            debug!(
                "Buffer limit reached: {} used + {} request > {} limit",
                ByteSize(current_used),
                ByteSize(size),
                ByteSize(self.max_buffer_memory)
            );
            return false;
        }

        true
    }

    /// Reserve memory for a buffer
    pub fn reserve(&self, size: u64) {
        let new_used = self.used_memory.fetch_add(size, Ordering::Relaxed) + size;
        trace!(
            "Reserved {}, total used: {}",
            ByteSize(size),
            ByteSize(new_used)
        );
    }

    /// Release memory when buffer is consumed
    pub fn release(&self, size: u64) {
        let new_used = self.used_memory.fetch_sub(size, Ordering::Relaxed) - size;
        trace!(
            "Released {}, total used: {}",
            ByteSize(size),
            ByteSize(new_used)
        );
    }
}
