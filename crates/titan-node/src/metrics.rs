//! Prometheus metrics bridge for Titan.
//!
//! This module bridges the lock-free atomics in titan-core to the
//! Prometheus registry. The bridge runs on a low-priority thread
//! and reads atomics every 1 second.

use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use lazy_static::lazy_static;
use prometheus::{
    self, Encoder, IntCounter, IntGauge, Histogram, HistogramOpts,
    Registry, TextEncoder,
};
use titan_core::{ORDERS_PROCESSED, FILLS_EXECUTED, ORDERS_REJECTED};

lazy_static! {
    /// Custom registry for Titan metrics
    pub static ref REGISTRY: Registry = Registry::new();
    
    // === Order Counters ===
    pub static ref TITAN_ORDERS_TOTAL: IntCounter = IntCounter::new(
        "titan_orders_total",
        "Total orders submitted to the matching engine"
    ).expect("metric creation failed");
    
    pub static ref TITAN_FILLS_TOTAL: IntCounter = IntCounter::new(
        "titan_fills_total", 
        "Total fills executed"
    ).expect("metric creation failed");
    
    pub static ref TITAN_REJECTS_TOTAL: IntCounter = IntCounter::new(
        "titan_rejects_total",
        "Total orders rejected"
    ).expect("metric creation failed");
    
    // === Match Latency Histogram ===
    pub static ref TITAN_MATCH_LATENCY: Histogram = Histogram::with_opts(
        HistogramOpts::new(
            "titan_match_latency_ns",
            "Matching latency in nanoseconds"
        ).buckets(vec![
            100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 
            10000.0, 20000.0, 50000.0, 100000.0
        ])
    ).expect("histogram creation failed");
    
    // === Book Depth Gauges (Top 5 levels) ===
    pub static ref TITAN_BID_LEVEL_1_QTY: IntGauge = IntGauge::new(
        "titan_book_bid_level_1_qty", "Quantity at best bid"
    ).expect("metric creation failed");
    pub static ref TITAN_BID_LEVEL_2_QTY: IntGauge = IntGauge::new(
        "titan_book_bid_level_2_qty", "Quantity at bid level 2"
    ).expect("metric creation failed");
    pub static ref TITAN_BID_LEVEL_3_QTY: IntGauge = IntGauge::new(
        "titan_book_bid_level_3_qty", "Quantity at bid level 3"
    ).expect("metric creation failed");
    pub static ref TITAN_BID_LEVEL_4_QTY: IntGauge = IntGauge::new(
        "titan_book_bid_level_4_qty", "Quantity at bid level 4"
    ).expect("metric creation failed");
    pub static ref TITAN_BID_LEVEL_5_QTY: IntGauge = IntGauge::new(
        "titan_book_bid_level_5_qty", "Quantity at bid level 5"
    ).expect("metric creation failed");
    
    pub static ref TITAN_ASK_LEVEL_1_QTY: IntGauge = IntGauge::new(
        "titan_book_ask_level_1_qty", "Quantity at best ask"
    ).expect("metric creation failed");
    pub static ref TITAN_ASK_LEVEL_2_QTY: IntGauge = IntGauge::new(
        "titan_book_ask_level_2_qty", "Quantity at ask level 2"
    ).expect("metric creation failed");
    pub static ref TITAN_ASK_LEVEL_3_QTY: IntGauge = IntGauge::new(
        "titan_book_ask_level_3_qty", "Quantity at ask level 3"
    ).expect("metric creation failed");
    pub static ref TITAN_ASK_LEVEL_4_QTY: IntGauge = IntGauge::new(
        "titan_book_ask_level_4_qty", "Quantity at ask level 4"
    ).expect("metric creation failed");
    pub static ref TITAN_ASK_LEVEL_5_QTY: IntGauge = IntGauge::new(
        "titan_book_ask_level_5_qty", "Quantity at ask level 5"
    ).expect("metric creation failed");
    
    // === Price Gauges ===
    pub static ref TITAN_BEST_BID: IntGauge = IntGauge::new(
        "titan_best_bid_price", "Best bid price in ticks"
    ).expect("metric creation failed");
    pub static ref TITAN_BEST_ASK: IntGauge = IntGauge::new(
        "titan_best_ask_price", "Best ask price in ticks"  
    ).expect("metric creation failed");
}

/// Initialize and register all metrics with the registry.
pub fn init_metrics() {
    REGISTRY.register(Box::new(TITAN_ORDERS_TOTAL.clone())).unwrap();
    REGISTRY.register(Box::new(TITAN_FILLS_TOTAL.clone())).unwrap();
    REGISTRY.register(Box::new(TITAN_REJECTS_TOTAL.clone())).unwrap();
    REGISTRY.register(Box::new(TITAN_MATCH_LATENCY.clone())).unwrap();
    
    // Book depth
    REGISTRY.register(Box::new(TITAN_BID_LEVEL_1_QTY.clone())).unwrap();
    REGISTRY.register(Box::new(TITAN_BID_LEVEL_2_QTY.clone())).unwrap();
    REGISTRY.register(Box::new(TITAN_BID_LEVEL_3_QTY.clone())).unwrap();
    REGISTRY.register(Box::new(TITAN_BID_LEVEL_4_QTY.clone())).unwrap();
    REGISTRY.register(Box::new(TITAN_BID_LEVEL_5_QTY.clone())).unwrap();
    REGISTRY.register(Box::new(TITAN_ASK_LEVEL_1_QTY.clone())).unwrap();
    REGISTRY.register(Box::new(TITAN_ASK_LEVEL_2_QTY.clone())).unwrap();
    REGISTRY.register(Box::new(TITAN_ASK_LEVEL_3_QTY.clone())).unwrap();
    REGISTRY.register(Box::new(TITAN_ASK_LEVEL_4_QTY.clone())).unwrap();
    REGISTRY.register(Box::new(TITAN_ASK_LEVEL_5_QTY.clone())).unwrap();
    
    REGISTRY.register(Box::new(TITAN_BEST_BID.clone())).unwrap();
    REGISTRY.register(Box::new(TITAN_BEST_ASK.clone())).unwrap();
}

/// Previous values for delta calculation
struct MetricsBridge {
    last_orders: u64,
    last_fills: u64,
    last_rejects: u64,
}

impl MetricsBridge {
    fn new() -> Self {
        Self {
            last_orders: 0,
            last_fills: 0,
            last_rejects: 0,
        }
    }
    
    /// Update counters from atomics (called every 1s)
    fn update_from_atomics(&mut self) {
        let current_orders = ORDERS_PROCESSED.load(Ordering::Relaxed);
        let current_fills = FILLS_EXECUTED.load(Ordering::Relaxed);
        let current_rejects = ORDERS_REJECTED.load(Ordering::Relaxed);
        
        // Calculate deltas and update prometheus counters
        if current_orders > self.last_orders {
            TITAN_ORDERS_TOTAL.inc_by(current_orders - self.last_orders);
        }
        if current_fills > self.last_fills {
            TITAN_FILLS_TOTAL.inc_by(current_fills - self.last_fills);
        }
        if current_rejects > self.last_rejects {
            TITAN_REJECTS_TOTAL.inc_by(current_rejects - self.last_rejects);
        }
        
        self.last_orders = current_orders;
        self.last_fills = current_fills;
        self.last_rejects = current_rejects;
    }
}

/// Spawn the metrics bridge thread.
/// Returns a handle to the thread.
pub fn spawn_metrics_thread() -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("titan-metrics".to_string())
        .spawn(|| {
            let mut bridge = MetricsBridge::new();
            
            loop {
                thread::sleep(Duration::from_secs(1));
                bridge.update_from_atomics();
            }
        })
        .expect("Failed to spawn metrics thread")
}

/// Spawn the HTTP server for /metrics endpoint.
pub fn spawn_http_server(port: u16) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("titan-http".to_string())
        .spawn(move || {
            let addr = format!("0.0.0.0:{}", port);
            let server = tiny_http::Server::http(&addr)
                .expect("Failed to start HTTP server");
            
            println!("📊 Metrics server listening on http://{}/metrics", addr);
            
            for request in server.incoming_requests() {
                let response = match request.url() {
                    "/metrics" => {
                        let encoder = TextEncoder::new();
                        let mut buffer = Vec::new();
                        let metric_families = REGISTRY.gather();
                        encoder.encode(&metric_families, &mut buffer).unwrap();
                        
                        tiny_http::Response::from_data(buffer)
                            .with_header(
                                tiny_http::Header::from_bytes(
                                    &b"Content-Type"[..],
                                    &b"text/plain; charset=utf-8"[..]
                                ).unwrap()
                            )
                    }
                    "/health" => {
                        tiny_http::Response::from_string("OK")
                    }
                    _ => {
                        tiny_http::Response::from_string("Not Found")
                            .with_status_code(404)
                    }
                };
                
                let _ = request.respond(response);
            }
        })
        .expect("Failed to spawn HTTP server thread")
}

/// Get current metrics as Prometheus text format.
pub fn get_metrics_text() -> String {
    let encoder = TextEncoder::new();
    let mut buffer = Vec::new();
    let metric_families = REGISTRY.gather();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    String::from_utf8(buffer).unwrap()
}

/// Update book depth metrics from the engine.
/// Called by the engine thread periodically.
pub fn update_book_depth(
    bid_levels: &[(u64, u64)],  // (price_ticks, qty)
    ask_levels: &[(u64, u64)],
) {
    let bid_gauges = [
        &*TITAN_BID_LEVEL_1_QTY,
        &*TITAN_BID_LEVEL_2_QTY,
        &*TITAN_BID_LEVEL_3_QTY,
        &*TITAN_BID_LEVEL_4_QTY,
        &*TITAN_BID_LEVEL_5_QTY,
    ];
    
    let ask_gauges = [
        &*TITAN_ASK_LEVEL_1_QTY,
        &*TITAN_ASK_LEVEL_2_QTY,
        &*TITAN_ASK_LEVEL_3_QTY,
        &*TITAN_ASK_LEVEL_4_QTY,
        &*TITAN_ASK_LEVEL_5_QTY,
    ];
    
    for (i, gauge) in bid_gauges.iter().enumerate() {
        let qty = bid_levels.get(i).map(|(_, q)| *q as i64).unwrap_or(0);
        gauge.set(qty);
    }
    
    for (i, gauge) in ask_gauges.iter().enumerate() {
        let qty = ask_levels.get(i).map(|(_, q)| *q as i64).unwrap_or(0);
        gauge.set(qty);
    }
    
    // Update best bid/ask prices
    if let Some((price, _)) = bid_levels.first() {
        TITAN_BEST_BID.set(*price as i64);
    }
    if let Some((price, _)) = ask_levels.first() {
        TITAN_BEST_ASK.set(*price as i64);
    }
}
