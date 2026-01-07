//! Market data publisher implementation.
//!
//! Uses UDP for low-latency market data dissemination.

use std::net::{UdpSocket, SocketAddr};
use std::io;

use titan_proto::{MessageBuilder, TradeMessage, QuoteMessage, MessageHeader, MessageType};

/// Market data publisher.
pub struct Publisher {
    socket: UdpSocket,
    dest_addr: SocketAddr,
    builder: MessageBuilder,
    buffer: [u8; 512],
}

impl Publisher {
    /// Create a new publisher.
    ///
    /// For multicast, use a multicast group address (e.g., "239.255.0.1:12345").
    /// For unicast, use the destination address directly.
    pub fn new(dest_addr: &str) -> io::Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.set_nonblocking(true)?;
        
        let dest: SocketAddr = dest_addr.parse().map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidInput, e)
        })?;
        
        // For multicast, set TTL
        if dest.ip().is_multicast() {
            socket.set_multicast_ttl_v4(4)?;
        }
        
        Ok(Self {
            socket,
            dest_addr: dest,
            builder: MessageBuilder::new(),
            buffer: [0; 512],
        })
    }
    
    /// Publish a trade.
    pub fn publish_trade(
        &mut self,
        symbol_id: u32,
        side: u8,
        price: u64,
        quantity: u64,
        timestamp: u64,
        trade_id: u64,
    ) -> io::Result<()> {
        let seq = self.builder.next_sequence();
        
        let trade = TradeMessage {
            header: MessageHeader::new(
                MessageType::Trade as u8,
                (core::mem::size_of::<TradeMessage>() - core::mem::size_of::<MessageHeader>()) as u16,
                seq,
            ),
            symbol_id,
            side,
            _padding: [0; 3],
            price,
            quantity,
            timestamp,
            trade_id,
        };
        
        let bytes = bytemuck::bytes_of(&trade);
        self.buffer[..bytes.len()].copy_from_slice(bytes);
        
        match self.socket.send_to(&self.buffer[..bytes.len()], self.dest_addr) {
            Ok(_) => Ok(()),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(e) => Err(e),
        }
    }
    
    /// Publish a quote update.
    pub fn publish_quote(
        &mut self,
        symbol_id: u32,
        bid_price: u64,
        ask_price: u64,
    ) -> io::Result<()> {
        let size = self.builder.build_quote(&mut self.buffer, symbol_id, bid_price, ask_price);
        
        match self.socket.send_to(&self.buffer[..size], self.dest_addr) {
            Ok(_) => Ok(()),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(e) => Err(e),
        }
    }
    
    /// Publish execution report.
    pub fn publish_execution(
        &mut self,
        order_id: u64,
        symbol_id: u32,
        side: u8,
        price: u64,
        qty: u64,
        leaves_qty: u64,
        timestamp: u64,
    ) -> io::Result<()> {
        let size = self.builder.build_execution_report(
            &mut self.buffer,
            order_id,
            symbol_id,
            side,
            price,
            qty,
            leaves_qty,
            timestamp,
        );
        
        match self.socket.send_to(&self.buffer[..size], self.dest_addr) {
            Ok(_) => Ok(()),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(e) => Err(e),
        }
    }
}
