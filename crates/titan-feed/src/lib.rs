//! Market data feed publisher.
//!
//! Publishes trade executions and quote updates via UDP multicast.

pub mod publisher;

pub use publisher::Publisher;
