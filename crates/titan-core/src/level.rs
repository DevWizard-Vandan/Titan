//! Price level queue management.
//!
//! A price level contains all orders at a specific price,
//! organized as a FIFO queue (price-time priority).

use crate::fixed::Quantity;
use crate::pool::OrderHandle;

/// Maximum orders per price level.
/// Tune based on expected market depth.
pub const MAX_ORDERS_PER_LEVEL: usize = 1024;

/// A single price level in the order book.
///
/// Uses a circular buffer for FIFO order queue, which is cache-friendly
/// and provides O(1) push/pop operations.
#[repr(C)]
pub struct PriceLevel {
    /// Total quantity at this level.
    pub total_qty: Quantity,
    /// Number of orders at this level.
    order_count: u16,
    /// Head of circular buffer (next to dequeue).
    head: u16,
    /// Tail of circular buffer (next insert position).
    tail: u16,
    /// Padding for alignment.
    _padding: u16,
    /// Circular buffer of order handles.
    orders: [OrderHandle; MAX_ORDERS_PER_LEVEL],
}

impl PriceLevel {
    /// Create a new empty price level.
    pub fn new() -> Self {
        Self {
            total_qty: Quantity::ZERO,
            order_count: 0,
            head: 0,
            tail: 0,
            _padding: 0,
            orders: [OrderHandle::INVALID; MAX_ORDERS_PER_LEVEL],
        }
    }
    
    /// Check if level is empty.
    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.order_count == 0
    }
    
    /// Number of orders at this level.
    #[inline(always)]
    pub const fn len(&self) -> usize {
        self.order_count as usize
    }
    
    /// Check if level is full.
    #[inline(always)]
    pub const fn is_full(&self) -> bool {
        self.order_count as usize >= MAX_ORDERS_PER_LEVEL
    }
    
    /// Add order to back of queue.
    ///
    /// Returns `false` if level is full.
    #[inline(always)]
    pub fn push_back(&mut self, handle: OrderHandle, qty: Quantity) -> bool {
        if self.is_full() {
            return false;
        }
        
        self.orders[self.tail as usize] = handle;
        self.tail = ((self.tail as usize + 1) % MAX_ORDERS_PER_LEVEL) as u16;
        self.order_count += 1;
        self.total_qty = self.total_qty.saturating_add(qty);
        true
    }
    
    /// Get front order handle (for matching).
    #[inline(always)]
    pub fn front(&self) -> Option<OrderHandle> {
        if self.is_empty() {
            None
        } else {
            Some(self.orders[self.head as usize])
        }
    }
    
    /// Peek at front order handle without removing.
    #[inline(always)]
    pub fn peek(&self) -> Option<OrderHandle> {
        self.front()
    }
    
    /// Remove front order from queue.
    ///
    /// Note: Does NOT update total_qty. Caller must call reduce_qty separately
    /// if the order was partially or fully filled.
    #[inline(always)]
    pub fn pop_front(&mut self) -> Option<OrderHandle> {
        if self.is_empty() {
            return None;
        }
        
        let handle = self.orders[self.head as usize];
        self.orders[self.head as usize] = OrderHandle::INVALID;
        self.head = ((self.head as usize + 1) % MAX_ORDERS_PER_LEVEL) as u16;
        self.order_count -= 1;
        Some(handle)
    }
    
    /// Update total quantity (after partial or full fill).
    #[inline(always)]
    pub fn reduce_qty(&mut self, qty: Quantity) {
        self.total_qty = self.total_qty.saturating_sub(qty);
    }
    
    /// Add to total quantity (e.g., when modifying order size up).
    #[inline(always)]
    pub fn add_qty(&mut self, qty: Quantity) {
        self.total_qty = self.total_qty.saturating_add(qty);
    }
    
    /// Reset the level to empty state.
    #[inline(always)]
    pub fn clear(&mut self) {
        self.order_count = 0;
        self.head = 0;
        self.tail = 0;
        self.total_qty = Quantity::ZERO;
        // Note: We don't clear the orders array for performance
    }
    
    /// Iterator over order handles (for debugging/testing).
    pub fn iter(&self) -> PriceLevelIter<'_> {
        PriceLevelIter {
            level: self,
            pos: 0,
        }
    }
}

impl Default for PriceLevel {
    fn default() -> Self {
        Self::new()
    }
}

/// Iterator over order handles in a price level.
pub struct PriceLevelIter<'a> {
    level: &'a PriceLevel,
    pos: usize,
}

impl<'a> Iterator for PriceLevelIter<'a> {
    type Item = OrderHandle;
    
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.level.order_count as usize {
            return None;
        }
        
        let idx = ((self.level.head as usize + self.pos) % MAX_ORDERS_PER_LEVEL) as usize;
        self.pos += 1;
        Some(self.level.orders[idx])
    }
    
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.level.order_count as usize - self.pos;
        (remaining, Some(remaining))
    }
}

impl<'a> ExactSizeIterator for PriceLevelIter<'a> {}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use alloc::vec;
    use super::*;
    
    #[test]
    fn test_level_push_pop() {
        let mut level = PriceLevel::new();
        assert!(level.is_empty());
        
        // Push some orders
        assert!(level.push_back(OrderHandle(1), Quantity(100)));
        assert!(level.push_back(OrderHandle(2), Quantity(200)));
        assert!(level.push_back(OrderHandle(3), Quantity(300)));
        
        assert_eq!(level.len(), 3);
        assert_eq!(level.total_qty.0, 600);
        
        // Pop in FIFO order
        assert_eq!(level.pop_front(), Some(OrderHandle(1)));
        assert_eq!(level.pop_front(), Some(OrderHandle(2)));
        assert_eq!(level.pop_front(), Some(OrderHandle(3)));
        assert_eq!(level.pop_front(), None);
        
        assert!(level.is_empty());
    }
    
    #[test]
    fn test_level_wrap_around() {
        let mut level = PriceLevel::new();
        
        // Fill half
        for i in 0..512 {
            assert!(level.push_back(OrderHandle(i), Quantity(1)));
        }
        
        // Pop half
        for i in 0..256 {
            assert_eq!(level.pop_front().map(|h| h.0), Some(i));
        }
        
        // Push more (should wrap around)
        for i in 512..768 {
            assert!(level.push_back(OrderHandle(i), Quantity(1)));
        }
        
        // Pop remaining
        for i in 256..768 {
            assert_eq!(level.pop_front().map(|h| h.0), Some(i));
        }
        
        assert!(level.is_empty());
    }
    
    #[test]
    fn test_level_front() {
        let mut level = PriceLevel::new();
        assert!(level.front().is_none());
        
        level.push_back(OrderHandle(42), Quantity(100));
        assert_eq!(level.front(), Some(OrderHandle(42)));
        
        // Front doesn't remove
        assert_eq!(level.front(), Some(OrderHandle(42)));
        assert_eq!(level.len(), 1);
    }
    
    #[test]
    fn test_level_iterator() {
        let mut level = PriceLevel::new();
        level.push_back(OrderHandle(1), Quantity(1));
        level.push_back(OrderHandle(2), Quantity(1));
        level.push_back(OrderHandle(3), Quantity(1));
        
        let handles: Vec<u32> = level.iter().map(|h| h.0).collect();
        assert_eq!(handles, vec![1, 2, 3]);
    }
}

