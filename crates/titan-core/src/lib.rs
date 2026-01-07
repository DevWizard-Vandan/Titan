//! # Titan Core
//!
//! Low-latency order book and matching engine.
//!
//! ## Design Principles
//! - Zero allocations in hot path
//! - Cache-line aligned data structures
//! - Fixed-point arithmetic (no floats)
//! - Single-threaded, lock-free design

#![no_std]
#![allow(dead_code)]

extern crate alloc;

pub mod fixed;
pub mod order;
pub mod pool;
pub mod level;
pub mod book;
pub mod engine;

pub use fixed::{Price, Quantity};
pub use order::{Order, OrderId, SymbolId, Side, OrderType};
pub use pool::{OrderPool, OrderHandle};
pub use level::PriceLevel;
pub use book::{OrderBook, BookSide};
pub use engine::{Fill, OrderResult, RejectReason, MatchingEngine};
