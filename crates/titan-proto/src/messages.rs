//! Binary message definitions.
//!
//! All messages use fixed-size layouts for zero-copy parsing.
//! Little-endian byte order is used throughout.

use bytemuck::{Pod, Zeroable};
use core::mem::size_of;

/// Message type discriminator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageType {
    // Inbound (client → engine)
    NewOrder = 0x01,
    CancelOrder = 0x02,
    ModifyOrder = 0x03,
    
    // Outbound (engine → client)
    ExecutionReport = 0x10,
    OrderAck = 0x11,
    OrderReject = 0x12,
    CancelAck = 0x13,
    
    // Market Data
    Trade = 0x20,
    Quote = 0x21,
    BookUpdate = 0x22,
    
    // System
    Heartbeat = 0xFE,
    SystemError = 0xFF,
}

impl TryFrom<u8> for MessageType {
    type Error = ();
    
    fn try_from(value: u8) -> Result<Self, ()> {
        match value {
            0x01 => Ok(MessageType::NewOrder),
            0x02 => Ok(MessageType::CancelOrder),
            0x03 => Ok(MessageType::ModifyOrder),
            0x10 => Ok(MessageType::ExecutionReport),
            0x11 => Ok(MessageType::OrderAck),
            0x12 => Ok(MessageType::OrderReject),
            0x13 => Ok(MessageType::CancelAck),
            0x20 => Ok(MessageType::Trade),
            0x21 => Ok(MessageType::Quote),
            0x22 => Ok(MessageType::BookUpdate),
            0xFE => Ok(MessageType::Heartbeat),
            0xFF => Ok(MessageType::SystemError),
            _ => Err(()),
        }
    }
}

/// Fixed-size message header (8 bytes).
#[derive(Clone, Copy, Debug, Default)]
#[repr(C, packed)]
pub struct MessageHeader {
    /// Message type.
    pub msg_type: u8,
    /// Message flags (reserved).
    pub flags: u8,
    /// Payload length (excluding header).
    pub length: u16,
    /// Sequence number.
    pub sequence: u32,
}

const _: () = assert!(size_of::<MessageHeader>() == 8);

// SAFETY: MessageHeader is plain-old-data with no padding issues
unsafe impl Pod for MessageHeader {}
unsafe impl Zeroable for MessageHeader {}

impl MessageHeader {
    /// Create a new header.
    pub const fn new(msg_type: u8, length: u16, sequence: u32) -> Self {
        Self {
            msg_type,
            flags: 0,
            length,
            sequence,
        }
    }
    
    /// Get total message size (header + payload).
    pub const fn total_size(&self) -> usize {
        size_of::<Self>() + self.length as usize
    }
}

/// New Order message (64 bytes total).
#[derive(Clone, Copy, Debug, Default)]
#[repr(C, packed)]
pub struct NewOrderMessage {
    pub header: MessageHeader,      // 8 bytes
    pub order_id: u64,              // 8 bytes
    pub symbol_id: u32,             // 4 bytes
    pub side: u8,                   // 1 byte (0=Buy, 1=Sell)
    pub order_type: u8,             // 1 byte (0=Limit, 1=IOC, 2=FOK, 3=PostOnly)
    pub _padding1: u16,             // 2 bytes (alignment)
    pub price: u64,                 // 8 bytes (fixed-point)
    pub quantity: u64,              // 8 bytes
    pub client_order_id: [u8; 20],  // 20 bytes (client reference)
    pub _reserved: [u8; 4],         // 4 bytes
}

const _: () = assert!(size_of::<NewOrderMessage>() == 64);

unsafe impl Pod for NewOrderMessage {}
unsafe impl Zeroable for NewOrderMessage {}

impl NewOrderMessage {
    /// Create a new order message.
    pub fn new(
        sequence: u32,
        order_id: u64,
        symbol_id: u32,
        side: u8,
        order_type: u8,
        price: u64,
        quantity: u64,
    ) -> Self {
        Self {
            header: MessageHeader::new(
                MessageType::NewOrder as u8,
                (size_of::<Self>() - size_of::<MessageHeader>()) as u16,
                sequence,
            ),
            order_id,
            symbol_id,
            side,
            order_type,
            _padding1: 0,
            price,
            quantity,
            client_order_id: [0; 20],
            _reserved: [0; 4],
        }
    }
}

/// Cancel Order message (32 bytes).
#[derive(Clone, Copy, Debug, Default)]
#[repr(C, packed)]
pub struct CancelOrderMessage {
    pub header: MessageHeader,      // 8 bytes
    pub order_id: u64,              // 8 bytes
    pub symbol_id: u32,             // 4 bytes
    pub _reserved: [u8; 12],        // 12 bytes
}

const _: () = assert!(size_of::<CancelOrderMessage>() == 32);

unsafe impl Pod for CancelOrderMessage {}
unsafe impl Zeroable for CancelOrderMessage {}

impl CancelOrderMessage {
    pub fn new(sequence: u32, order_id: u64, symbol_id: u32) -> Self {
        Self {
            header: MessageHeader::new(
                MessageType::CancelOrder as u8,
                (size_of::<Self>() - size_of::<MessageHeader>()) as u16,
                sequence,
            ),
            order_id,
            symbol_id,
            _reserved: [0; 12],
        }
    }
}

/// Execution type for reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ExecType {
    New = 0,
    Fill = 1,
    PartialFill = 2,
    Canceled = 3,
    Rejected = 4,
}

/// Execution Report (outbound, 64 bytes).
#[derive(Clone, Copy, Debug, Default)]
#[repr(C, packed)]
pub struct ExecutionReport {
    pub header: MessageHeader,      // 8 bytes
    pub order_id: u64,              // 8 bytes
    pub exec_id: u64,               // 8 bytes
    pub symbol_id: u32,             // 4 bytes
    pub side: u8,                   // 1 byte
    pub exec_type: u8,              // 1 byte
    pub _padding1: u16,             // 2 bytes
    pub exec_price: u64,            // 8 bytes
    pub exec_qty: u64,              // 8 bytes
    pub leaves_qty: u64,            // 8 bytes (remaining qty)
    pub timestamp: u64,             // 8 bytes
}

const _: () = assert!(size_of::<ExecutionReport>() == 64);

unsafe impl Pod for ExecutionReport {}
unsafe impl Zeroable for ExecutionReport {}

impl ExecutionReport {
    pub fn new_fill(
        sequence: u32,
        order_id: u64,
        exec_id: u64,
        symbol_id: u32,
        side: u8,
        price: u64,
        qty: u64,
        leaves_qty: u64,
        timestamp: u64,
    ) -> Self {
        let exec_type = if leaves_qty == 0 {
            ExecType::Fill as u8
        } else {
            ExecType::PartialFill as u8
        };
        
        Self {
            header: MessageHeader::new(
                MessageType::ExecutionReport as u8,
                (size_of::<Self>() - size_of::<MessageHeader>()) as u16,
                sequence,
            ),
            order_id,
            exec_id,
            symbol_id,
            side,
            exec_type,
            _padding1: 0,
            exec_price: price,
            exec_qty: qty,
            leaves_qty,
            timestamp,
        }
    }
}

/// Quote message (32 bytes).
#[derive(Clone, Copy, Debug, Default)]
#[repr(C, packed)]
pub struct QuoteMessage {
    pub header: MessageHeader,      // 8 bytes
    pub symbol_id: u32,             // 4 bytes
    pub _padding: u32,              // 4 bytes
    pub bid_price: u64,             // 8 bytes
    pub ask_price: u64,             // 8 bytes
}

const _: () = assert!(size_of::<QuoteMessage>() == 32);

unsafe impl Pod for QuoteMessage {}
unsafe impl Zeroable for QuoteMessage {}

/// Trade message (48 bytes).
#[derive(Clone, Copy, Debug, Default)]
#[repr(C, packed)]
pub struct TradeMessage {
    pub header: MessageHeader,      // 8 bytes
    pub symbol_id: u32,             // 4 bytes
    pub side: u8,                   // 1 byte (aggressor side)
    pub _padding: [u8; 3],          // 3 bytes
    pub price: u64,                 // 8 bytes
    pub quantity: u64,              // 8 bytes
    pub timestamp: u64,             // 8 bytes
    pub trade_id: u64,              // 8 bytes
}

const _: () = assert!(size_of::<TradeMessage>() == 48);

unsafe impl Pod for TradeMessage {}
unsafe impl Zeroable for TradeMessage {}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_message_sizes() {
        assert_eq!(size_of::<MessageHeader>(), 8);
        assert_eq!(size_of::<NewOrderMessage>(), 64);
        assert_eq!(size_of::<CancelOrderMessage>(), 32);
        assert_eq!(size_of::<ExecutionReport>(), 64);
    }
    
    #[test]
    fn test_new_order_creation() {
        let msg = NewOrderMessage::new(1, 12345, 42, 0, 0, 10000, 100);
        // Copy values to avoid packed struct reference issues
        let msg_type = msg.header.msg_type;
        let order_id = msg.order_id;
        let symbol_id = msg.symbol_id;
        assert_eq!(msg_type, MessageType::NewOrder as u8);
        assert_eq!(order_id, 12345);
        assert_eq!(symbol_id, 42);
    }
}
