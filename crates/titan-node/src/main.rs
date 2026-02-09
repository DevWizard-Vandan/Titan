//! Titan Node - Production matching engine orchestrator.
//!
//! This binary spawns and coordinates all engine components:
//! - Engine Thread: CPU-pinned, hot path matching
//! - Network Thread: TCP gateway for order ingestion  
//! - Metrics Thread: Prometheus metrics bridge
//! - Snapshot Thread: Background persistence

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use titan_core::{MatchingEngine, Price, SymbolId};
use titan_node::metrics::{self, update_book_depth};
use titan_node::snapshot::SnapshotManager;

/// Orders between snapshots
const SNAPSHOT_INTERVAL: u64 = 100_000;

/// Shared engine state accessible across threads
pub struct EngineState {
    /// Order counter for snapshot triggers
    pub order_count: AtomicU64,
    /// Shutdown signal
    pub shutdown: AtomicBool,
}

impl EngineState {
    pub fn new() -> Self {
        Self {
            order_count: AtomicU64::new(0),
            shutdown: AtomicBool::new(false),
        }
    }
}

fn main() {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║                      TITAN NODE v0.1.0                       ║");
    println!("║            Production Matching Engine Orchestrator           ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();
    
    // Initialize metrics registry
    metrics::init_metrics();
    println!("📊 Metrics registry initialized");
    
    // Initialize snapshot manager
    let snapshot_manager = SnapshotManager::new("./data/snapshots")
        .expect("Failed to initialize snapshot manager");
    println!("💾 Snapshot manager initialized");
    
    // Check for existing snapshot to recover from
    match snapshot_manager.load_latest() {
        Ok(Some((seq, _data))) => {
            println!("🔄 Found snapshot at sequence {}, recovery available", seq);
            // TODO: Deserialize and restore engine state
        }
        Ok(None) => {
            println!("📦 No existing snapshot found, starting fresh");
        }
        Err(e) => {
            eprintln!("⚠️  Failed to check for snapshots: {}", e);
        }
    }
    
    // Shared state
    let state = Arc::new(EngineState::new());
    
    // Spawn metrics threads
    let _metrics_thread = metrics::spawn_metrics_thread();
    let _http_thread = metrics::spawn_http_server(9090);
    
    // Create matching engine
    let mut engine = MatchingEngine::new(
        SymbolId(1),
        20,  // 1M order pool capacity
        Price::ZERO,
    );
    println!("⚡ Matching engine initialized (1M order capacity)");
    
    // Channel for Gateway -> Engine
    let (order_tx, order_rx) = crossbeam_channel::bounded::<titan_net::gateway::GatewayEvent>(4096);
    
    // Spawn Gateway Thread
    thread::Builder::new()
        .name("titan-gateway".to_string())
        .spawn(move || {
            let mut gateway = titan_net::Gateway::bind("0.0.0.0:8080")
                .expect("Failed to bind gateway to 0.0.0.0:8080");
            
            println!("🌐 Gateway listening on tcp://0.0.0.0:8080");
            
            loop {
                match gateway.poll(Some(1000)) {
                    Ok(events) => {
                        for event in events {
                            // Forward all relevant events to the engine
                            match event {
                                titan_net::gateway::GatewayEvent::NewOrder { .. } => {
                                    let _ = order_tx.send(*event);
                                }
                                _ => {} // Ignore connection events and cancels for now
                            }
                        }
                    }
                    Err(e) => eprintln!("Gateway poll error: {}", e),
                }
            }
        })
        .expect("Failed to spawn gateway thread");
    
    // Try to pin to CPU core (optional, best-effort)
    if let Some(core_ids) = core_affinity::get_core_ids() {
        if let Some(core_id) = core_ids.first() {
            if core_affinity::set_for_current(*core_id) {
                println!("📍 Engine thread pinned to CPU core {:?}", core_id);
            }
        }
    }
    
    println!();
    println!("🚀 Titan Node is running!");
    println!("   Gateway:  tcp://0.0.0.0:8080");
    println!("   Metrics:  http://0.0.0.0:9090/metrics");
    println!("   Health:   http://0.0.0.0:9090/health");
    println!();
    println!("Press Ctrl+C to shutdown...");
    println!();
    
    // Setup Ctrl+C handler
    let state_clone = Arc::clone(&state);
    ctrlc_handler(state_clone);
    
    // Main loop - update book depth metrics periodically
    let mut last_depth_update = std::time::Instant::now();
    
    while !state.shutdown.load(Ordering::Relaxed) {
        // Drain incoming orders from gateway
        while let Ok(event) = order_rx.try_recv() {
            match event {
                titan_net::gateway::GatewayEvent::NewOrder { 
                    order_id, symbol_id, side, order_type, price, quantity, .. 
                } => {
                    let side = if side == 0 { titan_core::Side::Buy } else { titan_core::Side::Sell };
                    let order_type = match order_type {
                        0 => titan_core::OrderType::Limit,
                        1 => titan_core::OrderType::IOC,
                        2 => titan_core::OrderType::FOK,
                        3 => titan_core::OrderType::PostOnly,
                        _ => titan_core::OrderType::Limit,
                    };
                    
                    let order = titan_core::Order::new(
                        titan_core::OrderId(order_id),
                        titan_core::SymbolId(symbol_id),
                        side,
                        order_type,
                        titan_core::Price::from_ticks(price),
                        titan_core::Quantity(quantity),
                        0, // timestamp placeholder
                    );
                    
                    // Submit to engine
                    // Using order_id as timestamp for consistency in this demo
                    engine.submit_order(order, order_id);
                    state.order_count.fetch_add(1, Ordering::Relaxed);
                }
                _ => {}
            }
        }

        // Update book depth metrics every 100ms
        if last_depth_update.elapsed() >= Duration::from_millis(100) {
            let bid_levels: Vec<(u64, u64)> = engine.book.bids
                .top_n_levels::<5>()
                .iter()
                .map(|(p, q)| (p.0, q.0))
                .collect();
            
            let ask_levels: Vec<(u64, u64)> = engine.book.asks
                .top_n_levels::<5>()
                .iter()
                .map(|(p, q)| (p.0, q.0))
                .collect();
            
            update_book_depth(&bid_levels, &ask_levels);
            last_depth_update = std::time::Instant::now();
        }
        
        // Check for snapshot trigger
        let current_orders = state.order_count.load(Ordering::Relaxed);
        if current_orders > 0 && current_orders % SNAPSHOT_INTERVAL == 0 {
            // TODO: Serialize engine state and request snapshot
            // let data = engine.book.snapshot_to_buffer();
            // snapshot_manager.request_snapshot(current_orders, timestamp, data);
        }
        
        // Sleep briefly to avoid busy spinning in demo mode
        thread::sleep(Duration::from_millis(10));
    }
    
    println!("\n🛑 Shutting down Titan Node...");
    snapshot_manager.shutdown();
    println!("✅ Shutdown complete");
}

/// Setup Ctrl+C signal handler
fn ctrlc_handler(state: Arc<EngineState>) {
    #[cfg(unix)]
    {
        use std::sync::mpsc::channel;
        let (tx, rx) = channel();
        
        thread::spawn(move || {
            let _ = rx.recv();
            state.shutdown.store(true, Ordering::Relaxed);
        });
        
        unsafe {
            libc::signal(libc::SIGINT, tx as *const _ as usize);
        }
    }
    
    #[cfg(windows)]
    {
        // On Windows, use a simple polling approach
        thread::spawn(move || {
            // Wait for any signal that would cause shutdown
            loop {
                thread::sleep(Duration::from_millis(100));
                // Check if parent process signaled shutdown
                if state.shutdown.load(Ordering::Relaxed) {
                    break;
                }
            }
        });
    }
}
