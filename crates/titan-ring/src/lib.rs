//! Lock-free SPSC Ring Buffer (Disruptor pattern).
//!
//! This module implements a Single-Producer Single-Consumer ring buffer
//! with cache-line padding to prevent false sharing.

#![no_std]

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU64, Ordering};
use core::mem::MaybeUninit;

/// Default buffer size (must be power of 2).
pub const DEFAULT_BUFFER_SIZE: usize = 1024 * 1024; // 1M entries

/// Padded atomic counter to prevent false sharing.
/// Uses 128-byte alignment to ensure it occupies its own cache line.
#[repr(C, align(128))]
struct PaddedAtomicU64 {
    value: AtomicU64,
}

impl PaddedAtomicU64 {
    const fn new(v: u64) -> Self {
        Self {
            value: AtomicU64::new(v),
        }
    }
}

/// Single-Producer Single-Consumer lock-free ring buffer.
///
/// Uses atomic sequencing inspired by the LMAX Disruptor pattern.
/// The buffer provides wait-free operations for both producer and consumer.
#[repr(C)]
pub struct SpscRing<T: Copy, const N: usize = DEFAULT_BUFFER_SIZE> {
    /// Write cursor (owned by producer).
    write_cursor: PaddedAtomicU64,
    
    /// Cached read position for producer (reduces cache line bouncing).
    cached_read: PaddedAtomicU64,
    
    /// Read cursor (owned by consumer).
    read_cursor: PaddedAtomicU64,
    
    /// Cached write position for consumer.
    cached_write: PaddedAtomicU64,
    
    /// The actual buffer.
    buffer: UnsafeCell<[MaybeUninit<T>; N]>,
}

// SAFETY: Ring buffer is designed for single-producer single-consumer,
// with proper atomic synchronization between the two.
unsafe impl<T: Copy + Send, const N: usize> Send for SpscRing<T, N> {}
unsafe impl<T: Copy + Send, const N: usize> Sync for SpscRing<T, N> {}

impl<T: Copy, const N: usize> SpscRing<T, N> {
    const MASK: u64 = (N - 1) as u64;
    
    /// Create a new ring buffer.
    ///
    /// # Panics
    /// Panics if N is not a power of 2.
    pub fn new() -> Self {
        assert!(N.is_power_of_two(), "Buffer size must be power of 2");
        
        Self {
            write_cursor: PaddedAtomicU64::new(0),
            cached_read: PaddedAtomicU64::new(0),
            read_cursor: PaddedAtomicU64::new(0),
            cached_write: PaddedAtomicU64::new(0),
            buffer: UnsafeCell::new(unsafe { MaybeUninit::uninit().assume_init() }),
        }
    }
    
    /// Get buffer capacity.
    #[inline(always)]
    pub const fn capacity(&self) -> usize {
        N
    }
    
    /// Split into producer and consumer handles.
    ///
    /// # Safety
    /// Must only be called once. Multiple producers or consumers will cause UB.
    pub fn split(&mut self) -> (Producer<'_, T, N>, Consumer<'_, T, N>) {
        (
            Producer { ring: self },
            Consumer { ring: self },
        )
    }
}

impl<T: Copy, const N: usize> Default for SpscRing<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Producer handle (write-only).
pub struct Producer<'a, T: Copy, const N: usize = DEFAULT_BUFFER_SIZE> {
    ring: &'a SpscRing<T, N>,
}

impl<'a, T: Copy, const N: usize> Producer<'a, T, N> {
    /// Attempt to publish a value.
    ///
    /// Returns `false` if buffer is full.
    #[inline(always)]
    pub fn try_publish(&mut self, value: T) -> bool {
        let write_pos = self.ring.write_cursor.value.load(Ordering::Relaxed);
        
        // Check if buffer is full using cached read position
        let cached_read = self.ring.cached_read.value.load(Ordering::Relaxed);
        if write_pos - cached_read >= N as u64 {
            // Refresh cached read position
            let current_read = self.ring.read_cursor.value.load(Ordering::Acquire);
            self.ring.cached_read.value.store(current_read, Ordering::Relaxed);
            
            if write_pos - current_read >= N as u64 {
                return false; // Buffer is actually full
            }
        }
        
        // Write the value
        let idx = (write_pos & SpscRing::<T, N>::MASK) as usize;
        unsafe {
            let buffer = &mut *self.ring.buffer.get();
            buffer[idx].write(value);
        }
        
        // Publish (release barrier ensures writes are visible)
        self.ring.write_cursor.value.store(write_pos + 1, Ordering::Release);
        
        true
    }
    
    /// Publish a value, spinning until space is available.
    #[inline]
    pub fn publish(&mut self, value: T) {
        while !self.try_publish(value) {
            core::hint::spin_loop();
        }
    }
    
    /// Batch publish for efficiency.
    #[inline]
    pub fn publish_batch(&mut self, values: &[T]) {
        for &value in values {
            self.publish(value);
        }
    }
    
    /// Check remaining capacity.
    #[inline]
    pub fn remaining_capacity(&self) -> usize {
        let write_pos = self.ring.write_cursor.value.load(Ordering::Relaxed);
        let read_pos = self.ring.read_cursor.value.load(Ordering::Acquire);
        N - (write_pos - read_pos) as usize
    }
}

/// Consumer handle (read-only).
pub struct Consumer<'a, T: Copy, const N: usize = DEFAULT_BUFFER_SIZE> {
    ring: &'a SpscRing<T, N>,
}

impl<'a, T: Copy, const N: usize> Consumer<'a, T, N> {
    /// Attempt to consume a value.
    ///
    /// Returns `None` if buffer is empty.
    #[inline(always)]
    pub fn try_consume(&mut self) -> Option<T> {
        let read_pos = self.ring.read_cursor.value.load(Ordering::Relaxed);
        
        // Check if buffer is empty using cached write position
        let cached_write = self.ring.cached_write.value.load(Ordering::Relaxed);
        if read_pos >= cached_write {
            // Refresh cached write position
            let current_write = self.ring.write_cursor.value.load(Ordering::Acquire);
            self.ring.cached_write.value.store(current_write, Ordering::Relaxed);
            
            if read_pos >= current_write {
                return None; // Buffer is actually empty
            }
        }
        
        // Read the value
        let idx = (read_pos & SpscRing::<T, N>::MASK) as usize;
        let value = unsafe {
            let buffer = &*self.ring.buffer.get();
            buffer[idx].assume_init_read()
        };
        
        // Acknowledge consumption (release barrier)
        self.ring.read_cursor.value.store(read_pos + 1, Ordering::Release);
        
        Some(value)
    }
    
    /// Consume a value, spinning until one is available (BUSY WAIT).
    #[inline(always)]
    pub fn consume(&mut self) -> T {
        loop {
            if let Some(value) = self.try_consume() {
                return value;
            }
            core::hint::spin_loop();
        }
    }
    
    /// Batch consume for efficiency.
    ///
    /// Returns number of items consumed.
    #[inline]
    pub fn consume_batch(&mut self, buffer: &mut [T]) -> usize {
        let mut count = 0;
        for slot in buffer.iter_mut() {
            match self.try_consume() {
                Some(value) => {
                    *slot = value;
                    count += 1;
                }
                None => break,
            }
        }
        count
    }
    
    /// Check number of items available to consume.
    #[inline]
    pub fn available(&self) -> usize {
        let write_pos = self.ring.write_cursor.value.load(Ordering::Acquire);
        let read_pos = self.ring.read_cursor.value.load(Ordering::Relaxed);
        (write_pos - read_pos) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_single_message() {
        let mut ring: SpscRing<u64, 16> = SpscRing::new();
        let (mut producer, mut consumer) = ring.split();
        
        assert!(producer.try_publish(42));
        assert_eq!(consumer.try_consume(), Some(42));
        assert_eq!(consumer.try_consume(), None);
    }
    
    #[test]
    fn test_fill_drain() {
        let mut ring: SpscRing<u64, 16> = SpscRing::new();
        let (mut producer, mut consumer) = ring.split();
        
        // Fill completely
        for i in 0..16 {
            assert!(producer.try_publish(i), "Failed at {}", i);
        }
        
        // Should be full
        assert!(!producer.try_publish(100));
        
        // Drain
        for i in 0..16 {
            assert_eq!(consumer.try_consume(), Some(i));
        }
        
        // Should be empty
        assert_eq!(consumer.try_consume(), None);
    }
    
    #[test]
    fn test_wrap_around() {
        let mut ring: SpscRing<u64, 4> = SpscRing::new();
        let (mut producer, mut consumer) = ring.split();
        
        // Write and read multiple times, wrapping around
        for round in 0..10 {
            let base = round * 4;
            
            for i in 0..4 {
                assert!(producer.try_publish(base + i));
            }
            
            for i in 0..4 {
                assert_eq!(consumer.try_consume(), Some(base + i));
            }
        }
    }
    
    #[test]
    fn test_remaining_capacity() {
        let mut ring: SpscRing<u64, 8> = SpscRing::new();
        let (mut producer, _consumer) = ring.split();
        
        assert_eq!(producer.remaining_capacity(), 8);
        
        producer.try_publish(1);
        assert_eq!(producer.remaining_capacity(), 7);
        
        producer.try_publish(2);
        producer.try_publish(3);
        assert_eq!(producer.remaining_capacity(), 5);
    }
    
    #[test]
    fn test_available() {
        let mut ring: SpscRing<u64, 8> = SpscRing::new();
        let (mut producer, consumer) = ring.split();
        
        assert_eq!(consumer.available(), 0);
        
        producer.try_publish(1);
        producer.try_publish(2);
        assert_eq!(consumer.available(), 2);
    }
}
