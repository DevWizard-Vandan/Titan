//! Object pool for zero-allocation order management.
//!
//! Pre-allocates all order slots at startup. Uses LIFO free list
//! for better cache locality on recently deallocated slots.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::mem::MaybeUninit;
use crate::order::Order;

/// Index into the order pool.
///
/// Uses u32 to save space (supports up to 4 billion orders).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct OrderHandle(pub u32);

impl OrderHandle {
    /// Invalid handle constant.
    pub const INVALID: Self = Self(u32::MAX);
    
    /// Check if handle is valid.
    #[inline(always)]
    pub const fn is_valid(self) -> bool {
        self.0 != u32::MAX
    }
    
    /// Get raw index.
    #[inline(always)]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl Default for OrderHandle {
    fn default() -> Self {
        Self::INVALID
    }
}

/// Pre-allocated pool of orders.
///
/// Capacity should be power of 2 for efficient operations.
pub struct OrderPool {
    /// Storage for orders.
    orders: Box<[MaybeUninit<Order>]>,
    /// LIFO free list for O(1) alloc/dealloc.
    free_list: Vec<u32>,
    /// Total capacity.
    capacity: u32,
    /// Number of active orders.
    active_count: u32,
}

impl OrderPool {
    /// Create a new pool with 2^order_bits capacity.
    ///
    /// # Example
    /// - `order_bits = 20` → 1,048,576 orders
    /// - `order_bits = 24` → 16,777,216 orders
    ///
    /// # Panics
    /// Panics if order_bits > 28 (256M orders max).
    pub fn new(order_bits: u32) -> Self {
        assert!(order_bits <= 28, "Pool too large (max 2^28)");
        let capacity = 1u32 << order_bits;
        
        // Allocate uninitialized storage
        let mut orders: Vec<MaybeUninit<Order>> = Vec::with_capacity(capacity as usize);
        // SAFETY: MaybeUninit doesn't require initialization
        unsafe { orders.set_len(capacity as usize); }
        
        // Pre-populate free list in reverse (LIFO gives better cache locality)
        let free_list: Vec<u32> = (0..capacity).rev().collect();
        
        Self {
            orders: orders.into_boxed_slice(),
            free_list,
            capacity,
            active_count: 0,
        }
    }
    
    /// Create a pool with specified capacity (must be power of 2).
    pub fn with_capacity(capacity: usize) -> Self {
        assert!(capacity.is_power_of_two(), "Capacity must be power of 2");
        assert!(capacity <= (1 << 28), "Capacity too large");
        
        let bits = capacity.trailing_zeros();
        Self::new(bits)
    }
    
    /// Allocate an order slot.
    ///
    /// Returns `None` if pool is exhausted.
    #[inline(always)]
    pub fn allocate(&mut self) -> Option<OrderHandle> {
        self.free_list.pop().map(|idx| {
            self.active_count += 1;
            OrderHandle(idx)
        })
    }
    
    /// Return an order slot to the pool.
    ///
    /// # Safety
    /// The handle must have been previously allocated and not yet deallocated.
    #[inline(always)]
    pub fn deallocate(&mut self, handle: OrderHandle) {
        debug_assert!(handle.0 < self.capacity, "Invalid handle");
        debug_assert!(self.active_count > 0, "Double deallocation");
        
        self.free_list.push(handle.0);
        self.active_count -= 1;
    }
    
    /// Get immutable reference to order.
    ///
    /// # Safety
    /// Handle must point to an initialized order.
    #[inline(always)]
    pub fn get(&self, handle: OrderHandle) -> &Order {
        debug_assert!(handle.0 < self.capacity, "Handle out of bounds");
        // SAFETY: Caller ensures handle points to initialized order
        unsafe { self.orders[handle.index()].assume_init_ref() }
    }
    
    /// Get mutable reference to order.
    ///
    /// # Safety
    /// Handle must point to an initialized order.
    #[inline(always)]
    pub fn get_mut(&mut self, handle: OrderHandle) -> &mut Order {
        debug_assert!(handle.0 < self.capacity, "Handle out of bounds");
        // SAFETY: Caller ensures handle points to initialized order
        unsafe { self.orders[handle.index()].assume_init_mut() }
    }
    
    /// Write a new order into the slot.
    #[inline(always)]
    pub fn insert(&mut self, handle: OrderHandle, order: Order) {
        debug_assert!(handle.0 < self.capacity, "Handle out of bounds");
        self.orders[handle.index()].write(order);
    }
    
    /// Allocate and insert an order in one operation.
    #[inline(always)]
    pub fn allocate_and_insert(&mut self, order: Order) -> Option<OrderHandle> {
        let handle = self.allocate()?;
        self.insert(handle, order);
        Some(handle)
    }
    
    /// Number of available slots.
    #[inline(always)]
    pub fn available(&self) -> usize {
        self.free_list.len()
    }
    
    /// Number of active orders.
    #[inline(always)]
    pub fn active(&self) -> usize {
        self.active_count as usize
    }
    
    /// Total capacity.
    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.capacity as usize
    }
    
    /// Check if pool is exhausted.
    #[inline(always)]
    pub fn is_full(&self) -> bool {
        self.free_list.is_empty()
    }
    
    /// Check if pool is empty.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.active_count == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order::{OrderId, SymbolId, Side, OrderType};
    use crate::fixed::{Price, Quantity};
    
    #[test]
    fn test_pool_allocate_deallocate() {
        let mut pool = OrderPool::new(4); // 16 slots
        assert_eq!(pool.capacity(), 16);
        assert_eq!(pool.available(), 16);
        
        let h1 = pool.allocate().unwrap();
        assert_eq!(pool.available(), 15);
        assert_eq!(pool.active(), 1);
        
        let h2 = pool.allocate().unwrap();
        assert_eq!(pool.available(), 14);
        assert_eq!(pool.active(), 2);
        
        pool.deallocate(h1);
        assert_eq!(pool.available(), 15);
        assert_eq!(pool.active(), 1);
        
        // LIFO: next alloc should return h1's slot
        let h3 = pool.allocate().unwrap();
        assert_eq!(h3.0, h1.0);
    }
    
    #[test]
    fn test_pool_insert_get() {
        let mut pool = OrderPool::new(4);
        let handle = pool.allocate().unwrap();
        
        let order = Order::new(
            OrderId(42),
            SymbolId(1),
            Side::Buy,
            OrderType::Limit,
            Price::from_ticks(100),
            Quantity(1000),
            12345,
        );
        
        pool.insert(handle, order);
        
        let retrieved = pool.get(handle);
        assert_eq!(retrieved.order_id.0, 42);
        assert_eq!(retrieved.remaining_qty.0, 1000);
    }
    
    #[test]
    fn test_pool_exhaustion() {
        let mut pool = OrderPool::new(2); // 4 slots
        
        let _h1 = pool.allocate().unwrap();
        let _h2 = pool.allocate().unwrap();
        let _h3 = pool.allocate().unwrap();
        let _h4 = pool.allocate().unwrap();
        
        assert!(pool.is_full());
        assert!(pool.allocate().is_none());
    }
}
