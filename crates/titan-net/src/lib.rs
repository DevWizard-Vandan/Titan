//! Network gateway for low-latency TCP/UDP I/O.
//!
//! Uses mio for non-blocking event-driven networking.

pub mod gateway;

pub use gateway::Gateway;
