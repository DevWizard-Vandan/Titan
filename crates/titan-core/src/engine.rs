//! Matching engine core.
//!
//! This is THE hot path. Every nanosecond matters here.
//! The matching algorithm implements price-time priority.

use arrayvec::ArrayVec;
use crate::fixed::{Price, Quantity};
use crate::order::{Order, OrderId, Side, OrderType, SymbolId};
use crate::pool::{OrderPool, OrderHandle};
use crate::book::OrderBook;

/// Maximum fills per order (limits stack usage).
pub const MAX_FILLS_PER_ORDER: usize = 64;

/// Execution report for a single fill.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Fill {
    /// Maker order ID.
    pub maker_order_id: OrderId,
    /// Taker order ID.
    pub taker_order_id: OrderId,
    /// Execution price.
    pub price: Price,
    /// Execution quantity.
    pub quantity: Quantity,
    /// Maker side.
    pub maker_side: Side,
    /// Symbol.
    pub symbol: SymbolId,
    /// Timestamp.
    pub timestamp: u64,
}

/// Result of order submission.
#[derive(Debug)]
pub enum OrderResult {
    /// Order fully filled.
    Filled {
        fills: ArrayVec<Fill, MAX_FILLS_PER_ORDER>,
    },
    /// Order partially filled, rest resting on book.
    PartialFill {
        fills: ArrayVec<Fill, MAX_FILLS_PER_ORDER>,
        resting_qty: Quantity,
        handle: OrderHandle,
    },
    /// Order resting on book (no matches).
    Resting {
        handle: OrderHandle,
    },
    /// Order rejected.
    Rejected {
        reason: RejectReason,
    },
    /// Order cancelled (IOC with no fill, FOK with partial available).
    Cancelled {
        filled_qty: Quantity,
        fills: ArrayVec<Fill, MAX_FILLS_PER_ORDER>,
    },
}

/// Rejection reasons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectReason {
    /// Price is invalid (out of range).
    InvalidPrice,
    /// Quantity is zero or invalid.
    InvalidQuantity,
    /// Order pool exhausted.
    PoolExhausted,
    /// Price level full.
    BookFull,
    /// Post-only order would immediately match.
    PostOnlyWouldMatch,
    /// Symbol not found.
    SymbolNotFound,
    /// FOK order cannot be fully filled.
    InsufficientLiquidity,
}

/// The matching engine.
///
/// Combines an OrderBook with an OrderPool for complete order lifecycle.
pub struct MatchingEngine {
    /// The order book.
    pub book: OrderBook,
    /// The order pool.
    pub pool: OrderPool,
    /// Symbol for this engine.
    pub symbol: SymbolId,
}

impl MatchingEngine {
    /// Create a new matching engine.
    ///
    /// `pool_bits`: log2 of pool capacity (e.g., 20 = 1M orders)
    /// `base_price`: minimum price for book indexing
    pub fn new(symbol: SymbolId, pool_bits: u32, base_price: Price) -> Self {
        Self {
            book: OrderBook::new(base_price),
            pool: OrderPool::with_capacity(1 << pool_bits),
            symbol,
        }
    }
    
    /// Submit an order to the matching engine.
    ///
    /// This is THE hot path - every nanosecond matters.
    #[inline]
    pub fn submit_order(&mut self, mut order: Order, timestamp: u64) -> OrderResult {
        // === VALIDATION (minimal, fast-fail) ===
        if order.remaining_qty.is_zero() {
            return OrderResult::Rejected { reason: RejectReason::InvalidQuantity };
        }
        
        if order.price.is_zero() && order.order_type != OrderType::IOC {
            return OrderResult::Rejected { reason: RejectReason::InvalidPrice };
        }
        
        // Assign timestamp
        order.timestamp = timestamp;
        
        // === POST-ONLY CHECK ===
        if order.order_type == OrderType::PostOnly {
            let opposite_side = self.book.opposite_side_mut(order.side);
            if opposite_side.would_match(order.price, order.side) {
                return OrderResult::Rejected { reason: RejectReason::PostOnlyWouldMatch };
            }
        }
        
        // === FOK PRE-CHECK ===
        if order.order_type == OrderType::FOK {
            if !self.can_fill_completely(&order) {
                return OrderResult::Rejected { reason: RejectReason::InsufficientLiquidity };
            }
        }
        
        // === MATCHING ===
        let mut fills = ArrayVec::new();
        self.match_order(&mut order, &mut fills);
        
        // === POST-MATCH HANDLING ===
        if order.remaining_qty.is_zero() {
            // Fully filled
            return OrderResult::Filled { fills };
        }
        
        match order.order_type {
            OrderType::IOC => {
                // Cancel remaining
                OrderResult::Cancelled {
                    filled_qty: order.filled_qty(),
                    fills,
                }
            }
            OrderType::FOK => {
                // Should have been caught by pre-check, but handle anyway
                OrderResult::Cancelled {
                    filled_qty: order.filled_qty(),
                    fills,
                }
            }
            OrderType::Limit | OrderType::PostOnly => {
                // Add remaining to book
                match self.add_to_book(order) {
                    Some(handle) => {
                        if fills.is_empty() {
                            OrderResult::Resting { handle }
                        } else {
                            OrderResult::PartialFill {
                                fills,
                                resting_qty: order.remaining_qty,
                                handle,
                            }
                        }
                    }
                    None => OrderResult::Rejected { reason: RejectReason::PoolExhausted },
                }
            }
        }
    }
    
    /// Check if order can be completely filled (for FOK).
    #[inline]
    fn can_fill_completely(&self, order: &Order) -> bool {
        let opposite_side = match order.side {
            Side::Buy => &self.book.asks,
            Side::Sell => &self.book.bids,
        };
        
        // Simple check: just verify there's enough total quantity at crossing prices
        if let Some(best_price) = opposite_side.best_price() {
            let crosses = match order.side {
                Side::Buy => order.price.0 >= best_price.0,
                Side::Sell => order.price.0 <= best_price.0,
            };
            
            if crosses {
                // For simplicity, just check if best level has enough
                // In production, would walk the book
                if let Some(level) = opposite_side.best_level() {
                    return level.total_qty.0 >= order.remaining_qty.0;
                }
            }
        }
        
        false
    }
    
    /// Core matching loop.
    /// Refactored to avoid borrow checker issues by not holding mutable reference across operations.
    #[inline(always)]
    fn match_order(&mut self, order: &mut Order, fills: &mut ArrayVec<Fill, MAX_FILLS_PER_ORDER>) {
        loop {
            if order.remaining_qty.is_zero() {
                break;
            }
            
            // Get best price for comparison (immutable borrow, released immediately)
            let (best_price, crosses) = {
                let opposite_side = match order.side {
                    Side::Buy => &self.book.asks,
                    Side::Sell => &self.book.bids,
                };
                
                match opposite_side.best_price() {
                    Some(bp) => {
                        let c = match order.side {
                            Side::Buy => order.price.0 >= bp.0,
                            Side::Sell => order.price.0 <= bp.0,
                        };
                        (bp, c)
                    }
                    None => break, // No liquidity
                }
            };
            
            if !crosses {
                break;
            }
            
            // Match one order at a time at the best level
            let fill_result = self.match_one_at_best(order.side.opposite(), order, best_price);
            
            match fill_result {
                Some(fill) => {
                    if !fills.is_full() {
                        fills.push(fill);
                    }
                }
                None => {
                    // No more orders at this level, find next best
                    let opposite_side = match order.side {
                        Side::Buy => &mut self.book.asks,
                        Side::Sell => &mut self.book.bids,
                    };
                    opposite_side.find_next_best();
                }
            }
        }
    }
    
    /// Match against one maker order at the best level.
    /// Returns Some(Fill) if matched, None if level is exhausted.
    #[inline]
    fn match_one_at_best(&mut self, maker_side: Side, taker: &mut Order, exec_price: Price) -> Option<Fill> {
        let opposite_book = match maker_side {
            Side::Buy => &mut self.book.bids,
            Side::Sell => &mut self.book.asks,
        };
        
        let best_level = opposite_book.best_level_mut()?;
        
        if best_level.is_empty() {
            return None;
        }
        
        let maker_handle = best_level.front()?;
        let maker = self.pool.get_mut(maker_handle);
        
        // Calculate fill quantity
        let fill_qty = taker.remaining_qty.min(maker.remaining_qty);
        
        // Create fill record
        let fill = Fill {
            maker_order_id: maker.order_id,
            taker_order_id: taker.order_id,
            price: exec_price,
            quantity: fill_qty,
            maker_side: maker.side,
            symbol: taker.symbol,
            timestamp: taker.timestamp,
        };
        
        // Execute fill
        taker.fill(fill_qty);
        maker.fill(fill_qty);
        
        // Update level
        let opposite_book = match maker_side {
            Side::Buy => &mut self.book.bids,
            Side::Sell => &mut self.book.asks,
        };
        
        if let Some(level) = opposite_book.best_level_mut() {
            level.reduce_qty(fill_qty);
            
            // Remove maker if fully filled
            if self.pool.get(maker_handle).is_filled() {
                level.pop_front();
                self.pool.deallocate(maker_handle);
                opposite_book.decrement_order_count();
            }
        }
        
        opposite_book.reduce_qty(fill_qty);
        
        Some(fill)
    }
    
    /// Add order to the book.
    #[inline]
    fn add_to_book(&mut self, order: Order) -> Option<OrderHandle> {
        let handle = self.pool.allocate()?;
        self.pool.insert(handle, order);
        
        let book_side = self.book.side_mut(order.side);
        let order_ref = self.pool.get(handle);
        
        if book_side.add_order(handle, order_ref) {
            Some(handle)
        } else {
            self.pool.deallocate(handle);
            None
        }
    }
    
    /// Cancel an order by handle.
    #[inline]
    pub fn cancel_order(&mut self, handle: OrderHandle) -> Option<Order> {
        if !handle.is_valid() {
            return None;
        }
        
        let order = *self.pool.get(handle);
        
        // Remove from book
        let book_side = self.book.side_mut(order.side);
        if let Some(level) = book_side.level_at_price_mut(order.price) {
            level.reduce_qty(order.remaining_qty);
        }
        
        book_side.reduce_qty(order.remaining_qty);
        book_side.decrement_order_count();
        
        self.pool.deallocate(handle);
        
        Some(order)
    }
    
    /// Get order by handle.
    #[inline(always)]
    pub fn get_order(&self, handle: OrderHandle) -> Option<&Order> {
        if handle.is_valid() {
            Some(self.pool.get(handle))
        } else {
            None
        }
    }
    
    /// Get pool statistics.
    pub fn pool_stats(&self) -> (usize, usize) {
        (self.pool.active(), self.pool.capacity())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    fn create_engine() -> MatchingEngine {
        MatchingEngine::new(SymbolId(1), 10, Price::ZERO) // 1024 orders
    }
    
    #[test]
    fn test_simple_match() {
        let mut engine = create_engine();
        
        // Place sell order
        let sell = Order::new(
            OrderId(1), SymbolId(1), Side::Sell, OrderType::Limit,
            Price::from_ticks(100), Quantity(100), 0,
        );
        let result = engine.submit_order(sell, 1);
        assert!(matches!(result, OrderResult::Resting { .. }));
        
        // Place matching buy order
        let buy = Order::new(
            OrderId(2), SymbolId(1), Side::Buy, OrderType::Limit,
            Price::from_ticks(100), Quantity(100), 2,
        );
        let result = engine.submit_order(buy, 2);
        
        match result {
            OrderResult::Filled { fills } => {
                assert_eq!(fills.len(), 1);
                assert_eq!(fills[0].quantity.0, 100);
                assert_eq!(fills[0].price, Price::from_ticks(100));
                assert_eq!(fills[0].maker_order_id.0, 1);
                assert_eq!(fills[0].taker_order_id.0, 2);
            }
            _ => panic!("Expected Filled, got {:?}", result),
        }
    }
    
    #[test]
    fn test_partial_fill() {
        let mut engine = create_engine();
        
        // Place sell order for 50
        let sell = Order::new(
            OrderId(1), SymbolId(1), Side::Sell, OrderType::Limit,
            Price::from_ticks(100), Quantity(50), 0,
        );
        engine.submit_order(sell, 1);
        
        // Place buy order for 100
        let buy = Order::new(
            OrderId(2), SymbolId(1), Side::Buy, OrderType::Limit,
            Price::from_ticks(100), Quantity(100), 2,
        );
        let result = engine.submit_order(buy, 2);
        
        match result {
            OrderResult::PartialFill { fills, resting_qty, .. } => {
                assert_eq!(fills.len(), 1);
                assert_eq!(fills[0].quantity.0, 50);
                assert_eq!(resting_qty.0, 50);
            }
            _ => panic!("Expected PartialFill, got {:?}", result),
        }
    }
    
    #[test]
    fn test_price_time_priority() {
        let mut engine = create_engine();
        
        // Place two sell orders at same price
        let sell1 = Order::new(
            OrderId(1), SymbolId(1), Side::Sell, OrderType::Limit,
            Price::from_ticks(100), Quantity(50), 0,
        );
        engine.submit_order(sell1, 1);
        
        let sell2 = Order::new(
            OrderId(2), SymbolId(1), Side::Sell, OrderType::Limit,
            Price::from_ticks(100), Quantity(50), 0,
        );
        engine.submit_order(sell2, 2);
        
        // Buy should match with first sell (time priority)
        let buy = Order::new(
            OrderId(3), SymbolId(1), Side::Buy, OrderType::Limit,
            Price::from_ticks(100), Quantity(50), 3,
        );
        let result = engine.submit_order(buy, 3);
        
        match result {
            OrderResult::Filled { fills } => {
                assert_eq!(fills[0].maker_order_id.0, 1); // First order matched
            }
            _ => panic!("Expected Filled"),
        }
    }
    
    #[test]
    fn test_ioc_no_match() {
        let mut engine = create_engine();
        
        // IOC order with no matching liquidity
        let order = Order::new(
            OrderId(1), SymbolId(1), Side::Buy, OrderType::IOC,
            Price::from_ticks(100), Quantity(100), 0,
        );
        let result = engine.submit_order(order, 1);
        
        match result {
            OrderResult::Cancelled { filled_qty, .. } => {
                assert_eq!(filled_qty.0, 0);
            }
            _ => panic!("Expected Cancelled"),
        }
    }
    
    #[test]
    fn test_post_only_reject() {
        let mut engine = create_engine();
        
        // Place sell at 100
        let sell = Order::new(
            OrderId(1), SymbolId(1), Side::Sell, OrderType::Limit,
            Price::from_ticks(100), Quantity(100), 0,
        );
        engine.submit_order(sell, 1);
        
        // Post-only buy at 100 should be rejected (would match)
        let buy = Order::new(
            OrderId(2), SymbolId(1), Side::Buy, OrderType::PostOnly,
            Price::from_ticks(100), Quantity(100), 2,
        );
        let result = engine.submit_order(buy, 2);
        
        assert!(matches!(result, OrderResult::Rejected { reason: RejectReason::PostOnlyWouldMatch }));
    }
}
