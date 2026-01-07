//! Network gateway implementation using mio.
//!
//! This provides a non-blocking TCP server that feeds orders
//! into the matching engine via the ring buffer.

use mio::{Events, Interest, Poll, Token};
use mio::net::{TcpListener, TcpStream};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::SocketAddr;

use titan_proto::{MessageParser, MessageType};

const SERVER: Token = Token(0);
const MAX_CONNECTIONS: usize = 1024;
const READ_BUFFER_SIZE: usize = 4096;
const WRITE_BUFFER_SIZE: usize = 4096;

/// Per-connection state.
pub struct Connection {
    stream: TcpStream,
    read_buffer: [u8; READ_BUFFER_SIZE],
    read_pos: usize,
    write_buffer: [u8; WRITE_BUFFER_SIZE],
    write_pos: usize,
    write_len: usize,
    addr: SocketAddr,
}

impl Connection {
    fn new(stream: TcpStream, addr: SocketAddr) -> Self {
        Self {
            stream,
            read_buffer: [0; READ_BUFFER_SIZE],
            read_pos: 0,
            write_buffer: [0; WRITE_BUFFER_SIZE],
            write_pos: 0,
            write_len: 0,
            addr,
        }
    }
    
    /// Queue data for writing.
    pub fn queue_write(&mut self, data: &[u8]) -> bool {
        let available = WRITE_BUFFER_SIZE - self.write_len;
        if data.len() > available {
            return false;
        }
        
        self.write_buffer[self.write_len..self.write_len + data.len()].copy_from_slice(data);
        self.write_len += data.len();
        true
    }
    
    /// Get address.
    #[allow(dead_code)]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

/// Gateway event type for order processing.
#[derive(Clone, Copy, Debug)]
pub enum GatewayEvent {
    /// New order received.
    NewOrder {
        token: Token,
        order_id: u64,
        symbol_id: u32,
        side: u8,
        order_type: u8,
        price: u64,
        quantity: u64,
    },
    /// Cancel order received.
    CancelOrder {
        token: Token,
        order_id: u64,
        symbol_id: u32,
    },
    /// Connection established.
    Connected { token: Token },
    /// Connection closed.
    Disconnected { token: Token },
}

/// Network gateway.
pub struct Gateway {
    poll: Poll,
    listener: TcpListener,
    connections: HashMap<Token, Connection>,
    next_token: usize,
    events: Vec<GatewayEvent>,
}

impl Gateway {
    /// Create a new gateway bound to the specified address.
    pub fn bind(addr: &str) -> io::Result<Self> {
        let poll = Poll::new()?;
        let addr: SocketAddr = addr.parse().map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidInput, e)
        })?;
        
        let mut listener = TcpListener::bind(addr)?;
        poll.registry().register(&mut listener, SERVER, Interest::READABLE)?;
        
        Ok(Self {
            poll,
            listener,
            connections: HashMap::with_capacity(MAX_CONNECTIONS),
            next_token: 1,
            events: Vec::with_capacity(256),
        })
    }
    
    /// Poll for events with optional timeout (in milliseconds).
    /// Returns slice of gateway events.
    pub fn poll(&mut self, timeout_ms: Option<u64>) -> io::Result<&[GatewayEvent]> {
        self.events.clear();
        
        let mut mio_events = Events::with_capacity(256);
        let timeout = timeout_ms.map(std::time::Duration::from_millis);
        
        self.poll.poll(&mut mio_events, timeout)?;
        
        for event in mio_events.iter() {
            match event.token() {
                SERVER => self.accept_connections()?,
                token => {
                    let is_readable = event.is_readable();
                    let is_writable = event.is_writable();
                    self.handle_connection(token, is_readable, is_writable)?;
                }
            }
        }
        
        Ok(&self.events)
    }
    
    /// Poll with zero timeout (non-blocking).
    pub fn poll_immediate(&mut self) -> io::Result<&[GatewayEvent]> {
        self.poll(Some(0))
    }
    
    fn accept_connections(&mut self) -> io::Result<()> {
        loop {
            match self.listener.accept() {
                Ok((mut stream, addr)) => {
                    let token = Token(self.next_token);
                    self.next_token += 1;
                    
                    stream.set_nodelay(true)?;
                    
                    self.poll.registry().register(
                        &mut stream,
                        token,
                        Interest::READABLE | Interest::WRITABLE,
                    )?;
                    
                    self.connections.insert(token, Connection::new(stream, addr));
                    self.events.push(GatewayEvent::Connected { token });
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }
        
        Ok(())
    }
    
    fn handle_connection(&mut self, token: Token, is_readable: bool, is_writable: bool) -> io::Result<()> {
        if is_readable {
            if let Some(should_close) = self.read_from_connection(token)? {
                if should_close {
                    self.remove_connection(token);
                    self.events.push(GatewayEvent::Disconnected { token });
                    return Ok(());
                }
            }
        }
        
        if is_writable {
            self.write_to_connection(token)?;
        }
        
        Ok(())
    }
    
    fn read_from_connection(&mut self, token: Token) -> io::Result<Option<bool>> {
        let conn = match self.connections.get_mut(&token) {
            Some(c) => c,
            None => return Ok(None),
        };
        
        loop {
            match conn.stream.read(&mut conn.read_buffer[conn.read_pos..]) {
                Ok(0) => {
                    // Connection closed
                    return Ok(Some(true));
                }
                Ok(n) => {
                    conn.read_pos += n;
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    return Ok(Some(true));
                }
            }
        }
        
        // Parse messages from the read buffer
        self.parse_messages(token);
        
        Ok(Some(false))
    }
    
    fn parse_messages(&mut self, token: Token) {
        let conn = match self.connections.get_mut(&token) {
            Some(c) => c,
            None => return,
        };
        
        let mut consumed = 0;
        
        while consumed + 8 <= conn.read_pos {
            let buffer = &conn.read_buffer[consumed..conn.read_pos];
            
            // Validate and get message length
            let (msg_type, msg_len) = match MessageParser::validate_message(buffer) {
                Ok((t, l)) => (t, l),
                Err(_) => break,
            };
            
            if consumed + msg_len > conn.read_pos {
                break; // Incomplete message
            }
            
            // Parse based on type
            match msg_type {
                MessageType::NewOrder => {
                    if let Ok(order) = MessageParser::parse_new_order(buffer) {
                        self.events.push(GatewayEvent::NewOrder {
                            token,
                            order_id: order.order_id,
                            symbol_id: order.symbol_id,
                            side: order.side,
                            order_type: order.order_type,
                            price: order.price,
                            quantity: order.quantity,
                        });
                    }
                }
                MessageType::CancelOrder => {
                    if let Ok(cancel) = MessageParser::parse_cancel(buffer) {
                        self.events.push(GatewayEvent::CancelOrder {
                            token,
                            order_id: cancel.order_id,
                            symbol_id: cancel.symbol_id,
                        });
                    }
                }
                _ => {}
            }
            
            consumed += msg_len;
        }
        
        // Compact buffer
        if consumed > 0 {
            let conn = self.connections.get_mut(&token).unwrap();
            conn.read_buffer.copy_within(consumed..conn.read_pos, 0);
            conn.read_pos -= consumed;
        }
    }
    
    fn write_to_connection(&mut self, token: Token) -> io::Result<()> {
        let conn = match self.connections.get_mut(&token) {
            Some(c) => c,
            None => return Ok(()),
        };
        
        while conn.write_pos < conn.write_len {
            match conn.stream.write(&conn.write_buffer[conn.write_pos..conn.write_len]) {
                Ok(n) => conn.write_pos += n,
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    // Connection error, will be handled on next read
                    break;
                }
            }
        }
        
        if conn.write_pos == conn.write_len {
            conn.write_pos = 0;
            conn.write_len = 0;
        }
        
        Ok(())
    }
    
    fn remove_connection(&mut self, token: Token) {
        if let Some(mut conn) = self.connections.remove(&token) {
            let _ = self.poll.registry().deregister(&mut conn.stream);
        }
    }
    
    /// Send data to a connection.
    pub fn send(&mut self, token: Token, data: &[u8]) -> bool {
        if let Some(conn) = self.connections.get_mut(&token) {
            conn.queue_write(data)
        } else {
            false
        }
    }
    
    /// Get number of active connections.
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }
}
