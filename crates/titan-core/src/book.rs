//! Order book data structures.
//!
//! The order book maintains two sides (bids and asks) with price levels
//! indexed by price for O(1) access.

use alloc::boxed::Box;
use crate::fixed::{Price, Quantity};
use crate::order::{Order, Side};
use crate::pool::OrderHandle;
use crate::level::PriceLevel;

/// Maximum number of price levels per side.
/// For a stock with $0.01 ticks and $1000 range: 100,000 levels.
/// Using 65536 (2^16) for efficient indexing.
pub const MAX_LEVELS: usize = 65536;

/// One side of the order book (Bids or Asks).
pub struct BookSide {
    /// Price levels indexed by tick offset from base price.
    /// Index = (price - base_price) / tick_size
    levels: Box<[Option<PriceLevel>]>,
    
    /// Best price level index (None if side is empty).
    best_idx: Option<u32>,
    
    /// Side indicator for price comparison.
    side: Side,
    
    /// Base price for indexing (lowest price in range).
    base_price: Price,
    
    /// Total order count on this side.
    order_count: u64,
    
    /// Total quantity on this side.
    total_qty: Quantity,
}

impl BookSide {
    /// Create a new book side.
    ///
    /// `base_price` is the minimum price that can be represented.
    /// Prices below this cannot be used.
    pub fn new(side: Side, base_price: Price) -> Self {
        // Allocate with all None (no levels initially)
        let mut levels_vec = alloc::vec::Vec::with_capacity(MAX_LEVELS);
        levels_vec.resize_with(MAX_LEVELS, || None);
        
        Self {
            levels: levels_vec.into_boxed_slice(),
            best_idx: None,
            side,
            base_price,
            order_count: 0,
            total_qty: Quantity::ZERO,
        }
    }
    
    /// Convert price to level index.
    #[inline(always)]
    fn price_to_idx(&self, price: Price) -> Option<usize> {
        if price.0 < self.base_price.0 {
            return None;
        }
        let offset = price.0 - self.base_price.0;
        let idx = (offset / Price::TICK_SIZE) as usize;
        if idx < MAX_LEVELS { Some(idx) } else { None }
    }
    
    /// Convert level index back to price.
    #[inline(always)]
    fn idx_to_price(&self, idx: usize) -> Price {
        Price(self.base_price.0 + (idx as u64 * Price::TICK_SIZE))
    }
    
    /// Add order to appropriate price level.
    #[inline]
    pub fn add_order(&mut self, handle: OrderHandle, order: &Order) -> bool {
        let idx = match self.price_to_idx(order.price) {
            Some(i) => i,
            None => return false,
        };
        
        // Get or create level
        let level = self.levels[idx].get_or_insert_with(PriceLevel::new);
        
        if !level.push_back(handle, order.remaining_qty) {
            return false;
        }
        
        self.order_count += 1;
        self.total_qty = self.total_qty.saturating_add(order.remaining_qty);
        
        // Update best price
        self.update_best_after_add(idx);
        
        true
    }
    
    /// Update best price after adding at index.
    #[inline]
    fn update_best_after_add(&mut self, new_idx: usize) {
        match self.best_idx {
            None => self.best_idx = Some(new_idx as u32),
            Some(current) => {
                let is_better = match self.side {
                    // For bids: higher price is better
                    Side::Buy => new_idx > current as usize,
                    // For asks: lower price is better
                    Side::Sell => new_idx < current as usize,
                };
                if is_better {
                    self.best_idx = Some(new_idx as u32);
                }
            }
        }
    }
    
    /// Get the best price level for matching (immutable).
    #[inline(always)]
    pub fn best_level(&self) -> Option<&PriceLevel> {
        self.best_idx
            .and_then(|idx| self.levels[idx as usize].as_ref())
    }
    
    /// Get the best price level for matching (mutable).
    #[inline(always)]
    pub fn best_level_mut(&mut self) -> Option<&mut PriceLevel> {
        self.best_idx
            .and_then(|idx| self.levels[idx as usize].as_mut())
    }
    
    /// Get the best price.
    #[inline(always)]
    pub fn best_price(&self) -> Option<Price> {
        self.best_idx.map(|idx| self.idx_to_price(idx as usize))
    }
    
    /// Check if incoming order price would cross the best resting price.
    #[inline(always)]
    pub fn would_match(&self, price: Price, incoming_side: Side) -> bool {
        if let Some(best_idx) = self.best_idx {
            let best_price = self.idx_to_price(best_idx as usize);
            match incoming_side {
                // Buy crosses if >= best ask
                Side::Buy => price.0 >= best_price.0,
                // Sell crosses if <= best bid
                Side::Sell => price.0 <= best_price.0,
            }
        } else {
            false
        }
    }
    
    /// Find next best price after current is exhausted.
    pub fn find_next_best(&mut self) {
        let current = match self.best_idx {
            Some(idx) => idx as usize,
            None => return,
        };
        
        // Check if current level is exhausted
        if self.levels[current]
            .as_ref()
            .map_or(true, |l| l.is_empty())
        {
            // Clear the empty level
            self.levels[current] = None;
        } else {
            // Level still has orders, keep it as best
            return;
        }
        
        // Search for next best
        self.best_idx = None;
        
        match self.side {
            // Bids: search downward (lower indices = lower prices)
            Side::Buy => {
                for idx in (0..current).rev() {
                    if self.levels[idx].as_ref().map_or(false, |l| !l.is_empty()) {
                        self.best_idx = Some(idx as u32);
                        break;
                    }
                }
            }
            // Asks: search upward (higher indices = higher prices)
            Side::Sell => {
                for idx in (current + 1)..MAX_LEVELS {
                    if self.levels[idx].as_ref().map_or(false, |l| !l.is_empty()) {
                        self.best_idx = Some(idx as u32);
                        break;
                    }
                }
            }
        }
    }
    
    /// Get level at specific price (mutable).
    #[inline]
    pub fn level_at_price_mut(&mut self, price: Price) -> Option<&mut PriceLevel> {
        let idx = self.price_to_idx(price)?;
        self.levels[idx].as_mut()
    }
    
    /// Check if side is empty.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.best_idx.is_none()
    }
    
    /// Get order count.
    #[inline(always)]
    pub fn order_count(&self) -> u64 {
        self.order_count
    }
    
    /// Get total quantity.
    #[inline(always)]
    pub fn total_qty(&self) -> Quantity {
        self.total_qty
    }
    
    /// Reduce total quantity (after fill).
    #[inline(always)]
    pub fn reduce_qty(&mut self, qty: Quantity) {
        self.total_qty = self.total_qty.saturating_sub(qty);
    }
    
    /// Decrement order count.
    #[inline(always)]
    pub fn decrement_order_count(&mut self) {
        self.order_count = self.order_count.saturating_sub(1);
    }
}

/// The complete order book for a single symbol.
pub struct OrderBook {
    /// Bid side (buyers).
    pub bids: BookSide,
    /// Ask side (sellers).
    pub asks: BookSide,
    /// Sequence number for determinism.
    sequence: u64,
}

impl OrderBook {
    /// Create a new order book.
    ///
    /// `base_price` is the minimum price for indexing.
    /// Typically set to 0 or a reasonable floor price.
    pub fn new(base_price: Price) -> Self {
        Self {
            bids: BookSide::new(Side::Buy, base_price),
            asks: BookSide::new(Side::Sell, base_price),
            sequence: 0,
        }
    }
    
    /// Get the current sequence number.
    #[inline(always)]
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    
    /// Increment and return sequence number.
    #[inline(always)]
    pub fn next_sequence(&mut self) -> u64 {
        self.sequence += 1;
        self.sequence
    }
    
    /// Get best bid price.
    #[inline(always)]
    pub fn best_bid(&self) -> Option<Price> {
        self.bids.best_price()
    }
    
    /// Get best ask price.
    #[inline(always)]
    pub fn best_ask(&self) -> Option<Price> {
        self.asks.best_price()
    }
    
    /// Get the spread (best ask - best bid).
    pub fn spread(&self) -> Option<Price> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) if ask.0 > bid.0 => Some(Price(ask.0 - bid.0)),
            _ => None,
        }
    }
    
    /// Get midpoint price.
    pub fn midpoint(&self) -> Option<Price> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => Some(Price((bid.0 + ask.0) / 2)),
            (Some(bid), None) => Some(bid),
            (None, Some(ask)) => Some(ask),
            (None, None) => None,
        }
    }
    
    /// Check if book is empty.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.bids.is_empty() && self.asks.is_empty()
    }
    
    /// Get mutable reference to appropriate side.
    #[inline(always)]
    pub fn side_mut(&mut self, side: Side) -> &mut BookSide {
        match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        }
    }
    
    /// Get immutable reference to appropriate side.
    #[inline(always)]
    pub fn side(&self, side: Side) -> &BookSide {
        match side {
            Side::Buy => &self.bids,
            Side::Sell => &self.asks,
        }
    }
    
    /// Get the opposite side for matching.
    #[inline(always)]
    pub fn opposite_side_mut(&mut self, side: Side) -> &mut BookSide {
        match side {
            Side::Buy => &mut self.asks,
            Side::Sell => &mut self.bids,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order::{OrderId, SymbolId, OrderType};
    
    #[test]
    fn test_book_side_add_order() {
        let mut side = BookSide::new(Side::Buy, Price::ZERO);
        
        let order = Order::new(
            OrderId(1),
            SymbolId(1),
            Side::Buy,
            OrderType::Limit,
            Price::from_ticks(100),
            Quantity(1000),
            0,
        );
        
        let handle = OrderHandle(0);
        assert!(side.add_order(handle, &order));
        
        assert_eq!(side.order_count(), 1);
        assert_eq!(side.best_price(), Some(Price::from_ticks(100)));
    }
    
    #[test]
    fn test_book_side_best_update() {
        let mut side = BookSide::new(Side::Buy, Price::ZERO);
        
        // Add order at price 100
        let order1 = Order::new(
            OrderId(1), SymbolId(1), Side::Buy, OrderType::Limit,
            Price::from_ticks(100), Quantity(100), 0,
        );
        side.add_order(OrderHandle(0), &order1);
        assert_eq!(side.best_price(), Some(Price::from_ticks(100)));
        
        // Add better order at price 110 (higher is better for bids)
        let order2 = Order::new(
            OrderId(2), SymbolId(1), Side::Buy, OrderType::Limit,
            Price::from_ticks(110), Quantity(100), 0,
        );
        side.add_order(OrderHandle(1), &order2);
        assert_eq!(side.best_price(), Some(Price::from_ticks(110)));
        
        // Add worse order at price 90
        let order3 = Order::new(
            OrderId(3), SymbolId(1), Side::Buy, OrderType::Limit,
            Price::from_ticks(90), Quantity(100), 0,
        );
        side.add_order(OrderHandle(2), &order3);
        // Best should still be 110
        assert_eq!(side.best_price(), Some(Price::from_ticks(110)));
    }
    
    #[test]
    fn test_book_spread() {
        let mut book = OrderBook::new(Price::ZERO);
        
        // Add bid at 100
        let bid = Order::new(
            OrderId(1), SymbolId(1), Side::Buy, OrderType::Limit,
            Price::from_ticks(100), Quantity(100), 0,
        );
        book.bids.add_order(OrderHandle(0), &bid);
        
        // Add ask at 101
        let ask = Order::new(
            OrderId(2), SymbolId(1), Side::Sell, OrderType::Limit,
            Price::from_ticks(101), Quantity(100), 0,
        );
        book.asks.add_order(OrderHandle(1), &ask);
        
        assert_eq!(book.best_bid(), Some(Price::from_ticks(100)));
        assert_eq!(book.best_ask(), Some(Price::from_ticks(101)));
        assert_eq!(book.spread(), Some(Price::from_ticks(1)));
    }
}
