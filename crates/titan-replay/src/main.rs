//! Titan Replay - Historical data replay and benchmarking.
//!
//! This binary replays market data through the matching engine
//! and measures latency distributions.

use std::time::Instant;

use titan_core::{
    MatchingEngine, Order, OrderId, SymbolId, Side, OrderType,
    Price, Quantity,
};
use titan_metrics::LatencyHistogram;

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

fn main() {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║                    TITAN REPLAY ENGINE                        ║");
    println!("║           Low-Latency Matching Engine Benchmark              ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();
    
    // Create engine with 1M order capacity
    let mut engine = MatchingEngine::new(SymbolId(1), 20, Price::ZERO);
    let mut gen = OrderGenerator::new(SymbolId(1));
    let mut latency = LatencyHistogram::new();
    
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
    let insert_count = 100_000u64;
    let start = Instant::now();
    
    for i in 0..insert_count {
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
    let match_count = 50_000u64;
    let start = Instant::now();
    
    for i in 0..match_count {
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
    let mixed_count = 100_000u64;
    
    // Reset engine
    engine = MatchingEngine::new(SymbolId(1), 20, Price::ZERO);
    gen = OrderGenerator::new(SymbolId(1));
    
    let start = Instant::now();
    
    for i in 0..mixed_count {
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
