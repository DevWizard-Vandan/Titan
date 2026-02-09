//! Titan Replay - Historical data replay and benchmarking.
//!
//! This binary replays market data through the matching engine
//! and measures latency distributions.
//!
//! # Modes
//! - `synthetic`: Generate synthetic orders locally (default, for benchmarking)
//! - `csv`: Replay orders from a CSV file via TCP to the gateway

use std::fs::File;
use std::io::{BufReader, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use clap::{Parser, ValueEnum};
use titan_core::{
    MatchingEngine, Order, OrderId, SymbolId, Side, OrderType,
    Price, Quantity,
};
use titan_metrics::LatencyHistogram;

/// Replay mode
#[derive(Debug, Clone, Copy, ValueEnum)]
enum Mode {
    /// Synthetic benchmark (local engine)
    Synthetic,
    /// CSV replay via TCP
    Csv,
}

/// Titan Replay - Market data replay and benchmarking tool
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Replay mode
    #[arg(short, long, value_enum, default_value = "synthetic")]
    mode: Mode,
    
    /// CSV file path (required for csv mode)
    #[arg(short, long)]
    file: Option<String>,
    
    /// Gateway host address
    #[arg(long, default_value = "127.0.0.1:8080")]
    host: String,
    
    /// Rate limit in orders per second (0 = unlimited)
    #[arg(short, long, default_value = "0")]
    rate_limit: u64,
    
    /// Enable time travel mode (busy spin to CSV timestamps)
    #[arg(long, default_value = "false")]
    time_travel: bool,
    
    /// Number of orders for synthetic mode
    #[arg(short, long, default_value = "100000")]
    count: u64,
}

/// CSV record format
#[derive(Debug, serde::Deserialize)]
struct CsvOrder {
    timestamp: u64,
    symbol: u64,
    #[serde(rename = "type")]
    order_type: String,
    side: String,
    price: u64,
    qty: u64,
}

/// Synthetic order generator for benchmarking.
struct OrderGenerator {
    next_id: u64,
    symbol: SymbolId,
}

impl OrderGenerator {
    fn new(symbol: SymbolId) -> Self {
        Self { next_id: 1, symbol }
    }
    
    fn next_buy(&mut self, price: u64, qty: u64) -> Order {
        let id = self.next_id;
        self.next_id += 1;
        Order::new(
            OrderId(id),
            self.symbol,
            Side::Buy,
            OrderType::Limit,
            Price::from_ticks(price),
            Quantity(qty),
            0,
        )
    }
    
    fn next_sell(&mut self, price: u64, qty: u64) -> Order {
        let id = self.next_id;
        self.next_id += 1;
        Order::new(
            OrderId(id),
            self.symbol,
            Side::Sell,
            OrderType::Limit,
            Price::from_ticks(price),
            Quantity(qty),
            0,
        )
    }
    
    fn next_ioc_buy(&mut self, price: u64, qty: u64) -> Order {
        let id = self.next_id;
        self.next_id += 1;
        Order::new(
            OrderId(id),
            self.symbol,
            Side::Buy,
            OrderType::IOC,
            Price::from_ticks(price),
            Quantity(qty),
            0,
        )
    }
}

/// Rate limiter using token bucket algorithm
struct RateLimiter {
    tokens_per_sec: u64,
    last_refill: Instant,
    tokens: f64,
}

impl RateLimiter {
    fn new(rate: u64) -> Self {
        Self {
            tokens_per_sec: rate,
            last_refill: Instant::now(),
            tokens: rate as f64,
        }
    }
    
    /// Wait until a token is available
    fn acquire(&mut self) {
        if self.tokens_per_sec == 0 {
            return; // Unlimited
        }
        
        // Refill tokens
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens += elapsed * self.tokens_per_sec as f64;
        self.tokens = self.tokens.min(self.tokens_per_sec as f64);
        self.last_refill = now;
        
        // Wait for token
        while self.tokens < 1.0 {
            std::thread::sleep(Duration::from_micros(10));
            let now = Instant::now();
            let elapsed = now.duration_since(self.last_refill).as_secs_f64();
            self.tokens += elapsed * self.tokens_per_sec as f64;
            self.last_refill = now;
        }
        
        self.tokens -= 1.0;
    }
}

fn main() {
    let args = Args::parse();
    
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║                    TITAN REPLAY ENGINE                        ║");
    println!("║           Low-Latency Matching Engine Benchmark              ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();
    
    match args.mode {
        Mode::Synthetic => run_synthetic_benchmark(&args),
        Mode::Csv => run_csv_replay(&args),
    }
}

/// Run synthetic benchmark (local engine)
fn run_synthetic_benchmark(args: &Args) {
    println!("🔧 Mode: Synthetic Benchmark");
    println!("📊 Orders: {}", args.count);
    if args.rate_limit > 0 {
        println!("⏱️  Rate Limit: {} orders/sec", args.rate_limit);
    }
    println!();
    
    // Create engine with 1M order capacity
    let mut engine = MatchingEngine::new(SymbolId(1), 20, Price::ZERO);
    let mut gen = OrderGenerator::new(SymbolId(1));
    let mut latency = LatencyHistogram::new();
    let mut rate_limiter = RateLimiter::new(args.rate_limit);
    
    // Warm up
    println!("[1/4] Warming up...");
    for _ in 0..10000 {
        let order = gen.next_buy(10000, 100);
        engine.submit_order(order, 0);
    }
    
    // Clear for benchmark
    engine = MatchingEngine::new(SymbolId(1), 20, Price::ZERO);
    gen = OrderGenerator::new(SymbolId(1));
    
    // Phase 1: Insertion benchmark
    println!("[2/4] Benchmarking insertions...");
    let insert_count = args.count;
    let start = Instant::now();
    
    for i in 0..insert_count {
        rate_limiter.acquire();
        let order_start = Instant::now();
        
        let price = 10000 + (i % 100);
        let side = if i % 2 == 0 { 
            gen.next_buy(price, 100)
        } else {
            gen.next_sell(price + 100, 100) // Spread to avoid matching
        };
        engine.submit_order(side, i);
        
        let elapsed_ns = order_start.elapsed().as_nanos() as u64;
        latency.record(elapsed_ns);
    }
    
    let insert_elapsed = start.elapsed();
    let insert_rate = insert_count as f64 / insert_elapsed.as_secs_f64();
    
    println!("   Inserted {} orders in {:.2?}", insert_count, insert_elapsed);
    println!("   Rate: {:.0} orders/sec", insert_rate);
    latency.print_summary("   Insert Latency");
    
    // Phase 2: Matching benchmark
    println!("\n[3/4] Benchmarking matching...");
    let mut match_latency = LatencyHistogram::new();
    let match_count = insert_count / 2;
    let start = Instant::now();
    
    for i in 0..match_count {
        rate_limiter.acquire();
        let order_start = Instant::now();
        
        // Create IOC order that will match against resting liquidity
        let price = 10100; // Will cross the spread
        let order = gen.next_ioc_buy(price, 50);
        engine.submit_order(order, insert_count + i);
        
        let elapsed_ns = order_start.elapsed().as_nanos() as u64;
        match_latency.record(elapsed_ns);
    }
    
    let match_elapsed = start.elapsed();
    let match_rate = match_count as f64 / match_elapsed.as_secs_f64();
    
    println!("   Matched {} orders in {:.2?}", match_count, match_elapsed);
    println!("   Rate: {:.0} matches/sec", match_rate);
    match_latency.print_summary("   Match Latency");
    
    // Phase 3: Mixed workload
    println!("\n[4/4] Benchmarking mixed workload...");
    let mut mixed_latency = LatencyHistogram::new();
    let mixed_count = args.count;
    
    // Reset engine
    engine = MatchingEngine::new(SymbolId(1), 20, Price::ZERO);
    gen = OrderGenerator::new(SymbolId(1));
    
    let start = Instant::now();
    
    for i in 0..mixed_count {
        rate_limiter.acquire();
        let order_start = Instant::now();
        
        // Mix of inserts and matches
        let order = match i % 10 {
            0..=6 => gen.next_buy(10000 + (i % 50), 100),  // 70% passive buys
            7..=8 => gen.next_sell(10000 + (i % 50), 100), // 20% passive sells
            _ => gen.next_ioc_buy(10100, 50),              // 10% aggressive
        };
        engine.submit_order(order, i);
        
        let elapsed_ns = order_start.elapsed().as_nanos() as u64;
        mixed_latency.record(elapsed_ns);
    }
    
    let mixed_elapsed = start.elapsed();
    let mixed_rate = mixed_count as f64 / mixed_elapsed.as_secs_f64();
    
    println!("   Processed {} orders in {:.2?}", mixed_count, mixed_elapsed);
    println!("   Rate: {:.0} orders/sec", mixed_rate);
    mixed_latency.print_summary("   Mixed Latency");
    
    // Summary
    print_summary(insert_rate, match_rate, mixed_rate, &engine);
}

/// Run CSV replay via TCP
fn run_csv_replay(args: &Args) {
    let file_path = args.file.as_ref().expect("CSV file path required for csv mode");
    
    println!("📁 Mode: CSV Replay");
    println!("📄 File: {}", file_path);
    println!("🌐 Target: {}", args.host);
    if args.rate_limit > 0 {
        println!("⏱️  Rate Limit: {} orders/sec", args.rate_limit);
    }
    if args.time_travel {
        println!("⏰ Time Travel: Enabled (busy spin to timestamps)");
    }
    println!();
    
    // Open CSV file
    let file = File::open(file_path).expect("Failed to open CSV file");
    let reader = BufReader::new(file);
    let mut csv_reader = csv::Reader::from_reader(reader);
    
    // Connect to gateway
    println!("🔌 Connecting to gateway at {}...", args.host);
    let mut stream = match TcpStream::connect(&args.host) {
        Ok(s) => {
            println!("✅ Connected!");
            s
        }
        Err(e) => {
            eprintln!("❌ Failed to connect: {}", e);
            eprintln!("   Make sure titan-node is running.");
            return;
        }
    };
    
    let mut rate_limiter = RateLimiter::new(args.rate_limit);
    let mut latency = LatencyHistogram::new();
    let mut order_count = 0u64;
    let start = Instant::now();
    let replay_start_time = if args.time_travel {
        Some(Instant::now())
    } else {
        None
    };
    let mut first_timestamp: Option<u64> = None;
    
    // Process CSV records
    for result in csv_reader.deserialize() {
        let record: CsvOrder = match result {
            Ok(r) => r,
            Err(e) => {
                eprintln!("⚠️  Failed to parse CSV row: {}", e);
                continue;
            }
        };
        
        // Time travel: busy spin until timestamp
        if let Some(start_time) = replay_start_time {
            if first_timestamp.is_none() {
                first_timestamp = Some(record.timestamp);
            }
            
            let offset_ns = record.timestamp.saturating_sub(first_timestamp.unwrap());
            let target_time = start_time + Duration::from_nanos(offset_ns);
            
            // Busy spin (precise timing)
            while Instant::now() < target_time {
                std::hint::spin_loop();
            }
        }
        
        // Rate limit
        rate_limiter.acquire();
        
        // Convert to wire format and send
        let order_start = Instant::now();
        
        // Parse side
        let side = match record.side.to_lowercase().as_str() {
            "buy" => 0,
            "sell" => 1,
            _ => 0, // Default to buy on error
        };
        
        // Parse type
        let order_type = match record.order_type.to_lowercase().as_str() {
            "limit" => 0,
            "ioc" => 1,
            "fok" => 2,
            "post_only" => 3,
            _ => 0, // Default to limit
        };
        
        // Create binary message
        // Using order_count as sequence number
        let sequence = (order_count + 1) as u32;
        let msg = titan_proto::NewOrderMessage::new(
            sequence,
            order_count + 1,        // order_id
            record.symbol as u32,   // symbol_id
            side,                   // side
            order_type,             // order_type
            record.price,           // price
            record.qty              // qty
        );
        
        // Safety: Casting the struct to a byte slice
        let msg_bytes = unsafe {
            std::slice::from_raw_parts(
                &msg as *const _ as *const u8,
                std::mem::size_of::<titan_proto::NewOrderMessage>()
            )
        };
        
        if let Err(e) = stream.write_all(msg_bytes) {
            eprintln!("❌ Failed to send order: {}", e);
            break;
        }
        
        let elapsed_ns = order_start.elapsed().as_nanos() as u64;
        latency.record(elapsed_ns);
        order_count += 1;
        
        // Progress update every 10k orders
        if order_count % 10000 == 0 {
            let rate = order_count as f64 / start.elapsed().as_secs_f64();
            println!("   📤 Sent {} orders ({:.0} orders/sec)", order_count, rate);
        }
    }
    
    let elapsed = start.elapsed();
    let rate = order_count as f64 / elapsed.as_secs_f64();
    
    println!();
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║                     CSV REPLAY COMPLETE                       ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║  Orders Sent:     {:>12}                             ║", order_count);
    println!("║  Elapsed Time:    {:>12.2?}                             ║", elapsed);
    println!("║  Send Rate:       {:>12.0} orders/sec                  ║", rate);
    println!("╚══════════════════════════════════════════════════════════════╝");
    
    latency.print_summary("   Send Latency");
}

fn print_summary(insert_rate: f64, match_rate: f64, mixed_rate: f64, engine: &MatchingEngine) {
    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!("║                      BENCHMARK SUMMARY                        ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║  Insert Rate:     {:>12.0} orders/sec                    ║", insert_rate);
    println!("║  Match Rate:      {:>12.0} orders/sec                    ║", match_rate);
    println!("║  Mixed Rate:      {:>12.0} orders/sec                    ║", mixed_rate);
    println!("╠══════════════════════════════════════════════════════════════╣");
    
    let (active, capacity) = engine.pool_stats();
    println!("║  Pool Usage:      {:>12} / {:>12}           ║", active, capacity);
    println!("╚══════════════════════════════════════════════════════════════╝");
    
    // Performance assessment
    println!();
    if mixed_rate > 1_000_000.0 {
        println!("✅ PASS: Achieved >1M orders/sec target!");
    } else if mixed_rate > 500_000.0 {
        println!("⚠️  CLOSE: {:.0} orders/sec (target: 1M)", mixed_rate);
    } else {
        println!("❌ NEEDS WORK: {:.0} orders/sec (target: 1M)", mixed_rate);
    }
}
