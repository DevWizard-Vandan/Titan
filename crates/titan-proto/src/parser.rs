//! Zero-copy message parser.
//!
//! Uses bytemuck for safe transmutation from raw bytes.

use bytemuck::try_from_bytes;
use core::mem::size_of;
use crate::messages::*;

/// Parse error types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// Buffer doesn't have enough bytes.
    BufferTooSmall,
    /// Invalid message type in header.
    InvalidMessageType,
    /// Message length doesn't match expected.
    InvalidLength,
    /// Buffer is not properly aligned.
    MisalignedBuffer,
}

/// Zero-copy message parser.
pub struct MessageParser;

impl MessageParser {
    /// Parse a message header from raw bytes.
    #[inline(always)]
    pub fn parse_header(buffer: &[u8]) -> Result<&MessageHeader, ParseError> {
        if buffer.len() < size_of::<MessageHeader>() {
            return Err(ParseError::BufferTooSmall);
        }
        
        try_from_bytes(&buffer[..size_of::<MessageHeader>()])
            .map_err(|_| ParseError::MisalignedBuffer)
    }
    
    /// Parse a NewOrder message (zero-copy).
    #[inline(always)]
    pub fn parse_new_order(buffer: &[u8]) -> Result<&NewOrderMessage, ParseError> {
        if buffer.len() < size_of::<NewOrderMessage>() {
            return Err(ParseError::BufferTooSmall);
        }
        
        try_from_bytes(&buffer[..size_of::<NewOrderMessage>()])
            .map_err(|_| ParseError::MisalignedBuffer)
    }
    
    /// Parse a Cancel message (zero-copy).
    #[inline(always)]
    pub fn parse_cancel(buffer: &[u8]) -> Result<&CancelOrderMessage, ParseError> {
        if buffer.len() < size_of::<CancelOrderMessage>() {
            return Err(ParseError::BufferTooSmall);
        }
        
        try_from_bytes(&buffer[..size_of::<CancelOrderMessage>()])
            .map_err(|_| ParseError::MisalignedBuffer)
    }
    
    /// Parse an ExecutionReport (zero-copy).
    #[inline(always)]
    pub fn parse_execution_report(buffer: &[u8]) -> Result<&ExecutionReport, ParseError> {
        if buffer.len() < size_of::<ExecutionReport>() {
            return Err(ParseError::BufferTooSmall);
        }
        
        try_from_bytes(&buffer[..size_of::<ExecutionReport>()])
            .map_err(|_| ParseError::MisalignedBuffer)
    }
    
    /// Determine message type and validate length.
    #[inline]
    pub fn validate_message(buffer: &[u8]) -> Result<(MessageType, usize), ParseError> {
        let header = Self::parse_header(buffer)?;
        
        // Copy the msg_type to avoid reference to packed struct
        let msg_type_byte = header.msg_type;
        
        let msg_type = MessageType::try_from(msg_type_byte)
            .map_err(|_| ParseError::InvalidMessageType)?;
        
        // Copy length to avoid reference to packed struct
        let header_length = header.length;
        
        let expected_len = match msg_type {
            MessageType::NewOrder => size_of::<NewOrderMessage>(),
            MessageType::CancelOrder => size_of::<CancelOrderMessage>(),
            MessageType::ExecutionReport => size_of::<ExecutionReport>(),
            MessageType::Quote => size_of::<QuoteMessage>(),
            MessageType::Trade => size_of::<TradeMessage>(),
            _ => size_of::<MessageHeader>() + header_length as usize,
        };
        
        if buffer.len() < expected_len {
            return Err(ParseError::BufferTooSmall);
        }
        
        Ok((msg_type, expected_len))
    }
}

/// Message builder for outbound messages.
pub struct MessageBuilder {
    sequence: u32,
    exec_id: u64,
}

impl MessageBuilder {
    /// Create a new message builder.
    pub const fn new() -> Self {
        Self {
            sequence: 0,
            exec_id: 0,
        }
    }
    
    /// Get next sequence number.
    #[inline(always)]
    pub fn next_sequence(&mut self) -> u32 {
        self.sequence = self.sequence.wrapping_add(1);
        self.sequence
    }
    
    /// Get next execution ID.
    #[inline(always)]
    pub fn next_exec_id(&mut self) -> u64 {
        self.exec_id += 1;
        self.exec_id
    }
    
    /// Build an execution report into a buffer.
    #[inline(always)]
    pub fn build_execution_report(
        &mut self,
        buffer: &mut [u8],
        order_id: u64,
        symbol_id: u32,
        side: u8,
        price: u64,
        qty: u64,
        leaves_qty: u64,
        timestamp: u64,
    ) -> usize {
        let report = ExecutionReport::new_fill(
            self.next_sequence(),
            order_id,
            self.next_exec_id(),
            symbol_id,
            side,
            price,
            qty,
            leaves_qty,
            timestamp,
        );
        
        let size = size_of::<ExecutionReport>();
        debug_assert!(buffer.len() >= size);
        
        buffer[..size].copy_from_slice(bytemuck::bytes_of(&report));
        size
    }
    
    /// Build a quote message into a buffer.
    #[inline(always)]
    pub fn build_quote(
        &mut self,
        buffer: &mut [u8],
        symbol_id: u32,
        bid_price: u64,
        ask_price: u64,
    ) -> usize {
        let quote = QuoteMessage {
            header: MessageHeader::new(
                MessageType::Quote as u8,
                (size_of::<QuoteMessage>() - size_of::<MessageHeader>()) as u16,
                self.next_sequence(),
            ),
            symbol_id,
            _padding: 0,
            bid_price,
            ask_price,
        };
        
        let size = size_of::<QuoteMessage>();
        buffer[..size].copy_from_slice(bytemuck::bytes_of(&quote));
        size
    }
}

impl Default for MessageBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_new_order() {
        let msg = NewOrderMessage::new(1, 12345, 42, 0, 0, 10000, 100);
        let bytes = bytemuck::bytes_of(&msg);
        
        let parsed = MessageParser::parse_new_order(bytes).unwrap();
        // Copy values to avoid packed struct reference issues
        let order_id = parsed.order_id;
        let symbol_id = parsed.symbol_id;
        let price = parsed.price;
        assert_eq!(order_id, 12345);
        assert_eq!(symbol_id, 42);
        assert_eq!(price, 10000);
    }
    
    #[test]
    fn test_validate_message() {
        let msg = NewOrderMessage::new(1, 12345, 42, 0, 0, 10000, 100);
        let bytes = bytemuck::bytes_of(&msg);
        
        let (msg_type, len) = MessageParser::validate_message(bytes).unwrap();
        assert_eq!(msg_type, MessageType::NewOrder);
        assert_eq!(len, 64);
    }
    
    #[test]
    fn test_buffer_too_small() {
        let buffer = [0u8; 4]; // Too small for header
        let result = MessageParser::parse_header(&buffer);
        assert!(matches!(result, Err(ParseError::BufferTooSmall)));
    }
}
