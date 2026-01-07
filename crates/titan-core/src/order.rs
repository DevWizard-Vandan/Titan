//! Order types and lifecycle management.
//!
//! The Order struct is exactly 64 bytes to fit in a single cache line.

use core::mem::size_of;
use crate::fixed::{Price, Quantity};

/// Side of the order book.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Side {
    /// Bid side (buyers).
    Buy = 0,
    /// Ask side (sellers).
    Sell = 1,
}

impl Side {
    /// Get the opposite side.
    #[inline(always)]
    pub const fn opposite(self) -> Self {
        match self {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        }
    }
    
    /// Check if this is the buy side.
    #[inline(always)]
    pub const fn is_buy(self) -> bool {
        matches!(self, Side::Buy)
    }
    
    /// Check if this is the sell side.
    #[inline(always)]
    pub const fn is_sell(self) -> bool {
        matches!(self, Side::Sell)
    }
}

/// Order type (Time-In-Force).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum OrderType {
    /// Good-Til-Cancelled: rests on book until filled or cancelled.
    Limit = 0,
    /// Immediate-Or-Cancel: fill what you can, cancel rest.
    IOC = 1,
    /// Fill-Or-Kill: fill entirely or reject entirely.
    FOK = 2,
    /// Post-Only: reject if would immediately match (maker-only).
    PostOnly = 3,
}

impl OrderType {
    /// Check if order should rest on book after partial fill.
    #[inline(always)]
    pub const fn should_rest(self) -> bool {
        matches!(self, OrderType::Limit | OrderType::PostOnly)
    }
}

/// Symbol identifier.
///
/// Pre-hashed at order entry. Maps "AAPL" → SymbolId(42) at startup.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[repr(transparent)]
pub struct SymbolId(pub u32);

impl SymbolId {
    /// Invalid/unset symbol.
    pub const INVALID: Self = Self(u32::MAX);
}

/// Unique order identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[repr(transparent)]
pub struct OrderId(pub u64);

impl OrderId {
    /// Invalid/unset order ID.
    pub const INVALID: Self = Self(0);
    
    /// Check if order ID is valid.
    #[inline(always)]
    pub const fn is_valid(self) -> bool {
        self.0 != 0
    }
}

/// The Order structure - EXACTLY 64 bytes (one cache line).
///
/// Layout is critical: frequently accessed fields first.
#[derive(Clone, Copy, Debug)]
#[repr(C, align(64))]
pub struct Order {
    // === HOT FIELDS (accessed during matching) === 32 bytes
    /// Order price.
    pub price: Price,               // 8 bytes
    /// Remaining quantity to fill.
    pub remaining_qty: Quantity,    // 8 bytes
    /// Unique order identifier.
    pub order_id: OrderId,          // 8 bytes
    /// Timestamp (RDTSC or monotonic nanos).
    pub timestamp: u64,             // 8 bytes
    
    // === WARM FIELDS (accessed occasionally) === 15 bytes
    /// Original quantity when order was placed.
    pub original_qty: Quantity,     // 8 bytes
    /// Symbol identifier.
    pub symbol: SymbolId,           // 4 bytes
    /// Order side (buy/sell).
    pub side: Side,                 // 1 byte
    /// Order type (limit, IOC, FOK, post-only).
    pub order_type: OrderType,      // 1 byte
    /// Bitflags for special handling.
    pub flags: u8,                  // 1 byte
    
    // === PADDING to 64 bytes ===
    _padding: [u8; 17],             // 17 bytes
}

// Compile-time assertion that Order is exactly 64 bytes.
const _: () = assert!(size_of::<Order>() == 64, "Order must be exactly 64 bytes");

impl Order {
    /// Create a new order.
    #[inline(always)]
    pub fn new(
        order_id: OrderId,
        symbol: SymbolId,
        side: Side,
        order_type: OrderType,
        price: Price,
        qty: Quantity,
        timestamp: u64,
    ) -> Self {
        Self {
            order_id,
            symbol,
            side,
            order_type,
            price,
            original_qty: qty,
            remaining_qty: qty,
            timestamp,
            flags: 0,
            _padding: [0; 17],
        }
    }
    
    /// Check if order is completely filled.
    #[inline(always)]
    pub const fn is_filled(&self) -> bool {
        self.remaining_qty.is_zero()
    }
    
    /// Fill the order by the given quantity.
    ///
    /// # Panics
    /// Debug-panics if qty > remaining_qty.
    #[inline(always)]
    pub fn fill(&mut self, qty: Quantity) {
        debug_assert!(qty.0 <= self.remaining_qty.0, "Fill quantity exceeds remaining");
        self.remaining_qty = self.remaining_qty.saturating_sub(qty);
    }
    
    /// Get filled quantity.
    #[inline(always)]
    pub const fn filled_qty(&self) -> Quantity {
        Quantity(self.original_qty.0 - self.remaining_qty.0)
    }
    
    /// Check if this is a buy order.
    #[inline(always)]
    pub const fn is_buy(&self) -> bool {
        self.side.is_buy()
    }
    
    /// Check if this is a sell order.
    #[inline(always)]
    pub const fn is_sell(&self) -> bool {
        self.side.is_sell()
    }
}

impl Default for Order {
    fn default() -> Self {
        Self {
            price: Price::ZERO,
            remaining_qty: Quantity::ZERO,
            order_id: OrderId::INVALID,
            timestamp: 0,
            original_qty: Quantity::ZERO,
            symbol: SymbolId::INVALID,
            side: Side::Buy,
            order_type: OrderType::Limit,
            flags: 0,
            _padding: [0; 17],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_order_size() {
        assert_eq!(size_of::<Order>(), 64);
    }
    
    #[test]
    fn test_order_fill() {
        let mut order = Order::new(
            OrderId(1),
            SymbolId(1),
            Side::Buy,
            OrderType::Limit,
            Price::from_ticks(100),
            Quantity(100),
            0,
        );
        
        assert!(!order.is_filled());
        order.fill(Quantity(50));
        assert_eq!(order.remaining_qty.0, 50);
        assert!(!order.is_filled());
        
        order.fill(Quantity(50));
        assert!(order.is_filled());
        assert_eq!(order.filled_qty().0, 100);
    }
    
    #[test]
    fn test_side_opposite() {
        assert_eq!(Side::Buy.opposite(), Side::Sell);
        assert_eq!(Side::Sell.opposite(), Side::Buy);
    }
}
