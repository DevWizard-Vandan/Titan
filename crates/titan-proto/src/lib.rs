//! Zero-copy binary protocol definitions.
//!
//! All messages are fixed-size, cache-line aligned, and can be
//! directly transmuted from wire bytes without parsing.

#![no_std]

pub mod messages;
pub mod parser;

pub use messages::*;
pub use parser::*;
